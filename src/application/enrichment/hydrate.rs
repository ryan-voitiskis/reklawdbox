//! Shared enrichment hydration workflow.

use crate::adapters::{providers, state};
use crate::application::analysis::batch::{CacheWriteRequest, send_cache_message};
use crate::application::analysis::{job as analysis_job, model::AnalysisCacheWrite};

use super::lookup::{self, LookupIdentity};
use super::model::{EnrichmentProvider, HydrationStage};

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

#[derive(Clone, Debug)]
pub(crate) enum HydrationFailureKind {
    AuthBatchFailed,
    DiscogsAuth(providers::discogs::AuthRemediation),
    Lookup(String),
    Serialize(String),
    SemaphoreClosed,
    CacheWrite(String),
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
    pub(crate) failures: Vec<HydrationFailure>,
}

impl EnrichmentTrackOutcome {
    fn absorb(&mut self, other: Self) {
        self.enriched += other.enriched;
        self.cached += other.cached;
        self.no_match += other.no_match;
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
    let mut handles = Vec::with_capacity(selected);

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
        handles.push((
            item_identity,
            tokio::spawn(async move {
                let result = worker(item).await;
                drop(permit);
                result
            }),
        ));
    }

    let scheduled = handles.len();
    let mut terminal_workers = 0usize;
    let mut completed = Vec::with_capacity(scheduled);
    let mut join_failures = Vec::new();
    for (identity, handle) in handles {
        match handle.await {
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

pub(crate) async fn acknowledge_enrichment_cache_write<T>(
    tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
    write: EnrichmentCacheWrite,
    context: &str,
) -> Result<(), String>
where
    T: From<EnrichmentCacheWrite>,
{
    send_cache_message(tx, T::from(write), context).await
}

/// Queue an MCP enrichment write while preserving its original public errors.
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
    EnrichmentTrackOutcome {
        failures: vec![HydrationFailure {
            identity: identity.clone(),
            provider,
            kind,
        }],
        ..EnrichmentTrackOutcome::default()
    }
}

async fn persist_provider_outcome<T: serde::Serialize>(
    identity: &HydrationTrackIdentity,
    provider: EnrichmentProvider,
    result: Option<T>,
    quality: impl FnOnce(&T) -> &'static str,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
) -> EnrichmentTrackOutcome {
    let (match_quality, response_json, matched) = match result {
        Some(result) => {
            let match_quality = quality(&result).to_string();
            let response_json = match serde_json::to_string(&result) {
                Ok(response_json) => response_json,
                Err(error) => {
                    return failed_stage(
                        identity,
                        provider,
                        HydrationFailureKind::Serialize(format!("Serialize error: {error}")),
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
    if let Err(error) = acknowledge_mcp_enrichment_cache_write(cache_tx, write).await {
        return failed_stage(identity, provider, HydrationFailureKind::CacheWrite(error));
    }

    EnrichmentTrackOutcome {
        enriched: usize::from(matched),
        no_match: usize::from(!matched),
        ..EnrichmentTrackOutcome::default()
    }
}

async fn run_discogs_stage<LookupFuture>(
    need: bool,
    identity: &HydrationTrackIdentity,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    auth_failed: &tokio::sync::watch::Receiver<bool>,
    auth_fail_tx: &tokio::sync::watch::Sender<bool>,
    lookup: LookupFuture,
) -> EnrichmentTrackOutcome
where
    LookupFuture: std::future::Future<
            Output = Result<
                Option<providers::discogs::DiscogsResult>,
                providers::discogs::LookupError,
            >,
        >,
{
    if !need {
        return EnrichmentTrackOutcome::default();
    }
    if *auth_failed.borrow() {
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
                let _ = auth_fail_tx.send(true);
                HydrationFailureKind::DiscogsAuth(remediation.clone())
            } else {
                HydrationFailureKind::Lookup(error.to_string())
            };
            failed_stage(identity, EnrichmentProvider::Discogs, kind)
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

#[allow(clippy::too_many_arguments)]
async fn run_track_enrichment<DiscogsFuture>(
    identity: HydrationTrackIdentity,
    providers: &[EnrichmentProvider],
    policy: EnrichmentCachePolicy,
    store_path: &str,
    http: &reqwest::Client,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    bandcamp_gate: std::sync::Arc<tokio::sync::Semaphore>,
    auth_failed: &tokio::sync::watch::Receiver<bool>,
    auth_fail_tx: &tokio::sync::watch::Sender<bool>,
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
    let plan = provider_stage_plan(store_path, &identity, providers, policy);
    let discogs = run_discogs_stage(
        plan.need_discogs,
        &identity,
        cache_tx,
        auth_failed,
        auth_fail_tx,
        discogs_lookup,
    );
    let bandcamp = run_bandcamp_stage(plan.need_bandcamp, &identity, http, bandcamp_gate, cache_tx);
    let (discogs, bandcamp) = tokio::join!(discogs, bandcamp);

    let mut outcome = EnrichmentTrackOutcome {
        cached: plan.cached,
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
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_enrichment_workers<DiscogsLookup, DiscogsFuture>(
    tracks: Vec<HydrationTrackIdentity>,
    config: EnrichmentWorkerConfig,
    http: reqwest::Client,
    cache_tx: tokio::sync::mpsc::Sender<CacheWriteRequest<EnrichmentCacheWrite>>,
    auth_failed: std::sync::Arc<tokio::sync::watch::Receiver<bool>>,
    auth_fail_tx: std::sync::Arc<tokio::sync::watch::Sender<bool>>,
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
            let auth_failed = auth_failed.clone();
            let auth_fail_tx = auth_fail_tx.clone();
            let discogs_lookup = discogs_lookup.clone();
            async move {
                let discogs_future = discogs_lookup(identity.clone());
                let outcome = run_track_enrichment(
                    identity,
                    &providers,
                    config.policy,
                    &store_path,
                    &http,
                    &cache_tx,
                    bandcamp_gate,
                    &auth_failed,
                    &auth_fail_tx,
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
}
