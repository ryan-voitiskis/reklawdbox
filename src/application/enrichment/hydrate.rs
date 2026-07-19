//! Shared enrichment hydration workflow.

use crate::adapters::{providers, state};
use crate::application::analysis::{
    batch::persist_analysis_cache_write,
    identity::audio_cache_identities_with_current_stratum_input, job as analysis_job,
    model::AnalysisCacheWrite,
};
use crate::application::batch::task_join_error_summary;
use crate::application::cache_writer::{
    self, CacheMessageError, CacheWriteRequest, CacheWriterReport, send_cache_message,
};
use crate::domain::library::Track;

use super::lookup::{self, LookupIdentity};
use super::model::{EnrichmentProvider, HydrationStage, HydrationStages};

pub(crate) fn provider_stages(providers: &[EnrichmentProvider]) -> Vec<HydrationStage> {
    providers
        .iter()
        .copied()
        .map(HydrationStage::Lookup)
        .collect()
}

/// Raw track identity plus the normalized cache keys used by all providers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HydrationTrackIdentity {
    pub(crate) track_id: String,
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) album: String,
    pub(crate) norm_artist: String,
    pub(crate) norm_title: String,
    pub(crate) norm_album: Option<String>,
}

impl HydrationTrackIdentity {
    pub(crate) fn new(track_id: String, artist: String, title: String, album: String) -> Self {
        let norm_artist = crate::domain::metadata::normalize_for_matching(&artist);
        let norm_title = crate::domain::metadata::normalize_for_matching(&title);
        let norm_album = crate::domain::metadata::normalize_for_matching(&album);
        let norm_album = (!norm_album.is_empty()).then_some(norm_album);
        Self {
            track_id,
            artist,
            title,
            album,
            norm_artist,
            norm_title,
            norm_album,
        }
    }

    fn cache_album(&self, provider: EnrichmentProvider) -> Option<&str> {
        (provider == EnrichmentProvider::Discogs)
            .then_some(self.norm_album.as_deref())
            .flatten()
    }

    fn lookup_identity(&self) -> LookupIdentity {
        LookupIdentity::new(
            self.artist.clone(),
            self.title.clone(),
            (!self.album.is_empty()).then(|| self.album.clone()),
        )
    }
}

/// Cache-selection policy for one CLI hydration invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HydrationSelectionPolicy {
    pub(crate) stages: HydrationStages,
    pub(crate) retry_cached_errors: bool,
    pub(crate) essentia_available: bool,
}

/// One selected audio-analysis job with the exact missing analyzer layers.
#[derive(Clone, Debug)]
pub(crate) struct HydrationAnalysisJob {
    pub(crate) track: Track,
    pub(crate) needs_stratum: bool,
    pub(crate) needs_essentia: bool,
}

/// Canonical cache interpretation for a CLI hydration invocation.
#[derive(Clone, Debug, Default)]
pub(crate) struct HydrationPlan {
    pub(crate) total_matched_tracks: usize,
    pub(crate) discogs_tracks: Vec<HydrationTrackIdentity>,
    pub(crate) analysis_jobs: Vec<HydrationAnalysisJob>,
    pub(crate) discogs_cached: u32,
    pub(crate) discogs_errors: u32,
    pub(crate) analysis_cached: u32,
}

impl HydrationPlan {
    pub(crate) fn total_work(&self) -> usize {
        self.discogs_tracks.len() + self.analysis_jobs.len()
    }
}

/// Select pending CLI hydration work with one application-owned cache policy.
pub(crate) fn select_hydration_work(
    store_conn: &rusqlite::Connection,
    tracks: &[Track],
    policy: &HydrationSelectionPolicy,
) -> Result<HydrationPlan, rusqlite::Error> {
    let want_discogs = policy
        .stages
        .contains(HydrationStage::Lookup(EnrichmentProvider::Discogs));
    let want_analysis = policy.stages.contains(HydrationStage::Analysis);
    let mut plan = HydrationPlan {
        total_matched_tracks: tracks.len(),
        ..HydrationPlan::default()
    };

    if want_discogs {
        let identities: Vec<_> = tracks
            .iter()
            .map(|track| {
                HydrationTrackIdentity::new(
                    track.id.clone(),
                    track.artist.clone(),
                    track.title.clone(),
                    track.album.clone(),
                )
            })
            .collect();
        let owned_keys: Vec<_> = identities
            .iter()
            .map(|identity| {
                (
                    EnrichmentProvider::Discogs.as_str().to_string(),
                    identity.norm_artist.clone(),
                    identity.norm_title.clone(),
                    identity.norm_album.clone().unwrap_or_default(),
                )
            })
            .collect();
        let key_refs: Vec<_> = owned_keys
            .iter()
            .map(|(provider, artist, title, album)| {
                (
                    provider.as_str(),
                    artist.as_str(),
                    title.as_str(),
                    album.as_str(),
                )
            })
            .collect();
        let cached_entries = state::batch_get_enrichment_including_errors(store_conn, &key_refs)?;

        for identity in identities {
            let key = (
                EnrichmentProvider::Discogs.as_str().to_string(),
                identity.norm_artist.clone(),
                identity.norm_title.clone(),
                identity.norm_album.clone().unwrap_or_default(),
            );
            if let Some(entry) = cached_entries.get(&key) {
                if entry.match_quality.as_deref() == Some("error") {
                    plan.discogs_errors += 1;
                    if !policy.retry_cached_errors {
                        continue;
                    }
                } else {
                    plan.discogs_cached += 1;
                    continue;
                }
            }
            plan.discogs_tracks.push(identity);
        }
    }

    if want_analysis {
        let identities = audio_cache_identities_with_current_stratum_input(
            tracks.iter().map(|track| track.file_path.as_str()),
        );
        let stratum_identities: Vec<_> = identities
            .iter()
            .flatten()
            .filter_map(|identity| identity.as_stratum_store_identity())
            .collect();
        let fresh_stratum = state::batch_fresh_audio_analysis_existence(
            store_conn,
            &stratum_identities,
            crate::adapters::audio::ANALYZER_STRATUM,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        )?;
        let fresh_essentia = if policy.essentia_available {
            let essentia_identities: Vec<_> = identities
                .iter()
                .flatten()
                .map(|identity| identity.as_essentia_store_identity())
                .collect();
            state::batch_fresh_audio_analysis_existence(
                store_conn,
                &essentia_identities,
                crate::adapters::audio::ANALYZER_ESSENTIA,
                crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
            )?
        } else {
            std::collections::HashSet::new()
        };

        for (track, identity) in tracks.iter().cloned().zip(identities) {
            let has_stratum = identity
                .as_ref()
                .is_some_and(|identity| fresh_stratum.contains(&identity.cache_key));
            let has_essentia = !policy.essentia_available
                || identity
                    .as_ref()
                    .is_some_and(|identity| fresh_essentia.contains(&identity.cache_key));
            if has_stratum && has_essentia {
                plan.analysis_cached += 1;
            } else {
                plan.analysis_jobs.push(HydrationAnalysisJob {
                    track,
                    needs_stratum: !has_stratum,
                    needs_essentia: !has_essentia,
                });
            }
        }
        // Longest-processing-time scheduling: keep the exact current ordering.
        plan.analysis_jobs
            .sort_by_key(|job| std::cmp::Reverse(job.track.length));
    }

    Ok(plan)
}

/// Resolve which tracks already have a terminal cache entry for every provider.
pub(crate) fn enrichment_completion_flags(
    store_conn: &rusqlite::Connection,
    tracks: &[HydrationTrackIdentity],
    providers: &[EnrichmentProvider],
) -> Result<Vec<bool>, rusqlite::Error> {
    let owned_keys: Vec<_> = tracks
        .iter()
        .flat_map(|track| {
            providers.iter().map(move |provider| {
                (
                    provider.as_str().to_string(),
                    track.norm_artist.clone(),
                    track.norm_title.clone(),
                    track.cache_album(*provider).unwrap_or_default().to_string(),
                )
            })
        })
        .collect();
    let key_refs: Vec<_> = owned_keys
        .iter()
        .map(|(provider, artist, title, album)| {
            (
                provider.as_str(),
                artist.as_str(),
                title.as_str(),
                album.as_str(),
            )
        })
        .collect();
    let cached = state::batch_get_enrichment(store_conn, &key_refs)?;

    Ok(tracks
        .iter()
        .map(|track| {
            providers.iter().all(|provider| {
                cached.contains_key(&(
                    provider.as_str().to_string(),
                    track.norm_artist.clone(),
                    track.norm_title.clone(),
                    track.cache_album(*provider).unwrap_or_default().to_string(),
                ))
            })
        })
        .collect())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EnrichmentCachePolicy {
    pub(crate) skip_cached: bool,
    pub(crate) force_refresh: bool,
}

/// Whether a failed Discogs lookup becomes a terminal retryable cache row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LookupFailurePersistence {
    DoNotCache,
    CacheTerminalError,
}

/// Batch-wide Discogs authentication failure coordination used by MCP enrichment.
#[derive(Clone, Debug)]
pub(crate) struct DiscogsBatchAuthState {
    failed: tokio::sync::watch::Receiver<bool>,
    fail: tokio::sync::watch::Sender<bool>,
}

impl DiscogsBatchAuthState {
    pub(crate) fn new(
        failed: tokio::sync::watch::Receiver<bool>,
        fail: tokio::sync::watch::Sender<bool>,
    ) -> Self {
        Self { failed, fail }
    }

    fn failed(&self) -> bool {
        *self.failed.borrow()
    }

    fn record_failure(&self) {
        let _ = self.fail.send(true);
    }
}

#[derive(Clone, Debug)]
pub(crate) enum HydrationFailureKind {
    AuthBatchFailed,
    DiscogsAuth(providers::discogs::AuthRemediation),
    Lookup(String),
    Serialize(String),
    SemaphoreClosed,
    CacheWrite(CacheMessageError),
}

impl HydrationFailureKind {
    pub(crate) fn stage(&self) -> &'static str {
        match self {
            Self::AuthBatchFailed | Self::DiscogsAuth(_) => "auth",
            Self::Lookup(_) => "lookup",
            Self::Serialize(_) => "serialize",
            Self::SemaphoreClosed => "semaphore",
            Self::CacheWrite(_) => "cache_write",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HydrationFailure {
    pub(crate) identity: HydrationTrackIdentity,
    pub(crate) provider: EnrichmentProvider,
    pub(crate) kind: HydrationFailureKind,
}

#[derive(Debug, Default)]
pub(crate) struct EnrichmentTrackOutcome {
    pub(crate) enriched: usize,
    pub(crate) cached: usize,
    pub(crate) no_match: usize,
    pub(crate) operation_failures: usize,
    pub(crate) cache_write_failures: usize,
    pub(crate) failures: Vec<HydrationFailure>,
}

impl EnrichmentTrackOutcome {
    fn absorb(&mut self, other: Self) {
        self.enriched += other.enriched;
        self.cached += other.cached;
        self.no_match += other.no_match;
        self.operation_failures += other.operation_failures;
        self.cache_write_failures += other.cache_write_failures;
        self.failures.extend(other.failures);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct ProviderStagePlan {
    need_discogs: bool,
    need_bandcamp: bool,
    cached: usize,
}

fn provider_stage_plan(
    store_path: &str,
    identity: &HydrationTrackIdentity,
    providers: &[EnrichmentProvider],
    policy: EnrichmentCachePolicy,
) -> ProviderStagePlan {
    let stages = provider_stages(providers);
    let wants = |provider| stages.contains(&HydrationStage::Lookup(provider));
    let want_discogs = wants(EnrichmentProvider::Discogs);
    let want_bandcamp = wants(EnrichmentProvider::Bandcamp);
    if !policy.skip_cached || policy.force_refresh {
        return ProviderStagePlan {
            need_discogs: want_discogs,
            need_bandcamp: want_bandcamp,
            cached: 0,
        };
    }

    let connection = match state::open_read_only(store_path) {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!("track enrichment failed to open read-only store: {error}");
            return ProviderStagePlan {
                need_discogs: want_discogs,
                need_bandcamp: want_bandcamp,
                cached: 0,
            };
        }
    };
    let is_cached = |provider: EnrichmentProvider| {
        state::get_enrichment(
            &connection,
            provider.as_str(),
            &identity.norm_artist,
            &identity.norm_title,
            identity.cache_album(provider),
            false,
        )
        .ok()
        .flatten()
        .is_some()
    };
    let discogs_cached = want_discogs && is_cached(EnrichmentProvider::Discogs);
    let bandcamp_cached = want_bandcamp && is_cached(EnrichmentProvider::Bandcamp);
    ProviderStagePlan {
        need_discogs: want_discogs && !discogs_cached,
        need_bandcamp: want_bandcamp && !bandcamp_cached,
        cached: usize::from(discogs_cached) + usize::from(bandcamp_cached),
    }
}

#[derive(Debug)]
pub(crate) struct HydrationWorkerCompletion<T> {
    pub(crate) terminal: bool,
    pub(crate) value: T,
}

impl<T> HydrationWorkerCompletion<T> {
    pub(crate) fn completed(value: T) -> Self {
        Self {
            terminal: true,
            value,
        }
    }

    pub(crate) fn cancelled(value: T) -> Self {
        Self {
            terminal: false,
            value,
        }
    }
}

#[derive(Debug)]
pub(crate) struct HydrationJoinFailure<I> {
    pub(crate) identity: I,
    pub(crate) summary: String,
    pub(crate) error: String,
}

#[derive(Debug)]
pub(crate) struct HydrationWorkerReport<I, T> {
    pub(crate) selected: usize,
    pub(crate) scheduled: usize,
    pub(crate) terminal_workers: usize,
    pub(crate) completed: Vec<(I, T)>,
    pub(crate) join_failures: Vec<HydrationJoinFailure<I>>,
}

impl<I, T> HydrationWorkerReport<I, T> {
    pub(crate) fn incomplete(&self) -> usize {
        self.selected.saturating_sub(self.terminal_workers)
    }
}

struct HydrationWorkerTasks<I, T> {
    handles: Vec<Option<(I, tokio::task::JoinHandle<HydrationWorkerCompletion<T>>)>>,
}

impl<I, T> Drop for HydrationWorkerTasks<I, T> {
    fn drop(&mut self) {
        for (_, handle) in self.handles.iter().flatten() {
            handle.abort();
        }
    }
}

/// Run a bounded set of hydration workers with stable identity and join accounting.
pub(crate) async fn run_bounded_workers<Item, Identity, Output, IdentityFn, Worker, WorkerFuture>(
    task_label: &'static str,
    items: Vec<Item>,
    concurrency: usize,
    cancel: tokio_util::sync::CancellationToken,
    identity: IdentityFn,
    worker: Worker,
) -> HydrationWorkerReport<Identity, Output>
where
    Item: Send + 'static,
    Identity: Clone + Send + 'static,
    IdentityFn: Fn(&Item) -> Identity,
    Worker: Fn(Item) -> WorkerFuture + Clone + Send + 'static,
    WorkerFuture: std::future::Future<Output = HydrationWorkerCompletion<Output>> + Send + 'static,
    Output: Send + 'static,
{
    let selected = items.len();
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut tasks = HydrationWorkerTasks {
        handles: Vec::with_capacity(selected),
    };

    for item in items {
        if cancel.is_cancelled() {
            break;
        }
        let permit = tokio::select! {
            result = semaphore.clone().acquire_owned() => match result {
                Ok(permit) => permit,
                Err(_) => break,
            },
            _ = cancel.cancelled() => break,
        };
        let item_identity = identity(&item);
        let worker = worker.clone();
        tasks.handles.push(Some((
            item_identity,
            tokio::spawn(async move {
                let result = worker(item).await;
                drop(permit);
                result
            }),
        )));
    }

    let scheduled = tasks.handles.len();
    let mut terminal_workers = 0usize;
    let mut completed = Vec::with_capacity(scheduled);
    let mut join_failures = Vec::new();
    for index in 0..scheduled {
        let result = {
            let (_, handle) = tasks.handles[index]
                .as_mut()
                .expect("scheduled hydration worker handle should exist");
            handle.await
        };
        let (identity, _) = tasks.handles[index]
            .take()
            .expect("joined hydration worker handle should exist");
        match result {
            Ok(completion) => {
                if completion.terminal {
                    terminal_workers += 1;
                }
                completed.push((identity, completion.value));
            }
            Err(error) => {
                let outcome = if error.is_cancelled() {
                    "was cancelled"
                } else if error.is_panic() {
                    "panicked"
                } else {
                    "failed"
                };
                join_failures.push(HydrationJoinFailure {
                    identity,
                    summary: format!("{task_label} {outcome}"),
                    error: error.to_string(),
                });
            }
        }
    }

    HydrationWorkerReport {
        selected,
        scheduled,
        terminal_workers,
        completed,
        join_failures,
    }
}

#[derive(Debug, Default)]
pub(crate) struct HydrationAnalysisOutcome {
    pub(crate) operation_failed: bool,
    pub(crate) cache_write_failed: bool,
}

impl HydrationAnalysisOutcome {
    pub(crate) fn succeeded(&self) -> bool {
        !self.operation_failed && !self.cache_write_failed
    }
}

/// Canonical final accounting for one selected hydration stage.
#[derive(Debug, Default)]
pub(crate) struct HydrationStageReport {
    pub(crate) selected: usize,
    pub(crate) terminal_workers: usize,
    pub(crate) enriched: u32,
    pub(crate) no_match: u32,
    pub(crate) failed: u32,
    pub(crate) operation_failures: u32,
    pub(crate) cache_write_failures: u32,
    pub(crate) worker_join_failures: u32,
    pub(crate) error_summaries: Vec<String>,
}

impl HydrationStageReport {
    pub(crate) fn incomplete(&self) -> usize {
        self.selected.saturating_sub(self.terminal_workers)
    }

    pub(crate) fn failed_task(selected: usize, terminal_workers: usize, summary: String) -> Self {
        Self {
            selected,
            terminal_workers,
            worker_join_failures: 1,
            error_summaries: vec![summary],
            ..Self::default()
        }
    }
}

pub(crate) fn discogs_stage_report(
    worker_report: HydrationWorkerReport<HydrationTrackIdentity, EnrichmentTrackOutcome>,
) -> HydrationStageReport {
    debug_assert!(worker_report.scheduled <= worker_report.selected);
    debug_assert_eq!(
        worker_report.incomplete(),
        worker_report
            .selected
            .saturating_sub(worker_report.terminal_workers)
    );
    let mut report = HydrationStageReport {
        selected: worker_report.selected,
        terminal_workers: worker_report.terminal_workers,
        ..HydrationStageReport::default()
    };
    for (_, outcome) in worker_report.completed {
        report.enriched += outcome.enriched as u32;
        report.no_match += outcome.no_match as u32;
        report.failed += u32::from(!outcome.failures.is_empty());
        report.operation_failures += outcome.operation_failures as u32;
        report.cache_write_failures += outcome.cache_write_failures as u32;
    }
    for failure in worker_report.join_failures {
        tracing::error!("{}: {}", failure.summary, failure.error);
        report.worker_join_failures += 1;
        report.error_summaries.push(failure.summary);
    }
    report
}

pub(crate) fn analysis_stage_report<I>(
    worker_report: HydrationWorkerReport<I, HydrationAnalysisOutcome>,
) -> HydrationStageReport {
    debug_assert!(worker_report.scheduled <= worker_report.selected);
    debug_assert_eq!(
        worker_report.incomplete(),
        worker_report
            .selected
            .saturating_sub(worker_report.terminal_workers)
    );
    let mut report = HydrationStageReport {
        selected: worker_report.selected,
        terminal_workers: worker_report.terminal_workers,
        ..HydrationStageReport::default()
    };
    for (_, outcome) in worker_report.completed {
        report.enriched += u32::from(outcome.succeeded());
        report.failed += u32::from(!outcome.succeeded());
        report.operation_failures += u32::from(outcome.operation_failed);
        report.cache_write_failures += u32::from(outcome.cache_write_failed);
    }
    for failure in worker_report.join_failures {
        tracing::error!("{}: {}", failure.summary, failure.error);
        report.worker_join_failures += 1;
        report.error_summaries.push(failure.summary);
    }
    report
}

/// Dispatch the `Analysis` hydration stage through the shared application job.
pub(crate) async fn run_analysis_stage<T>(
    raw_file_path: &str,
    needs_stratum: bool,
    needs_essentia: bool,
    essentia_python: Option<&str>,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
) -> HydrationAnalysisOutcome
where
    T: From<AnalysisCacheWrite>,
{
    let mut outcome = HydrationAnalysisOutcome::default();
    let report = match analysis_job::run(
        raw_file_path,
        needs_stratum,
        needs_essentia,
        essentia_python,
        true,
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            tracing::error!("Analysis job failed for {raw_file_path}: {error}");
            outcome.operation_failed = true;
            return outcome;
        }
    };

    if let Some(Err(error)) = report.stratum {
        tracing::error!("stratum-dsp analysis failed for {raw_file_path}: {error}");
        outcome.operation_failed = true;
    }
    if let Some(Err(error)) = report.essentia {
        tracing::error!("Essentia analysis failed for {raw_file_path}: {error}");
        outcome.operation_failed = true;
    }
    for message in report.cache_messages {
        let analyzer = message.analyzer.clone();
        if let Err(error) =
            send_cache_message(cache_tx, T::from(message), &format!("{analyzer} analysis")).await
        {
            tracing::error!("{error}");
            outcome.cache_write_failed = true;
        }
    }

    outcome
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnrichmentCacheWrite {
    pub(crate) provider: EnrichmentProvider,
    pub(crate) norm_artist: String,
    pub(crate) norm_title: String,
    pub(crate) norm_album: Option<String>,
    pub(crate) match_quality: Option<String>,
    pub(crate) response_json: Option<String>,
}

/// Mixed cache payloads produced by the CLI hydration command.
#[derive(Debug)]
pub(crate) enum HydrateCacheMessage {
    Enrichment(EnrichmentCacheWrite),
    AudioAnalysis(AnalysisCacheWrite),
}

impl From<EnrichmentCacheWrite> for HydrateCacheMessage {
    fn from(write: EnrichmentCacheWrite) -> Self {
        Self::Enrichment(write)
    }
}

impl From<AnalysisCacheWrite> for HydrateCacheMessage {
    fn from(write: AnalysisCacheWrite) -> Self {
        Self::AudioAnalysis(write)
    }
}

#[derive(Debug)]
pub(crate) struct HydrateCacheWriterStartError {
    pub(crate) summary: String,
}

impl std::fmt::Display for HydrateCacheWriterStartError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.summary)
    }
}

impl std::error::Error for HydrateCacheWriterStartError {}

#[derive(Debug, Default)]
pub(crate) struct HydrateCacheWriterCompletion {
    pub(crate) report: CacheWriterReport,
    pub(crate) join_failures: u32,
    pub(crate) error_summaries: Vec<String>,
}

/// Application-owned terminal reports for one complete CLI hydration run.
#[derive(Debug, Default)]
pub(crate) struct HydrationApplicationReport {
    pub(crate) discogs: HydrationStageReport,
    pub(crate) analysis: HydrationStageReport,
    pub(crate) writer: CacheWriterReport,
    pub(crate) writer_join_failures: u32,
    pub(crate) error_summaries: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct HydrationFinalAccounting {
    pub(crate) operation_failures: u32,
    pub(crate) application_join_failures: u32,
    pub(crate) writer_failures: u32,
    pub(crate) incomplete: usize,
}

impl HydrationApplicationReport {
    pub(crate) fn assemble(
        discogs: HydrationStageReport,
        analysis: HydrationStageReport,
        writer: HydrateCacheWriterCompletion,
    ) -> Self {
        let mut error_summaries = Vec::new();
        error_summaries.extend(discogs.error_summaries.iter().cloned());
        error_summaries.extend(analysis.error_summaries.iter().cloned());
        error_summaries.extend(writer.error_summaries.iter().cloned());
        Self {
            discogs,
            analysis,
            writer: writer.report,
            writer_join_failures: writer.join_failures,
            error_summaries,
        }
    }

    pub(crate) fn final_accounting(&self) -> HydrationFinalAccounting {
        HydrationFinalAccounting {
            operation_failures: self.discogs.operation_failures + self.analysis.operation_failures,
            application_join_failures: self.discogs.worker_join_failures
                + self.analysis.worker_join_failures
                + self.writer_join_failures,
            writer_failures: self.writer.failed,
            incomplete: self.discogs.incomplete() + self.analysis.incomplete(),
        }
    }
}

/// Owns the mixed hydration writer from channel creation through final join.
pub(crate) struct HydrateCacheWriterSession {
    sender: Option<tokio::sync::mpsc::Sender<CacheWriteRequest<HydrateCacheMessage>>>,
    writer: Option<tokio::task::JoinHandle<CacheWriterReport>>,
    cancel: tokio_util::sync::CancellationToken,
    cancel_on_drop: bool,
}

impl HydrateCacheWriterSession {
    pub(crate) fn start(
        store_path: String,
        capacity: usize,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<Self, HydrateCacheWriterStartError> {
        Self::start_with_completion(store_path, capacity, cancel, None)
    }

    fn start_with_completion(
        store_path: String,
        capacity: usize,
        cancel: tokio_util::sync::CancellationToken,
        completion: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<Self, HydrateCacheWriterStartError> {
        let (sender, receiver) = tokio::sync::mpsc::channel(capacity.max(1));
        let writer_cancel = cancel.clone();
        let spawned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            tokio::task::spawn_blocking(move || {
                let report = run_hydrate_cache_writer(store_path, receiver, writer_cancel);
                if let Some(completion) = completion {
                    let _ = completion.send(());
                }
                report
            })
        }));
        let writer = spawned.map_err(|panic| HydrateCacheWriterStartError {
            summary: format!(
                "hydrate cache writer task spawn failed: {}",
                panic_summary(panic)
            ),
        })?;
        Ok(Self {
            sender: Some(sender),
            writer: Some(writer),
            cancel,
            cancel_on_drop: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn start_observed(
        store_path: String,
        capacity: usize,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<(Self, tokio::sync::oneshot::Receiver<()>), HydrateCacheWriterStartError> {
        let (completion, completed) = tokio::sync::oneshot::channel();
        Self::start_with_completion(store_path, capacity, cancel, Some(completion))
            .map(|session| (session, completed))
    }

    pub(crate) fn sender(
        &self,
    ) -> &tokio::sync::mpsc::Sender<CacheWriteRequest<HydrateCacheMessage>> {
        self.sender
            .as_ref()
            .expect("hydrate cache writer sender is unavailable after finish")
    }

    pub(crate) async fn finish(mut self) -> HydrateCacheWriterCompletion {
        self.sender.take();
        let writer = self
            .writer
            .take()
            .expect("hydrate cache writer handle is unavailable after finish");
        let completion = match writer.await {
            Ok(report) => HydrateCacheWriterCompletion {
                error_summaries: report.error_summaries.clone(),
                report,
                ..HydrateCacheWriterCompletion::default()
            },
            Err(error) => {
                let summary = task_join_error_summary("hydrate cache writer task", &error);
                tracing::error!("{summary}: {error}");
                HydrateCacheWriterCompletion {
                    join_failures: 1,
                    error_summaries: vec![summary],
                    ..HydrateCacheWriterCompletion::default()
                }
            }
        };
        self.cancel_on_drop = false;
        completion
    }
}

impl Drop for HydrateCacheWriterSession {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.cancel.cancel();
        }
        self.sender.take();
        // A running blocking task cannot be aborted. Detaching after our
        // sender closes keeps the receiver and SQLite alive until any
        // analysis-reaper-held sender and acknowledgement have drained.
        self.writer.take();
    }
}

fn panic_summary(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

pub(crate) fn run_hydrate_cache_writer(
    store_path: String,
    receiver: tokio::sync::mpsc::Receiver<CacheWriteRequest<HydrateCacheMessage>>,
    cancel: tokio_util::sync::CancellationToken,
) -> CacheWriterReport {
    cache_writer::run(
        store_path,
        receiver,
        cancel,
        |connection, message| match message {
            HydrateCacheMessage::Enrichment(write) => {
                persist_enrichment_cache_write(connection, write)
            }
            HydrateCacheMessage::AudioAnalysis(analysis) => {
                persist_analysis_cache_write(connection, analysis)
            }
        },
        |message| match message {
            HydrateCacheMessage::Enrichment(write) => format!("{} enrichment", write.provider),
            HydrateCacheMessage::AudioAnalysis(analysis) => {
                format!("{} analysis", analysis.analyzer)
            }
        },
    )
}

#[cfg(test)]
pub(crate) async fn acknowledge_enrichment_cache_write<T>(
    tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
    write: EnrichmentCacheWrite,
    context: &str,
) -> Result<(), String>
where
    T: From<EnrichmentCacheWrite>,
{
    send_cache_message(tx, T::from(write), context)
        .await
        .map_err(|error| error.to_string())
}

/// Queue an MCP enrichment write while preserving its original public errors.
#[cfg(test)]
pub(crate) async fn acknowledge_mcp_enrichment_cache_write(
    tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    write: EnrichmentCacheWrite,
) -> Result<(), String> {
    let (acknowledgement, result) = tokio::sync::oneshot::channel();
    tx.send(CacheWriteRequest {
        payload: write,
        acknowledgement,
    })
    .await
    .map_err(|_| "cache write queue closed".to_string())?;
    result
        .await
        .map_err(|_| "cache writer acknowledgement canceled".to_string())?
}

fn failed_stage(
    identity: &HydrationTrackIdentity,
    provider: EnrichmentProvider,
    kind: HydrationFailureKind,
) -> EnrichmentTrackOutcome {
    let operation_failures = usize::from(matches!(
        &kind,
        HydrationFailureKind::AuthBatchFailed
            | HydrationFailureKind::DiscogsAuth(_)
            | HydrationFailureKind::Lookup(_)
            | HydrationFailureKind::Serialize(_)
            | HydrationFailureKind::SemaphoreClosed
    ));
    let cache_write_failures = usize::from(matches!(&kind, HydrationFailureKind::CacheWrite(_)));
    EnrichmentTrackOutcome {
        operation_failures,
        cache_write_failures,
        failures: vec![HydrationFailure {
            identity: identity.clone(),
            provider,
            kind,
        }],
        ..EnrichmentTrackOutcome::default()
    }
}

async fn persist_provider_outcome<T, Message>(
    identity: &HydrationTrackIdentity,
    provider: EnrichmentProvider,
    result: Option<T>,
    quality: impl FnOnce(&T) -> &'static str,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<Message>>,
) -> EnrichmentTrackOutcome
where
    T: serde::Serialize,
    Message: From<EnrichmentCacheWrite>,
{
    let (match_quality, response_json, matched) = match result {
        Some(result) => {
            let match_quality = quality(&result).to_string();
            let response_json = match serde_json::to_string(&result) {
                Ok(response_json) => response_json,
                Err(error) => {
                    return failed_stage(
                        identity,
                        provider,
                        HydrationFailureKind::Serialize(error.to_string()),
                    );
                }
            };
            (match_quality, Some(response_json), true)
        }
        None => ("none".to_string(), None, false),
    };
    let write = EnrichmentCacheWrite {
        provider,
        norm_artist: identity.norm_artist.clone(),
        norm_title: identity.norm_title.clone(),
        norm_album: identity.cache_album(provider).map(str::to_string),
        match_quality: Some(match_quality),
        response_json,
    };
    if let Err(error) = send_cache_message(
        cache_tx,
        Message::from(write),
        &format!("{provider} enrichment"),
    )
    .await
    {
        return failed_stage(identity, provider, HydrationFailureKind::CacheWrite(error));
    }

    EnrichmentTrackOutcome {
        enriched: usize::from(matched),
        no_match: usize::from(!matched),
        ..EnrichmentTrackOutcome::default()
    }
}

/// Resolve and durably account for one Discogs hydration track.
pub(crate) async fn hydrate_discogs_track<Message, LookupFuture>(
    identity: &HydrationTrackIdentity,
    failure_persistence: LookupFailurePersistence,
    auth: Option<&DiscogsBatchAuthState>,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<Message>>,
    lookup: LookupFuture,
) -> EnrichmentTrackOutcome
where
    Message: From<EnrichmentCacheWrite>,
    LookupFuture: std::future::Future<
            Output = Result<
                Option<providers::discogs::DiscogsResult>,
                providers::discogs::LookupError,
            >,
        >,
{
    if auth.is_some_and(DiscogsBatchAuthState::failed) {
        return failed_stage(
            identity,
            EnrichmentProvider::Discogs,
            HydrationFailureKind::AuthBatchFailed,
        );
    }

    match lookup.await {
        Ok(result) => {
            persist_provider_outcome(
                identity,
                EnrichmentProvider::Discogs,
                result,
                |result| {
                    if result.fuzzy_match { "fuzzy" } else { "exact" }
                },
                cache_tx,
            )
            .await
        }
        Err(error) => {
            let kind = if let Some(remediation) = error.auth_remediation() {
                if let Some(auth) = auth {
                    auth.record_failure();
                }
                HydrationFailureKind::DiscogsAuth(remediation.clone())
            } else {
                HydrationFailureKind::Lookup(error.to_string())
            };
            let mut outcome = failed_stage(identity, EnrichmentProvider::Discogs, kind);
            if failure_persistence == LookupFailurePersistence::CacheTerminalError {
                let write = EnrichmentCacheWrite {
                    provider: EnrichmentProvider::Discogs,
                    norm_artist: identity.norm_artist.clone(),
                    norm_title: identity.norm_title.clone(),
                    norm_album: identity
                        .cache_album(EnrichmentProvider::Discogs)
                        .map(str::to_string),
                    match_quality: Some("error".to_string()),
                    response_json: None,
                };
                if let Err(cache_error) =
                    send_cache_message(cache_tx, Message::from(write), "discogs error").await
                {
                    tracing::error!("{cache_error}");
                    outcome.cache_write_failures += 1;
                }
            }
            outcome
        }
    }
}

async fn run_bandcamp_stage(
    need: bool,
    identity: &HydrationTrackIdentity,
    http: &reqwest::Client,
    gate: std::sync::Arc<tokio::sync::Semaphore>,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
) -> EnrichmentTrackOutcome {
    if !need {
        return EnrichmentTrackOutcome::default();
    }
    let _permit = match gate.acquire().await {
        Ok(permit) => permit,
        Err(_) => {
            return failed_stage(
                identity,
                EnrichmentProvider::Bandcamp,
                HydrationFailureKind::SemaphoreClosed,
            );
        }
    };
    let lookup_identity = identity.lookup_identity();
    match lookup::dispatch_bandcamp(http, &lookup_identity, None).await {
        Ok(result) => {
            persist_provider_outcome(
                identity,
                EnrichmentProvider::Bandcamp,
                result,
                |result| {
                    if result.score == 100 {
                        "exact"
                    } else {
                        "fuzzy"
                    }
                },
                cache_tx,
            )
            .await
        }
        Err(error) => failed_stage(
            identity,
            EnrichmentProvider::Bandcamp,
            HydrationFailureKind::Lookup(error.to_string()),
        ),
    }
}

struct TrackEnrichmentWork {
    identity: HydrationTrackIdentity,
    plan: ProviderStagePlan,
}

async fn run_track_enrichment<DiscogsFuture>(
    work: TrackEnrichmentWork,
    http: &reqwest::Client,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    bandcamp_gate: std::sync::Arc<tokio::sync::Semaphore>,
    auth: &DiscogsBatchAuthState,
    discogs_lookup: DiscogsFuture,
) -> EnrichmentTrackOutcome
where
    DiscogsFuture: std::future::Future<
            Output = Result<
                Option<providers::discogs::DiscogsResult>,
                providers::discogs::LookupError,
            >,
        >,
{
    let discogs = async {
        if work.plan.need_discogs {
            hydrate_discogs_track(
                &work.identity,
                LookupFailurePersistence::DoNotCache,
                Some(auth),
                cache_tx,
                discogs_lookup,
            )
            .await
        } else {
            EnrichmentTrackOutcome::default()
        }
    };
    let bandcamp = run_bandcamp_stage(
        work.plan.need_bandcamp,
        &work.identity,
        http,
        bandcamp_gate,
        cache_tx,
    );
    let (discogs, bandcamp) = tokio::join!(discogs, bandcamp);

    let mut outcome = EnrichmentTrackOutcome {
        cached: work.plan.cached,
        ..EnrichmentTrackOutcome::default()
    };
    outcome.absorb(discogs);
    outcome.absorb(bandcamp);
    outcome
}

#[derive(Clone, Debug)]
pub(crate) struct EnrichmentWorkerConfig {
    pub(crate) providers: Vec<EnrichmentProvider>,
    pub(crate) policy: EnrichmentCachePolicy,
    pub(crate) store_path: String,
    pub(crate) concurrency: usize,
}

/// Run all selected provider stages for a bounded set of tracks.
pub(crate) async fn run_enrichment_workers<DiscogsLookup, DiscogsFuture>(
    tracks: Vec<HydrationTrackIdentity>,
    config: EnrichmentWorkerConfig,
    http: reqwest::Client,
    cache_tx: tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    auth: DiscogsBatchAuthState,
    discogs_lookup: DiscogsLookup,
) -> HydrationWorkerReport<HydrationTrackIdentity, EnrichmentTrackOutcome>
where
    DiscogsLookup: Fn(HydrationTrackIdentity) -> DiscogsFuture + Clone + Send + 'static,
    DiscogsFuture: std::future::Future<
            Output = Result<
                Option<providers::discogs::DiscogsResult>,
                providers::discogs::LookupError,
            >,
        > + Send
        + 'static,
{
    let concurrency = config.concurrency;
    let bandcamp_gate = std::sync::Arc::new(tokio::sync::Semaphore::new(2));
    run_bounded_workers(
        "enrich track worker task",
        tracks,
        concurrency,
        tokio_util::sync::CancellationToken::new(),
        Clone::clone,
        move |identity| {
            let providers = config.providers.clone();
            let store_path = config.store_path.clone();
            let http = http.clone();
            let cache_tx = cache_tx.clone();
            let bandcamp_gate = bandcamp_gate.clone();
            let auth = auth.clone();
            let discogs_lookup = discogs_lookup.clone();
            async move {
                let plan = provider_stage_plan(&store_path, &identity, &providers, config.policy);
                let discogs_future = discogs_lookup(identity.clone());
                let outcome = run_track_enrichment(
                    TrackEnrichmentWork { identity, plan },
                    &http,
                    &cache_tx,
                    bandcamp_gate,
                    &auth,
                    discogs_future,
                )
                .await;
                HydrationWorkerCompletion::completed(outcome)
            }
        },
    )
    .await
}

pub(crate) fn persist_enrichment_cache_write(
    conn: &rusqlite::Connection,
    write: &EnrichmentCacheWrite,
) -> Result<(), rusqlite::Error> {
    state::set_enrichment(
        conn,
        write.provider.as_str(),
        &write.norm_artist,
        &write.norm_title,
        write.norm_album.as_deref(),
        write.match_quality.as_deref(),
        write.response_json.as_deref(),
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EnrichmentCacheWriterReport {
    pub(crate) attempted: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) dropped_ack_receivers: usize,
}

pub(crate) fn run_enrichment_cache_writer(
    store_path: &str,
    mut cache_rx: tokio::sync::mpsc::Receiver<CacheWriteRequest<EnrichmentCacheWrite>>,
) -> EnrichmentCacheWriterReport {
    let connection =
        state::open(store_path).map_err(|error| format!("cache writer open failed: {error}"));
    if let Err(error) = &connection {
        tracing::error!("Enrich cache writer: {error}");
    }

    let mut report = EnrichmentCacheWriterReport::default();
    while let Some(request) = cache_rx.blocking_recv() {
        report.attempted += 1;
        let result = match &connection {
            Ok(conn) => persist_enrichment_cache_write(conn, &request.payload)
                .map_err(|error| format!("cache write failed: {error}")),
            Err(error) => Err(error.clone()),
        };

        if result.is_ok() {
            report.succeeded += 1;
        } else {
            report.failed += 1;
        }
        if request.acknowledgement.send(result).is_err() {
            report.dropped_ack_receivers += 1;
        }
    }

    debug_assert_eq!(report.attempted, report.succeeded + report.failed);
    report
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn enrichment_hydration_acknowledges_cache_writes() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let store_dir = tempfile::tempdir().expect("hydration store directory should create");
            let store_path = store_dir.path().join("internal.sqlite3");
            let store_path_string = store_path.to_string_lossy().to_string();
            let (sender, receiver) =
                tokio::sync::mpsc::channel::<CacheWriteRequest<EnrichmentCacheWrite>>(1);
            let writer_path = store_path_string.clone();
            let writer = tokio::task::spawn_blocking(move || {
                run_enrichment_cache_writer(&writer_path, receiver)
            });

            let result = acknowledge_enrichment_cache_write(
                &sender,
                EnrichmentCacheWrite {
                    provider: EnrichmentProvider::Bandcamp,
                    norm_artist: "shared artist".to_string(),
                    norm_title: "shared title".to_string(),
                    norm_album: None,
                    match_quality: Some("none".to_string()),
                    response_json: None,
                },
                "bandcamp hydration",
            )
            .await;
            assert_eq!(result, Ok(()));
            drop(sender);

            let report = writer.await.expect("hydration writer should join");
            assert_eq!(report.attempted, 1);
            assert_eq!(report.succeeded, 1);

            let conn = state::open(&store_path_string).expect("hydration store should reopen");
            let cached = state::get_enrichment(
                &conn,
                "bandcamp",
                "shared artist",
                "shared title",
                None,
                false,
            )
            .expect("hydration cache should read")
            .expect("acknowledged write should be durable");
            assert_eq!(cached.match_quality.as_deref(), Some("none"));
            assert!(cached.response_json.is_none());
        })
        .await
        .expect("hydration acknowledgement workflow should finish within five seconds");
    }

    fn synthetic_track(id: &str, file_path: String, length: i32) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Title {id}"),
            artist: format!("Artist {id}"),
            album: format!("Album {id}"),
            genre: String::new(),
            bpm: 0.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length,
            file_path,
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    fn discogs_result() -> providers::discogs::DiscogsResult {
        providers::discogs::DiscogsResult {
            title: "Synthetic Match".to_string(),
            year: "2026".to_string(),
            label: "Synthetic Records".to_string(),
            genres: vec!["Electronic".to_string()],
            styles: vec!["Techno".to_string()],
            url: "https://www.discogs.com/release/synthetic".to_string(),
            cover_image: String::new(),
            fuzzy_match: false,
        }
    }

    #[test]
    fn hydrate_selection_preserves_cached_error_analysis_and_lpt_policy() {
        let directory = tempfile::tempdir().expect("selection directory should create");
        let store_path = directory.path().join("internal.sqlite3");
        let connection = state::open(&store_path.to_string_lossy()).expect("store should open");
        let mut tracks = Vec::new();
        for (id, length) in [("cached", 10), ("error", 30), ("missing", 20)] {
            let path = directory.path().join(format!("{id}.flac"));
            std::fs::write(&path, b"synthetic audio identity")
                .expect("synthetic identity file should write");
            tracks.push(synthetic_track(
                id,
                path.to_string_lossy().to_string(),
                length,
            ));
        }

        let cached_identity = HydrationTrackIdentity::new(
            tracks[0].id.clone(),
            tracks[0].artist.clone(),
            tracks[0].title.clone(),
            tracks[0].album.clone(),
        );
        state::set_enrichment(
            &connection,
            "discogs",
            &cached_identity.norm_artist,
            &cached_identity.norm_title,
            cached_identity.norm_album.as_deref(),
            Some("exact"),
            Some("{}"),
        )
        .expect("terminal cache fixture should write");
        let error_identity = HydrationTrackIdentity::new(
            tracks[1].id.clone(),
            tracks[1].artist.clone(),
            tracks[1].title.clone(),
            tracks[1].album.clone(),
        );
        state::set_enrichment(
            &connection,
            "discogs",
            &error_identity.norm_artist,
            &error_identity.norm_title,
            error_identity.norm_album.as_deref(),
            Some("error"),
            None,
        )
        .expect("error cache fixture should write");

        let audio_identities = audio_cache_identities_with_current_stratum_input(
            tracks.iter().map(|track| track.file_path.as_str()),
        );
        let cached_audio = audio_identities[0]
            .as_ref()
            .expect("synthetic file should have an audio identity");
        state::set_audio_analysis_with_fingerprint(
            &connection,
            &cached_audio.cache_key,
            crate::adapters::audio::ANALYZER_STRATUM,
            cached_audio.file_size,
            cached_audio.file_mtime,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            cached_audio
                .stratum_input_fingerprint
                .as_deref()
                .expect("Stratum identity should include its input fingerprint"),
            "{}",
        )
        .expect("fresh Stratum fixture should write");

        let stages = HydrationStages::parse_csv("discogs,analysis").expect("stages should parse");
        let skipped_errors = select_hydration_work(
            &connection,
            &tracks,
            &HydrationSelectionPolicy {
                stages: stages.clone(),
                retry_cached_errors: false,
                essentia_available: false,
            },
        )
        .expect("selection should succeed");
        assert_eq!(skipped_errors.total_matched_tracks, 3);
        assert_eq!(skipped_errors.discogs_cached, 1);
        assert_eq!(skipped_errors.discogs_errors, 1);
        assert_eq!(
            skipped_errors
                .discogs_tracks
                .iter()
                .map(|identity| identity.track_id.as_str())
                .collect::<Vec<_>>(),
            ["missing"]
        );
        assert_eq!(skipped_errors.analysis_cached, 1);
        assert_eq!(
            skipped_errors
                .analysis_jobs
                .iter()
                .map(|job| job.track.id.as_str())
                .collect::<Vec<_>>(),
            ["error", "missing"],
            "analysis jobs must retain longest-processing-time ordering"
        );
        assert!(
            skipped_errors
                .analysis_jobs
                .iter()
                .all(|job| job.needs_stratum && !job.needs_essentia)
        );

        let retried_errors = select_hydration_work(
            &connection,
            &tracks,
            &HydrationSelectionPolicy {
                stages: stages.clone(),
                retry_cached_errors: true,
                essentia_available: false,
            },
        )
        .expect("retry selection should succeed");
        assert_eq!(
            retried_errors
                .discogs_tracks
                .iter()
                .map(|identity| identity.track_id.as_str())
                .collect::<Vec<_>>(),
            ["error", "missing"]
        );

        let essentia_required = select_hydration_work(
            &connection,
            &tracks,
            &HydrationSelectionPolicy {
                stages,
                retry_cached_errors: false,
                essentia_available: true,
            },
        )
        .expect("Essentia-aware selection should succeed");
        assert_eq!(essentia_required.analysis_cached, 0);
        let cached_job = essentia_required
            .analysis_jobs
            .iter()
            .find(|job| job.track.id == "cached")
            .expect("Stratum-only cached track should need Essentia");
        assert!(!cached_job.needs_stratum);
        assert!(cached_job.needs_essentia);
    }

    async fn run_discogs_policy_case(
        policy: LookupFailurePersistence,
        result: Result<Option<providers::discogs::DiscogsResult>, providers::discogs::LookupError>,
    ) -> (
        EnrichmentTrackOutcome,
        CacheWriterReport,
        Option<(String, Option<String>)>,
    ) {
        let directory = tempfile::tempdir().expect("policy store directory should create");
        let store_path = directory
            .path()
            .join("internal.sqlite3")
            .to_string_lossy()
            .to_string();
        let cancel = tokio_util::sync::CancellationToken::new();
        let session = HydrateCacheWriterSession::start(store_path.clone(), 4, cancel)
            .expect("writer session should start");
        let identity = HydrationTrackIdentity::new(
            "policy-track".to_string(),
            "Policy Artist".to_string(),
            "Policy Title".to_string(),
            "Policy Album".to_string(),
        );
        let outcome =
            hydrate_discogs_track(&identity, policy, None, session.sender(), async { result })
                .await;
        let completion = session.finish().await;
        let connection = state::open(&store_path).expect("policy store should reopen");
        let cached = state::get_enrichment(
            &connection,
            "discogs",
            &identity.norm_artist,
            &identity.norm_title,
            identity.norm_album.as_deref(),
            true,
        )
        .expect("policy cache should read")
        .map(|entry| (entry.match_quality.unwrap_or_default(), entry.response_json));
        (outcome, completion.report, cached)
    }

    #[tokio::test]
    async fn hydrate_discogs_policy_preserves_two_surface_cache_matrix() {
        tokio::time::timeout(Duration::from_secs(5), async {
            for policy in [
                LookupFailurePersistence::DoNotCache,
                LookupFailurePersistence::CacheTerminalError,
            ] {
                let (matched, report, cached) =
                    run_discogs_policy_case(policy, Ok(Some(discogs_result()))).await;
                assert_eq!(matched.enriched, 1);
                assert_eq!(matched.no_match, 0);
                assert!(matched.failures.is_empty());
                assert_eq!(report.succeeded, 1);
                let (quality, response) = cached.expect("match should be cached");
                assert_eq!(quality, "exact");
                assert!(response.is_some());

                let (no_match, report, cached) = run_discogs_policy_case(policy, Ok(None)).await;
                assert_eq!(no_match.enriched, 0);
                assert_eq!(no_match.no_match, 1);
                assert!(no_match.failures.is_empty());
                assert_eq!(report.succeeded, 1);
                assert_eq!(cached, Some(("none".to_string(), None)));
            }

            let (mcp_error, mcp_report, mcp_cached) = run_discogs_policy_case(
                LookupFailurePersistence::DoNotCache,
                Err(providers::discogs::LookupError::message(
                    "synthetic lookup failure",
                )),
            )
            .await;
            assert_eq!(mcp_error.operation_failures, 1);
            assert_eq!(mcp_report.attempted, 0);
            assert!(mcp_cached.is_none(), "MCP lookup errors must not be cached");

            let (cli_error, cli_report, cli_cached) = run_discogs_policy_case(
                LookupFailurePersistence::CacheTerminalError,
                Err(providers::discogs::LookupError::message(
                    "synthetic lookup failure",
                )),
            )
            .await;
            assert_eq!(cli_error.operation_failures, 1);
            assert_eq!(cli_report.succeeded, 1);
            assert_eq!(cli_cached, Some(("error".to_string(), None)));
        })
        .await
        .expect("Discogs policy matrix should finish within five seconds");
    }

    struct DropSignal(Option<tokio::sync::oneshot::Sender<()>>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            if let Some(signal) = self.0.take() {
                let _ = signal.send(());
            }
        }
    }

    #[tokio::test]
    async fn hydrate_worker_coordinator_abort_cleans_inner_worker() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let started = Arc::new(tokio::sync::Notify::new());
            let (dropped, worker_dropped) = tokio::sync::oneshot::channel();
            let dropped = Arc::new(Mutex::new(Some(dropped)));
            let coordinator = tokio::spawn({
                let started = started.clone();
                let dropped = dropped.clone();
                async move {
                    run_bounded_workers(
                        "cancellation worker",
                        vec!["track".to_string()],
                        1,
                        tokio_util::sync::CancellationToken::new(),
                        Clone::clone,
                        move |_| {
                            let started = started.clone();
                            let signal = dropped
                                .lock()
                                .expect("drop signal lock should remain available")
                                .take();
                            async move {
                                let _drop_signal = DropSignal(signal);
                                started.notify_one();
                                std::future::pending::<()>().await;
                                HydrationWorkerCompletion::completed(())
                            }
                        },
                    )
                    .await
                }
            });

            tokio::time::timeout(Duration::from_secs(1), started.notified())
                .await
                .expect("worker-start barrier should be bounded");
            coordinator.abort();
            let join = tokio::time::timeout(Duration::from_secs(1), coordinator)
                .await
                .expect("coordinator join should be bounded");
            assert!(
                join.expect_err("aborted coordinator should not succeed")
                    .is_cancelled()
            );
            tokio::time::timeout(Duration::from_secs(1), worker_dropped)
                .await
                .expect("inner worker cleanup should be bounded")
                .expect("inner worker should signal drop");
        })
        .await
        .expect("worker cleanup scenario should finish within five seconds");
    }

    #[tokio::test]
    async fn hydrate_cancellation_before_scheduling_reports_every_item_incomplete() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let cancel = tokio_util::sync::CancellationToken::new();
            cancel.cancel();
            let report = run_bounded_workers(
                "pre-cancelled worker",
                vec!["first", "second"],
                1,
                cancel,
                |identity| *identity,
                |_| async { HydrationWorkerCompletion::completed(()) },
            )
            .await;
            assert_eq!(report.selected, 2);
            assert_eq!(report.scheduled, 0);
            assert_eq!(report.terminal_workers, 0);
            assert_eq!(report.incomplete(), 2);
        })
        .await
        .expect("pre-cancelled scheduling should finish within five seconds");
    }

    fn hydrate_test_message(title: &str) -> HydrateCacheMessage {
        HydrateCacheMessage::Enrichment(EnrichmentCacheWrite {
            provider: EnrichmentProvider::Discogs,
            norm_artist: "writer artist".to_string(),
            norm_title: title.to_string(),
            norm_album: None,
            match_quality: Some("none".to_string()),
            response_json: None,
        })
    }

    fn hydrate_test_analysis_message(analyzer: &str, id: &str) -> HydrateCacheMessage {
        HydrateCacheMessage::AudioAnalysis(AnalysisCacheWrite {
            file_path: format!("/synthetic/hydrate-{id}.flac"),
            analyzer: analyzer.to_string(),
            file_size: 42,
            file_mtime: 7,
            analyzer_version: "test-v1".to_string(),
            input_fingerprint: String::new(),
            features_json: "{}".to_string(),
        })
    }

    #[tokio::test]
    async fn hydrate_writer_persists_both_payload_variants_before_ack() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let directory = tempfile::tempdir().expect("mixed writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let cancel = tokio_util::sync::CancellationToken::new();
            let session = HydrateCacheWriterSession::start(store_path.clone(), 2, cancel.clone())
                .expect("mixed writer should start");

            send_cache_message(
                session.sender(),
                hydrate_test_message("mixed enrichment"),
                "mixed enrichment",
            )
            .await
            .expect("mixed enrichment should be acknowledged");
            send_cache_message(
                session.sender(),
                hydrate_test_analysis_message("mixed-analysis", "success"),
                "mixed analysis",
            )
            .await
            .expect("mixed analysis should be acknowledged");

            let completion = session.finish().await;
            assert_eq!(completion.join_failures, 0);
            assert_eq!(completion.report.attempted, 2);
            assert_eq!(completion.report.succeeded, 2);
            assert_eq!(completion.report.failed, 0);
            assert!(!completion.report.threshold_cancelled);
            assert!(!cancel.is_cancelled());

            let connection = state::open(&store_path).expect("mixed store should reopen");
            let enrichment = state::get_enrichment(
                &connection,
                "discogs",
                "writer artist",
                "mixed enrichment",
                None,
                false,
            )
            .expect("mixed enrichment should read")
            .expect("acknowledged enrichment should be durable");
            assert_eq!(enrichment.match_quality.as_deref(), Some("none"));
            assert!(enrichment.response_json.is_none());
            let analysis = state::get_audio_analysis(
                &connection,
                "/synthetic/hydrate-success.flac",
                "mixed-analysis",
            )
            .expect("mixed analysis should read")
            .expect("acknowledged analysis should be durable");
            assert_eq!(analysis.analysis_version, "test-v1");
        })
        .await
        .expect("mixed writer success scenario should finish within five seconds");
    }

    #[tokio::test]
    async fn hydrate_writer_drop_cancels_producer_and_closes_store() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let directory = tempfile::tempdir().expect("writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let cancel = tokio_util::sync::CancellationToken::new();
            let (session, completed) =
                HydrateCacheWriterSession::start_observed(store_path.clone(), 2, cancel.clone())
                    .expect("observed writer should start");
            let sender = session.sender().clone();
            let (queued, queued_rx) = tokio::sync::oneshot::channel();
            let producer = tokio::spawn(async move {
                let (acknowledgement, acknowledged) = tokio::sync::oneshot::channel();
                drop(acknowledged);
                sender
                    .send(CacheWriteRequest {
                        payload: hydrate_test_message("cancelled caller"),
                        acknowledgement,
                    })
                    .await
                    .expect("writer request should queue");
                let _ = queued.send(());
                cancel.cancelled().await;
                drop(sender);
            });
            tokio::time::timeout(Duration::from_secs(1), queued_rx)
                .await
                .expect("queue barrier should be bounded")
                .expect("producer should report queued work");

            drop(session);
            tokio::time::timeout(Duration::from_secs(1), producer)
                .await
                .expect("producer cleanup should be bounded")
                .expect("producer should join");
            tokio::time::timeout(Duration::from_secs(1), completed)
                .await
                .expect("detached writer cleanup should be bounded")
                .expect("writer should report completion");

            let connection = state::open(&store_path).expect("closed store should reopen");
            let cached = state::get_enrichment(
                &connection,
                "discogs",
                "writer artist",
                "cancelled caller",
                None,
                false,
            )
            .expect("cleanup row should read");
            assert!(cached.is_some());
        })
        .await
        .expect("writer cancellation cleanup should finish within five seconds");
    }

    #[tokio::test]
    async fn hydrate_writer_reports_open_write_ack_and_join_failures_separately() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let invalid_directory = tempfile::tempdir().expect("invalid store directory");
            let invalid_path = invalid_directory.path().to_string_lossy().to_string();
            let open_cancel = tokio_util::sync::CancellationToken::new();
            let session = HydrateCacheWriterSession::start(invalid_path, 1, open_cancel.clone())
                .expect("open-failure writer should spawn");
            let rejected = send_cache_message(
                session.sender(),
                hydrate_test_message("open failure"),
                "hydrate test",
            )
            .await;
            assert!(rejected.is_err());
            let completion = session.finish().await;
            assert_eq!(completion.report.open_failures, 1);
            assert_eq!(completion.report.write_failures, 0);
            assert_eq!(completion.report.failed, 1);
            assert_eq!(completion.join_failures, 0);
            assert!(open_cancel.is_cancelled());

            let directory = tempfile::tempdir().expect("selective writer directory");
            let path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let connection = state::open(&path).expect("selective store should open");
            connection
                .execute_batch(
                    "CREATE TRIGGER reject_hydrate_write
                     BEFORE INSERT ON enrichment_cache
                     WHEN NEW.query_title = 'write failure'
                     BEGIN
                       SELECT RAISE(FAIL, 'injected hydrate write failure');
                     END;",
                )
                .expect("failure trigger should install");
            drop(connection);
            let session = HydrateCacheWriterSession::start(
                path.clone(),
                1,
                tokio_util::sync::CancellationToken::new(),
            )
            .expect("selective writer should start");
            let rejected = send_cache_message(
                session.sender(),
                hydrate_test_message("write failure"),
                "hydrate test",
            )
            .await;
            assert!(rejected.is_err());
            let analysis = AnalysisCacheWrite {
                file_path: "/tmp/hydrate-writer-recovery.flac".to_string(),
                analyzer: "test-analyzer".to_string(),
                file_size: 42,
                file_mtime: 7,
                analyzer_version: "test-v1".to_string(),
                input_fingerprint: String::new(),
                features_json: "{}".to_string(),
            };
            send_cache_message(
                session.sender(),
                HydrateCacheMessage::AudioAnalysis(analysis.clone()),
                "hydrate test analysis",
            )
            .await
            .expect("writer should recover for the next mixed payload");
            let completion = session.finish().await;
            assert_eq!(completion.report.open_failures, 0);
            assert_eq!(completion.report.write_failures, 1);
            assert_eq!(completion.report.failed, 1);
            assert_eq!(completion.report.succeeded, 1);
            let connection = state::open(&path).expect("recovered store should reopen");
            let cached =
                state::get_audio_analysis(&connection, &analysis.file_path, &analysis.analyzer)
                    .expect("recovered analysis cache read should succeed")
                    .expect("mixed analysis payload should be durable");
            assert_eq!(cached.analysis_version, analysis.analyzer_version);

            let directory = tempfile::tempdir().expect("ack writer directory");
            let path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let acknowledgement_cancel = tokio_util::sync::CancellationToken::new();
            let session = HydrateCacheWriterSession::start(path, 1, acknowledgement_cancel.clone())
                .expect("ack writer should start");
            let (acknowledgement, acknowledged) = tokio::sync::oneshot::channel();
            drop(acknowledged);
            session
                .sender()
                .send(CacheWriteRequest {
                    payload: hydrate_test_message("dropped ack"),
                    acknowledgement,
                })
                .await
                .expect("dropped-ack request should queue");
            let completion = session.finish().await;
            assert_eq!(completion.report.succeeded, 1);
            assert_eq!(completion.report.dropped_acknowledgements, 1);
            assert!(
                !acknowledgement_cancel.is_cancelled(),
                "a dropped acknowledgement receiver must not cancel the writer"
            );

            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            let panicking = HydrateCacheWriterSession {
                sender: Some(sender),
                writer: Some(tokio::task::spawn_blocking(|| -> CacheWriterReport {
                    panic!("injected hydrate writer panic")
                })),
                cancel: tokio_util::sync::CancellationToken::new(),
                cancel_on_drop: true,
            };
            let completion = panicking.finish().await;
            assert_eq!(completion.join_failures, 1);
            assert_eq!(completion.report.attempted, 0);
            assert_eq!(
                completion.error_summaries,
                ["hydrate cache writer task panicked"]
            );
        })
        .await
        .expect("writer failure matrix should finish within five seconds");
    }

    #[tokio::test]
    async fn hydrate_writer_threshold_cancels_and_drains_later_messages() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let directory = tempfile::tempdir().expect("threshold writer directory");
            let path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let connection = state::open(&path).expect("threshold store should open");
            connection
                .execute_batch(
                    "CREATE TRIGGER reject_every_hydrate_write
                     BEFORE INSERT ON enrichment_cache
                     BEGIN
                       SELECT RAISE(FAIL, 'injected threshold failure');
                     END;",
                )
                .expect("threshold trigger should install");
            drop(connection);

            let cancel = tokio_util::sync::CancellationToken::new();
            let session = HydrateCacheWriterSession::start(path.clone(), 4, cancel.clone())
                .expect("threshold writer should start");
            for index in 0..3 {
                send_cache_message(
                    session.sender(),
                    hydrate_test_message(&format!("threshold {index}")),
                    "hydrate threshold test",
                )
                .await
                .expect_err("threshold fixture should reject every payload");
            }
            let drained_analysis = hydrate_test_analysis_message("post-threshold", "drained");
            let error = send_cache_message(
                session.sender(),
                drained_analysis,
                "hydrate threshold analysis",
            )
            .await
            .expect_err("post-threshold analysis must be rejected while draining");
            assert!(
                error
                    .to_string()
                    .contains("cache writer stopped after 3 consecutive failures")
            );

            let completion = session.finish().await;
            assert!(cancel.is_cancelled());
            assert!(completion.report.threshold_cancelled);
            assert_eq!(completion.report.attempted, 4);
            assert_eq!(completion.report.succeeded, 0);
            assert_eq!(completion.report.failed, 4);
            assert_eq!(completion.report.write_failures, 3);
            let connection = state::open(&path).expect("threshold store should reopen");
            assert!(
                state::get_audio_analysis(
                    &connection,
                    "/synthetic/hydrate-drained.flac",
                    "post-threshold",
                )
                .expect("post-threshold analysis should read")
                .is_none(),
                "post-threshold analysis must be rejected without persistence"
            );
        })
        .await
        .expect("writer threshold scenario should finish within five seconds");
    }

    #[test]
    fn hydrate_application_report_owns_canonical_terminal_accounting() {
        let report = HydrationApplicationReport::assemble(
            HydrationStageReport {
                selected: 5,
                terminal_workers: 3,
                operation_failures: 2,
                worker_join_failures: 1,
                ..HydrationStageReport::default()
            },
            HydrationStageReport {
                selected: 4,
                terminal_workers: 1,
                operation_failures: 3,
                worker_join_failures: 2,
                ..HydrationStageReport::default()
            },
            HydrateCacheWriterCompletion {
                report: CacheWriterReport {
                    failed: 4,
                    ..CacheWriterReport::default()
                },
                join_failures: 1,
                error_summaries: vec!["writer join".to_string()],
            },
        );

        assert_eq!(
            report.final_accounting(),
            HydrationFinalAccounting {
                operation_failures: 5,
                application_join_failures: 4,
                writer_failures: 4,
                incomplete: 5,
            }
        );
        assert_eq!(report.error_summaries, ["writer join"]);
    }

    #[test]
    fn hydrate_writer_spawn_failure_is_typed() {
        let directory = tempfile::tempdir().expect("spawn-failure directory should create");
        let path = directory
            .path()
            .join("internal.sqlite3")
            .to_string_lossy()
            .to_string();
        let (completed, completion) = std::sync::mpsc::sync_channel(1);
        let harness = std::thread::spawn(move || {
            let error = HydrateCacheWriterSession::start(
                path,
                1,
                tokio_util::sync::CancellationToken::new(),
            )
            .err()
            .expect("starting outside Tokio should be reported as a spawn failure");
            let _ = completed.send(error);
        });
        let error = completion
            .recv_timeout(Duration::from_secs(1))
            .expect("spawn-failure harness completion should be bounded");
        harness
            .join()
            .expect("completed spawn-failure harness should join");
        assert!(
            error
                .summary
                .starts_with("hydrate cache writer task spawn failed:")
        );
    }
}
