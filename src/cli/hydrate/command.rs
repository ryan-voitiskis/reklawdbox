use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::adapters::audio as audio_adapter;
use crate::adapters::{rekordbox as db, state as store};
use crate::application::batch::{BatchOutcome, task_join_error_summary};
use crate::application::enrichment::hydrate::{
    EnrichmentTrackOutcome, HydrateCacheWriterSession, HydrationAnalysisOutcome,
    HydrationApplicationReport, HydrationPlan, HydrationSelectionPolicy, HydrationStageReport,
    HydrationWorkerCompletion, LookupFailurePersistence, analysis_stage_report,
    discogs_stage_report, hydrate_discogs_track, run_analysis_stage, run_bounded_workers,
    select_hydration_work,
};
use crate::application::enrichment::model::{EnrichmentProvider, HydrationStage};

use crate::cli::runtime::resources::{
    CpuPreset, analysis_concurrency_for_preset, apply_cpu_niceness, memory_budget_mb,
    track_memory_cost_mb,
};
use crate::cli::runtime::signals::{CliCancellationState, spawn_signal_handlers};

use super::{
    HydrateArgs, discogs as discogs_cli,
    presentation::{
        self, FinalPresentation, ProgressDisplay, ProviderCounters, StartupPresentation,
    },
};

pub(super) struct HydrateTask<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> HydrateTask<T> {
    pub(super) fn spawn(future: impl std::future::Future<Output = T> + Send + 'static) -> Self
    where
        T: Send + 'static,
    {
        Self {
            handle: Some(tokio::spawn(future)),
        }
    }

    pub(super) async fn join(mut self) -> Result<T, tokio::task::JoinError> {
        let result = self
            .handle
            .as_mut()
            .expect("hydrate task handle should be present")
            .await;
        self.handle.take();
        result
    }
}

impl<T> Drop for HydrateTask<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

pub(super) fn resolve_provider_task_join(
    provider: &'static str,
    selected: usize,
    counters: &ProviderCounters,
    result: Result<HydrationStageReport, tokio::task::JoinError>,
) -> HydrationStageReport {
    match result {
        Ok(report) => report,
        Err(error) => {
            counters.record_join_failure();
            let summary = task_join_error_summary(&format!("{provider} provider task"), &error);
            tracing::error!("{summary}: {error}");
            HydrationStageReport::failed_task(selected, counters.terminal() as usize, summary)
        }
    }
}

pub(super) async fn await_discogs_hydration<HydrationFuture>(
    cancel: &CancellationToken,
    hydration: HydrationFuture,
) -> Option<EnrichmentTrackOutcome>
where
    HydrationFuture: std::future::Future<Output = EnrichmentTrackOutcome>,
{
    tokio::select! {
        outcome = hydration => Some(outcome),
        _ = cancel.cancelled() => None,
    }
}

pub(super) async fn await_analysis_hydration<AnalysisFuture>(
    cancel: &CancellationToken,
    analysis: AnalysisFuture,
) -> HydrationAnalysisOutcome
where
    AnalysisFuture: std::future::Future<Output = HydrationAnalysisOutcome>,
{
    tokio::pin!(analysis);
    tokio::select! {
        outcome = &mut analysis => outcome,
        // Stratum decode/DSP uses spawn_blocking, which cannot be aborted once
        // running. Drain the already-started stage so command cancellation
        // cannot detach blocking work or close its cache acknowledgement path.
        _ = cancel.cancelled() => analysis.await,
    }
}

enum PreparedHydration {
    NoTracks,
    AllCached { total_tracks: usize },
    Ready(ReadyHydration),
}

struct ReadyHydration {
    client: reqwest::Client,
    store_path: String,
    plan: HydrationPlan,
    essentia_python: Option<String>,
    want_discogs: bool,
    want_analysis: bool,
    retry_errors: bool,
    cpu_preset: CpuPreset,
    analysis_concurrency: usize,
    analysis_budget_mb: u32,
    enrich_concurrency: usize,
    skip_confirmation: bool,
}

struct HydrationExecution {
    elapsed: Duration,
    want_discogs: bool,
    want_analysis: bool,
    application: HydrationApplicationReport,
    status_join_failures: u32,
    user_cancelled: bool,
    status_error_summaries: Vec<String>,
}

pub(crate) async fn run_hydrate(args: HydrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let ready = match prepare_hydration(args)? {
        PreparedHydration::NoTracks => {
            println!("No tracks match the given filters.");
            return Ok(());
        }
        PreparedHydration::AllCached { total_tracks } => {
            println!("Found {total_tracks} tracks matching filters.");
            println!("All cached. Nothing to do.");
            return Ok(());
        }
        PreparedHydration::Ready(ready) => ready,
    };

    presentation::print_startup(StartupPresentation {
        plan: &ready.plan,
        want_discogs: ready.want_discogs,
        want_analysis: ready.want_analysis,
        retry_errors: ready.retry_errors,
        cpu_preset: ready.cpu_preset,
        analysis_concurrency: ready.analysis_concurrency,
        analysis_budget_mb: ready.analysis_budget_mb,
        essentia_available: ready.essentia_python.is_some(),
    });
    if !presentation::confirm(ready.skip_confirmation)? {
        println!("Aborted.");
        return Ok(());
    }

    let discogs_mode = if ready.want_discogs && !ready.plan.discogs_tracks.is_empty() {
        Some(discogs_cli::ensure_auth(&ready.client, &ready.store_path).await?)
    } else {
        None
    };
    finish_hydration(execute_hydration(ready, discogs_mode).await?)
}

fn prepare_hydration(args: HydrateArgs) -> Result<PreparedHydration, Box<dyn std::error::Error>> {
    let cpu_preset = args.cpu;
    apply_cpu_niceness(cpu_preset);
    let analysis_concurrency = analysis_concurrency_for_preset(cpu_preset);
    let analysis_budget_mb = memory_budget_mb(cpu_preset);

    let want_discogs = args
        .providers
        .contains(HydrationStage::Lookup(EnrichmentProvider::Discogs));
    let want_analysis = args.providers.contains(HydrationStage::Analysis);

    // 1. Bootstrap
    let db_path = db::resolve_db_path().ok_or(
        "Cannot find Rekordbox database. Set REKORDBOX_DB_PATH or ensure Rekordbox is installed.",
    )?;
    let conn = db::open(&db_path)?;
    let store_path = store::resolve_path();
    let store_path_str = store_path
        .to_str()
        .ok_or("Invalid store path encoding")?
        .to_string();
    let store_conn = store::open(&store_path_str)?;
    let client = reqwest::Client::builder()
        .user_agent("Reklawdbox/0.1")
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .expect("failed to build HTTP client");

    let essentia_python = if want_analysis {
        audio_adapter::probe_essentia_python_path()
    } else {
        None
    };

    // 2. Resolve tracks
    let params = db::SearchParams {
        query: args.query,
        artist: args.artist,
        genre: args.genre,
        rating_min: args.rating_min,
        bpm_min: args.bpm_min,
        bpm_max: args.bpm_max,
        key: args.key,
        playlist: args.playlist,
        has_genre: None,
        has_label: None,
        year_zero: None,
        label: args.label,
        path: args.path,
        path_prefix: None,
        added_after: args
            .added_after
            .map(|s| db::validate_iso_date(&s, "added_after"))
            .transpose()?,
        added_before: args
            .added_before
            .map(|s| db::validate_iso_date(&s, "added_before"))
            .transpose()?,
        exclude_samples: true,
        limit: args.max_tracks,
        offset: None,
    };
    let tracks = db::search_tracks_unbounded(&conn, &params)?;
    drop(conn);

    if tracks.is_empty() {
        return Ok(PreparedHydration::NoTracks);
    }

    // 3. Select work through the application-owned cache policy.
    let retry_errors = !args.no_retry_errors;
    let plan = select_hydration_work(
        &store_conn,
        &tracks,
        &HydrationSelectionPolicy {
            stages: args.providers.clone(),
            retry_cached_errors: retry_errors,
            essentia_available: essentia_python.is_some(),
        },
    )?;
    drop(store_conn);

    let total_tracks = plan.total_matched_tracks;
    let total_work = plan.total_work();

    if total_work == 0 {
        return Ok(PreparedHydration::AllCached { total_tracks });
    }

    Ok(PreparedHydration::Ready(ReadyHydration {
        client,
        store_path: store_path_str,
        plan,
        essentia_python,
        want_discogs,
        want_analysis,
        retry_errors,
        cpu_preset,
        analysis_concurrency,
        analysis_budget_mb,
        enrich_concurrency: args.concurrency.unwrap_or(4).clamp(1, 16) as usize,
        skip_confirmation: args.yes,
    }))
}

async fn execute_hydration(
    ready: ReadyHydration,
    discogs_mode: Option<(crate::adapters::providers::discogs::BrokerConfig, String)>,
) -> Result<HydrationExecution, Box<dyn std::error::Error>> {
    let ReadyHydration {
        client,
        store_path,
        plan,
        essentia_python,
        want_discogs,
        want_analysis,
        analysis_concurrency,
        analysis_budget_mb,
        enrich_concurrency,
        ..
    } = ready;
    let discogs_selected = plan.discogs_tracks.len();
    let analysis_selected = plan.analysis_jobs.len();
    let total_work = plan.total_work();
    let cancel = CancellationToken::new();
    let progress = ProgressDisplay::new(total_work);

    let cancellation_state = CliCancellationState::default();
    spawn_signal_handlers(&progress.multi, &cancel, &cancellation_state);

    let discogs_counters = Arc::new(ProviderCounters::new());
    let analysis_counters = Arc::new(ProviderCounters::new());

    let writer_session =
        HydrateCacheWriterSession::start(store_path, enrich_concurrency * 8 + 32, cancel.clone())?;
    let cache_tx = writer_session.sender().clone();

    // Spawn provider loops concurrently.
    let dc = discogs_counters.clone();
    let ac = analysis_counters.clone();
    let status_cancel = cancel.clone();
    let status_pb_clone = progress.status.clone();
    let want_d = want_discogs;
    let want_a = want_analysis;
    let status_task = HydrateTask::spawn(async move {
        loop {
            if status_cancel.is_cancelled() {
                break;
            }
            let mut parts = Vec::new();
            if want_d {
                parts.push(format!(
                    "Discogs: {} enriched, {} errors",
                    dc.enriched(),
                    dc.errors(),
                ));
            }
            if want_a {
                parts.push(format!(
                    "Analysis: {} done, {} errors",
                    ac.enriched(),
                    ac.errors(),
                ));
            }
            status_pb_clone.set_message(parts.join(" | "));
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    let batch_start = Instant::now();
    let discogs_pending = plan.discogs_tracks;
    let analysis_pending = plan.analysis_jobs;

    let discogs_task = {
        let cancel = cancel.clone();
        let client = client.clone();
        let cache_tx = cache_tx.clone();
        let counters = discogs_counters.clone();
        let pb = progress.primary.clone();
        HydrateTask::spawn(async move {
            if discogs_pending.is_empty() {
                return HydrationStageReport {
                    selected: discogs_selected,
                    ..HydrationStageReport::default()
                };
            }
            let (broker_cfg, session_token) = match discogs_mode {
                Some(m) => m,
                None => {
                    return HydrationStageReport {
                        selected: discogs_selected,
                        ..HydrationStageReport::default()
                    };
                }
            };
            let worker_counters = counters.clone();
            let workers = run_bounded_workers(
                "discogs worker task",
                discogs_pending,
                enrich_concurrency,
                cancel.clone(),
                |identity| identity.clone(),
                move |identity: crate::application::enrichment::hydrate::HydrationTrackIdentity| {
                    let client = client.clone();
                    let cache_tx = cache_tx.clone();
                    let counters = worker_counters.clone();
                    let pb = pb.clone();
                    let cfg = broker_cfg.clone();
                    let token = session_token.clone();
                    let cancel = cancel.clone();
                    async move {
                        if cancel.is_cancelled() {
                            return HydrationWorkerCompletion::cancelled(
                                EnrichmentTrackOutcome::default(),
                            );
                        }

                        let hydration = hydrate_discogs_track(
                            &identity,
                            LookupFailurePersistence::CacheTerminalError,
                            None,
                            &cache_tx,
                            discogs_cli::lookup_with_retry(
                                &client,
                                &cfg,
                                &token,
                                &identity.artist,
                                &identity.title,
                                Some(&identity.album),
                            ),
                        );
                        let Some(outcome) = await_discogs_hydration(&cancel, hydration).await
                        else {
                            return HydrationWorkerCompletion::cancelled(
                                EnrichmentTrackOutcome::default(),
                            );
                        };

                        counters.observe_discogs(&identity, &outcome);

                        counters.record_terminal();
                        pb.inc(1);
                        HydrationWorkerCompletion::completed(outcome)
                    }
                },
            )
            .await;

            discogs_stage_report(workers)
        })
    };

    let analysis_task = {
        let cancel = cancel.clone();
        let cache_tx = cache_tx.clone();
        let counters = analysis_counters.clone();
        let pb = progress.primary.clone();
        HydrateTask::spawn(async move {
            if analysis_pending.is_empty() {
                return HydrationStageReport {
                    selected: analysis_selected,
                    ..HydrationStageReport::default()
                };
            }
            let mem_sem = Arc::new(tokio::sync::Semaphore::new(analysis_budget_mb as usize));
            let worker_counters = counters.clone();
            let workers = run_bounded_workers(
                "analysis worker task",
                analysis_pending,
                analysis_concurrency,
                cancel.clone(),
                |job| job.track.id.clone(),
                move |job: crate::application::enrichment::hydrate::HydrationAnalysisJob| {
                    let essentia_python = essentia_python.clone();
                    let cache_tx = cache_tx.clone();
                    let counters = worker_counters.clone();
                    let pb = pb.clone();
                    let cancel = cancel.clone();
                    let mem_sem = mem_sem.clone();
                    async move {
                        if cancel.is_cancelled() {
                            return HydrationWorkerCompletion::cancelled(
                                HydrationAnalysisOutcome::default(),
                            );
                        }

                        let cost_mb =
                            track_memory_cost_mb(job.track.length).min(analysis_budget_mb);
                        let mem_permit = tokio::select! {
                            result = mem_sem.acquire_many_owned(cost_mb) => match result {
                                Ok(permit) => permit,
                                Err(_) => return HydrationWorkerCompletion::cancelled(
                                    HydrationAnalysisOutcome::default(),
                                ),
                            },
                            _ = cancel.cancelled() => return HydrationWorkerCompletion::cancelled(
                                HydrationAnalysisOutcome::default(),
                            ),
                        };

                        let analysis = run_analysis_stage(
                            &job.track.file_path,
                            job.needs_stratum,
                            job.needs_essentia,
                            essentia_python.as_deref(),
                            &cache_tx,
                        );
                        let outcome = await_analysis_hydration(&cancel, analysis).await;

                        counters.observe_analysis(&outcome);

                        counters.record_terminal();
                        pb.inc(1);
                        drop(mem_permit);
                        HydrationWorkerCompletion::completed(outcome)
                    }
                },
            )
            .await;

            analysis_stage_report(workers)
        })
    };

    // Drop our sender so the writer sees EOF when all producer tasks finish
    drop(cache_tx);

    let discogs_report = resolve_provider_task_join(
        "discogs",
        discogs_selected,
        &discogs_counters,
        discogs_task.join().await,
    );
    let analysis_report = resolve_provider_task_join(
        "analysis",
        analysis_selected,
        &analysis_counters,
        analysis_task.join().await,
    );

    let user_cancelled = cancellation_state.user_requested();
    cancel.cancel();
    let mut status_error_summaries = Vec::new();
    let status_join_failures = match status_task.join().await {
        Ok(()) => 0_u32,
        Err(error) => {
            let summary = task_join_error_summary("hydrate status task", &error);
            tracing::error!("{summary}: {error}");
            status_error_summaries.push(summary);
            1
        }
    };
    let writer_completion = writer_session.finish().await;
    progress.finish();

    let elapsed = batch_start.elapsed();
    if discogs_report.worker_join_failures == 0 {
        debug_assert_eq!(discogs_counters.enriched(), discogs_report.enriched);
        debug_assert_eq!(discogs_counters.no_match(), discogs_report.no_match);
        debug_assert_eq!(
            discogs_counters.operation_errors(),
            discogs_report.operation_failures
        );
    }
    if analysis_report.worker_join_failures == 0 {
        debug_assert_eq!(analysis_counters.enriched(), analysis_report.enriched);
        debug_assert_eq!(
            analysis_counters.operation_errors(),
            analysis_report.operation_failures
        );
    }
    let application =
        HydrationApplicationReport::assemble(discogs_report, analysis_report, writer_completion);

    Ok(HydrationExecution {
        elapsed,
        want_discogs,
        want_analysis,
        application,
        status_join_failures,
        user_cancelled,
        status_error_summaries,
    })
}

fn finish_hydration(execution: HydrationExecution) -> Result<(), Box<dyn std::error::Error>> {
    let HydrationExecution {
        elapsed,
        want_discogs,
        want_analysis,
        application,
        status_join_failures,
        user_cancelled,
        status_error_summaries,
    } = execution;
    let accounting = application.final_accounting();
    presentation::print_final(FinalPresentation {
        elapsed,
        want_discogs,
        want_analysis,
        discogs: &application.discogs,
        analysis: &application.analysis,
        writer: &application.writer,
        writer_join_failures: application.writer_join_failures,
        incomplete: accounting.incomplete,
        user_cancelled,
    });

    hydrate_batch_outcome(
        &application,
        status_join_failures,
        user_cancelled,
        status_error_summaries,
    )
    .finish()
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
}

pub(super) fn hydrate_batch_outcome(
    application: &HydrationApplicationReport,
    status_join_failures: u32,
    user_cancelled: bool,
    status_error_summaries: Vec<String>,
) -> BatchOutcome {
    let accounting = application.final_accounting();
    let mut error_summaries = application.error_summaries.clone();
    error_summaries.extend(status_error_summaries);
    BatchOutcome {
        command: "hydrate",
        operation_failures: accounting.operation_failures,
        worker_join_failures: accounting.application_join_failures + status_join_failures,
        writer_failures: accounting.writer_failures,
        incomplete: accounting.incomplete,
        user_cancelled,
        error_summaries,
    }
}
