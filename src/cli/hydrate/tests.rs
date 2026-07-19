use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{CommandFactory, Parser};

use crate::adapters::providers::discogs;
use crate::adapters::state;
use crate::application::analysis::model::AnalysisCacheWrite;
use crate::application::cache_writer::{CacheWriteRequest, CacheWriterReport, send_cache_message};
use crate::application::enrichment::hydrate::{
    EnrichmentCacheWrite, EnrichmentTrackOutcome, HydrateCacheMessage,
    HydrateCacheWriterCompletion, HydrateCacheWriterSession, HydrationAnalysisOutcome,
    HydrationApplicationReport, HydrationFailureKind, HydrationStageReport, HydrationTrackIdentity,
    HydrationWorkerCompletion, LookupFailurePersistence, discogs_stage_report,
    hydrate_discogs_track, run_bounded_workers,
};
use crate::application::enrichment::model::{EnrichmentProvider, HydrationStage};
use crate::cli::command::Cli;

use super::command::{
    HydrateAnalysisTask, HydrateTask, hydrate_batch_outcome, resolve_provider_task_join,
};
use super::discogs as discogs_cli;
use super::presentation::{
    FinalPresentation, ProviderCounters, final_lines, hydration_failure_message,
};

#[test]
fn hydrate_help_and_defaults_preserve_the_complete_cli_contract() {
    let mut command = Cli::command();
    command.build();
    let hydrate = command
        .get_subcommands()
        .find(|subcommand| subcommand.get_name() == "hydrate")
        .expect("hydrate subcommand should exist");
    let long_flags: Vec<_> = hydrate
        .get_arguments()
        .filter_map(|argument| argument.get_long())
        .collect();
    assert_eq!(
        long_flags,
        [
            "providers",
            "playlist",
            "artist",
            "genre",
            "bpm-min",
            "bpm-max",
            "key",
            "label",
            "path",
            "query",
            "added-after",
            "added-before",
            "rating-min",
            "max-tracks",
            "no-retry-errors",
            "cpu",
            "concurrency",
            "yes",
            "help",
        ]
    );
    let providers = hydrate
        .get_arguments()
        .find(|argument| argument.get_id() == "providers")
        .expect("providers argument should exist");
    assert_eq!(
        providers
            .get_default_values()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>(),
        ["discogs,analysis"]
    );
    let cpu = hydrate
        .get_arguments()
        .find(|argument| argument.get_id() == "cpu")
        .expect("cpu argument should exist");
    assert_eq!(
        cpu.get_default_values()
            .iter()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>(),
        ["background"]
    );

    let Cli::Hydrate(args) =
        Cli::try_parse_from(["reklawdbox", "hydrate"]).expect("hydrate defaults should parse")
    else {
        panic!("hydrate command should parse as hydrate");
    };
    assert!(
        args.providers
            .contains(HydrationStage::Lookup(EnrichmentProvider::Discogs))
    );
    assert!(args.providers.contains(HydrationStage::Analysis));
    assert!(!args.no_retry_errors);
    assert!(args.concurrency.is_none());
    assert!(!args.yes);
    assert_eq!(args.cpu.to_string(), "background");
}

#[test]
fn discogs_cli_retry_keeps_bounded_sanitized_diagnostic_out_of_display() {
    let remote_instruction = "CLI_REMOTE_DIAGNOSTIC_FIXTURE";
    let retained = format!(
        "{remote_instruction}{}",
        "x".repeat(8_192 - remote_instruction.len())
    );
    let diagnostic = format!("{retained} [truncated]");
    let error = discogs::LookupError::http(503, None, diagnostic.clone());

    let retry = discogs_cli::http_retry_metadata(&error, 0)
        .expect("HTTP 503 should retain CLI retry metadata");
    assert_eq!(retry.status, 503);
    assert_eq!(retry.wait_seconds, 5);
    assert_eq!(retry.diagnostic_body, diagnostic);
    assert!(retry.diagnostic_body.len() <= 8_192 + " [truncated]".len());
    assert!(
        !retry
            .diagnostic_body
            .chars()
            .any(|character| character.is_ascii_control())
    );
    assert!(retry.diagnostic_body.contains(remote_instruction));
    assert!(!error.to_string().contains(remote_instruction));
    assert_eq!(error.to_string(), "broker proxy HTTP 503 (retryable)");
}

#[test]
fn discogs_cli_retry_metadata_preserves_429_cap_and_5xx_backoff() {
    let rate_limited =
        discogs::LookupError::http(429, Some("999".to_string()), "rate limited".to_string());
    let retry = discogs_cli::http_retry_metadata(&rate_limited, 3)
        .expect("HTTP 429 should remain retryable");
    assert_eq!(retry.wait_seconds, 120);
    assert_eq!(retry.diagnostic_body, "rate limited");

    let unavailable = discogs::LookupError::http(503, None, "unavailable".to_string());
    let retry = discogs_cli::http_retry_metadata(&unavailable, 3)
        .expect("HTTP 503 should remain retryable");
    assert_eq!(retry.wait_seconds, 40);

    let bad_request = discogs::LookupError::http(400, None, "bad request".to_string());
    assert!(discogs_cli::http_retry_metadata(&bad_request, 0).is_none());
}

#[test]
fn hydrate_cli_serialization_failure_keeps_legacy_single_prefix() {
    assert_eq!(
        hydration_failure_message(&HydrationFailureKind::Serialize(
            "synthetic serializer detail".to_string()
        )),
        "discogs enrichment cache serialization failed: synthetic serializer detail"
    );
}

#[tokio::test]
async fn hydrate_discogs_retry_uses_four_attempts_and_exact_injected_backoff() {
    async fn exercise(error: discogs::LookupError, expected_waits: Vec<Duration>) {
        let calls = Arc::new(AtomicU32::new(0));
        let waits = Arc::new(Mutex::new(Vec::new()));
        let result = discogs_cli::lookup_with_retry_for_test(
            {
                let calls = calls.clone();
                move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                    let error = error.clone();
                    async move {
                        Err::<Option<discogs::DiscogsResult>, discogs::LookupError>(error)
                    }
                }
            },
            {
                let waits = waits.clone();
                move |duration| {
                    waits
                        .lock()
                        .expect("retry wait recorder should remain available")
                        .push(duration);
                    std::future::ready(())
                }
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 4);
        assert_eq!(
            *waits.lock().expect("retry waits should remain available"),
            expected_waits
        );
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        exercise(
            discogs::LookupError::http(429, Some("7".to_string()), "rate limited".to_string()),
            vec![Duration::from_secs(7); 3],
        )
        .await;
        exercise(
            discogs::LookupError::http(503, None, "unavailable".to_string()),
            vec![
                Duration::from_secs(5),
                Duration::from_secs(10),
                Duration::from_secs(20),
            ],
        )
        .await;
        exercise(
            discogs::LookupError::message("synthetic transport failure"),
            vec![
                Duration::from_secs(5),
                Duration::from_secs(10),
                Duration::from_secs(20),
            ],
        )
        .await;
    })
    .await
    .expect("injected Discogs retry matrix should finish within five seconds");
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
async fn hydrate_outer_task_drop_aborts_status_or_provider_future() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let started = Arc::new(tokio::sync::Notify::new());
        let (dropped, dropped_rx) = tokio::sync::oneshot::channel();
        let task = HydrateTask::spawn({
            let started = started.clone();
            async move {
                let _drop_signal = DropSignal(Some(dropped));
                started.notify_one();
                std::future::pending::<()>().await;
            }
        });
        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .expect("task-start barrier should be bounded");
        drop(task);
        tokio::time::timeout(Duration::from_secs(1), dropped_rx)
            .await
            .expect("outer task cleanup should be bounded")
            .expect("outer task future should be dropped");
    })
    .await
    .expect("outer task cleanup scenario should finish within five seconds");
}

#[tokio::test]
async fn hydrate_analysis_owner_drop_drains_blocking_cache_lifecycle() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let normal_cancel = tokio_util::sync::CancellationToken::new();
        let (normally_joined, unexpected_reaper) =
            HydrateAnalysisTask::spawn_observed(normal_cancel.clone(), async { "joined" });
        assert_eq!(
            normally_joined
                .join()
                .await
                .expect("normal analysis owner should join"),
            "joined"
        );
        assert!(
            tokio::time::timeout(Duration::from_secs(1), unexpected_reaper)
                .await
                .expect("normal-join observer closure should be bounded")
                .is_err(),
            "normal analysis join must not spawn a detached reaper"
        );
        assert!(
            !normal_cancel.is_cancelled(),
            "normal analysis join must not cancel its scheduling token"
        );

        let directory = tempfile::tempdir().expect("analysis owner directory should create");
        let store_path = directory
            .path()
            .join("internal.sqlite3")
            .to_string_lossy()
            .to_string();
        let cancel = tokio_util::sync::CancellationToken::new();
        let (writer_session, mut writer_completed) =
            HydrateCacheWriterSession::start_observed(store_path.clone(), 2, cancel.clone())
                .expect("observed hydrate writer should start");
        let cache_tx = writer_session.sender().clone();

        let worker_calls = Arc::new(AtomicU32::new(0));
        let (blocking_started, blocking_started_rx) = tokio::sync::oneshot::channel();
        let (blocking_release, blocking_release_rx) = std::sync::mpsc::sync_channel(1);
        let (cache_acknowledged, cache_acknowledged_rx) = tokio::sync::oneshot::channel();
        let (coordinator_completed, coordinator_completed_rx) = tokio::sync::oneshot::channel();
        let (analysis_owner, reaper_completed) = HydrateAnalysisTask::spawn_observed(
            cancel.clone(),
            {
                let worker_calls = worker_calls.clone();
                let coordinator_cancel = cancel.clone();
                async move {
                let report = run_bounded_workers(
                    "analysis worker task",
                    vec![
                        (
                            1_u8,
                            Some(blocking_started),
                            Some(blocking_release_rx),
                            Some(cache_acknowledged),
                        ),
                        (2_u8, None, None, None),
                    ],
                    1,
                    coordinator_cancel,
                    |(identity, _, _, _)| *identity,
                    move |(_, started, release, acknowledged)| {
                        let worker_calls = worker_calls.clone();
                        let cache_tx = cache_tx.clone();
                        async move {
                            worker_calls.fetch_add(1, Ordering::Relaxed);
                            let write = tokio::task::spawn_blocking(move || {
                                started
                                    .expect("scheduled blocking analysis should have a start barrier")
                                    .send(())
                                    .expect("blocking analysis start should be observed");
                                release
                                    .expect("scheduled blocking analysis should have a release")
                                    .recv_timeout(Duration::from_secs(2))
                                    .expect("blocking analysis release should be bounded");
                                AnalysisCacheWrite {
                                    file_path: "/synthetic/hard-drop-analysis.flac".to_string(),
                                    analyzer: "hard-drop-analysis".to_string(),
                                    file_size: 42,
                                    file_mtime: 7,
                                    analyzer_version: "test-v1".to_string(),
                                    input_fingerprint: String::new(),
                                    features_json: "{}".to_string(),
                                }
                            })
                            .await
                            .expect("blocking analysis executor should join");
                            send_cache_message(
                                &cache_tx,
                                HydrateCacheMessage::AudioAnalysis(write),
                                "hard-drop analysis",
                            )
                            .await
                            .expect("hard-drop analysis cache write should be acknowledged");
                            acknowledged
                                .expect("scheduled analysis should have an acknowledgement observer")
                                .send(())
                                .expect("analysis cache acknowledgement should be observed");
                            HydrationWorkerCompletion::completed(
                                HydrationAnalysisOutcome::default(),
                            )
                        }
                    },
                )
                .await;
                let _ = coordinator_completed.send((
                    report.selected,
                    report.scheduled,
                    report.terminal_workers,
                    report.incomplete(),
                    report.join_failures.len(),
                ));
                report
            }
            },
        );

        tokio::time::timeout(Duration::from_secs(1), blocking_started_rx)
            .await
            .expect("blocking analysis start should be bounded")
            .expect("blocking analysis should start");
        assert_eq!(worker_calls.load(Ordering::Relaxed), 1);

        // Hard caller drop synchronously stops further scheduling, then hands
        // the coordinator to its detached reaper. The writer deliberately
        // remains alive while the started worker drains and acknowledges.
        drop(analysis_owner);
        assert!(
            cancel.is_cancelled(),
            "analysis owner drop must synchronously cancel scheduling"
        );
        assert!(
            matches!(
                writer_completed.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "writer must remain alive until its owner is explicitly finished"
        );
        tokio::task::yield_now().await;
        blocking_release
            .send(())
            .expect("started blocking analysis should still accept release");

        tokio::time::timeout(Duration::from_secs(1), cache_acknowledged_rx)
            .await
            .expect("cache acknowledgement should be bounded")
            .expect("started analysis should retain its cache acknowledgement");
        let coordinator_report =
            tokio::time::timeout(Duration::from_secs(1), coordinator_completed_rx)
                .await
                .expect("analysis coordinator completion should be bounded")
                .expect("analysis coordinator should report terminal accounting");
        assert_eq!(coordinator_report, (2, 1, 1, 1, 0));
        assert_eq!(worker_calls.load(Ordering::Relaxed), 1);
        tokio::time::timeout(Duration::from_secs(1), reaper_completed)
            .await
            .expect("analysis reaper completion should be bounded")
            .expect("detached analysis reaper should finish");
        let writer_completion = tokio::time::timeout(
            Duration::from_secs(1),
            writer_session.finish(),
        )
        .await
        .expect("writer finish should be bounded");
        assert_eq!(writer_completion.join_failures, 0);
        assert_eq!(writer_completion.report.attempted, 1);
        assert_eq!(writer_completion.report.succeeded, 1);
        assert_eq!(writer_completion.report.failed, 0);
        tokio::time::timeout(Duration::from_secs(1), writer_completed)
            .await
            .expect("writer completion observer should be bounded")
            .expect("writer should close its store");

        let connection = state::open(&store_path).expect("completed writer store should reopen");
        let cached = state::get_audio_analysis(
            &connection,
            "/synthetic/hard-drop-analysis.flac",
            "hard-drop-analysis",
        )
        .expect("hard-drop analysis cache should read")
        .expect("acknowledged hard-drop analysis row should be durable");
        assert_eq!(cached.analysis_version, "test-v1");
    })
    .await
    .expect("hard-drop analysis ownership should finish within five seconds");
}

#[test]
fn hydrate_analysis_owner_drop_without_current_runtime_retains_full_cleanup() {
    const WALL_CLOCK_WATCHDOG: Duration = Duration::from_secs(5);

    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "fallback proof must drop the owner outside an entered Tokio runtime"
    );

    let directory = tempfile::tempdir().expect("fallback analysis directory should create");
    let store_path = directory
        .path()
        .join("internal.sqlite3")
        .to_string_lossy()
        .to_string();
    let cancel = tokio_util::sync::CancellationToken::new();
    let worker_calls = Arc::new(AtomicU32::new(0));
    let (blocking_started, blocking_started_rx) = std::sync::mpsc::sync_channel(1);
    let (blocking_release, blocking_release_rx) = std::sync::mpsc::sync_channel(1);
    let (cache_acknowledged, cache_acknowledged_rx) = std::sync::mpsc::sync_channel(1);
    let (coordinator_completed, coordinator_completed_rx) = std::sync::mpsc::sync_channel(1);
    let (owner_ready, owner_ready_rx) = std::sync::mpsc::sync_channel(1);
    let (finish_writer, finish_writer_rx) = tokio::sync::oneshot::channel();
    let (writer_completed, writer_completed_rx) = std::sync::mpsc::sync_channel(1);
    let (runtime_completed, runtime_completed_rx) = std::sync::mpsc::sync_channel(1);

    let runtime_store_path = store_path.clone();
    let runtime_cancel = cancel.clone();
    let runtime_worker_calls = worker_calls.clone();
    let runtime_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("fallback runtime should build");
        let completed = runtime.block_on(async move {
            tokio::time::timeout(WALL_CLOCK_WATCHDOG * 2, async move {
                let (writer_session, writer_observed) = HydrateCacheWriterSession::start_observed(
                    runtime_store_path,
                    2,
                    runtime_cancel.clone(),
                )
                .expect("fallback writer should start");
                let cache_tx = writer_session.sender().clone();
                let coordinator_cancel = runtime_cancel.clone();
                let analysis_owner = HydrateAnalysisTask::spawn(runtime_cancel, async move {
                    let report = run_bounded_workers(
                        "analysis worker task",
                        vec![
                            (
                                1_u8,
                                Some(blocking_started),
                                Some(blocking_release_rx),
                                Some(cache_acknowledged),
                            ),
                            (2_u8, None, None, None),
                        ],
                        1,
                        coordinator_cancel,
                        |(identity, _, _, _)| *identity,
                        move |(_, started, release, acknowledged)| {
                            let worker_calls = runtime_worker_calls.clone();
                            let cache_tx = cache_tx.clone();
                            async move {
                                worker_calls.fetch_add(1, Ordering::Relaxed);
                                let write = tokio::task::spawn_blocking(move || {
                                    started
                                        .expect("fallback worker should have a start barrier")
                                        .send(())
                                        .expect("fallback worker start should be observed");
                                    release
                                        .expect("fallback worker should have a release")
                                        .recv_timeout(WALL_CLOCK_WATCHDOG)
                                        .expect("fallback worker release should be bounded");
                                    AnalysisCacheWrite {
                                        file_path: "/synthetic/no-runtime-analysis.flac"
                                            .to_string(),
                                        analyzer: "no-runtime-analysis".to_string(),
                                        file_size: 84,
                                        file_mtime: 9,
                                        analyzer_version: "test-v2".to_string(),
                                        input_fingerprint: String::new(),
                                        features_json: "{}".to_string(),
                                    }
                                })
                                .await
                                .expect("fallback blocking analysis should join");
                                send_cache_message(
                                    &cache_tx,
                                    HydrateCacheMessage::AudioAnalysis(write),
                                    "no-runtime analysis",
                                )
                                .await
                                .expect("fallback cache write should be acknowledged");
                                acknowledged
                                    .expect("fallback worker should have an acknowledgement")
                                    .send(())
                                    .expect("fallback acknowledgement should be observed");
                                HydrationWorkerCompletion::completed(
                                    HydrationAnalysisOutcome::default(),
                                )
                            }
                        },
                    )
                    .await;
                    coordinator_completed
                        .send((
                            report.selected,
                            report.scheduled,
                            report.terminal_workers,
                            report.incomplete(),
                            report.join_failures.len(),
                        ))
                        .expect("fallback coordinator report should be observed");
                    report
                });
                owner_ready
                    .send(analysis_owner)
                    .expect("fallback owner should transfer outside the runtime");

                // A closed signal still finishes the writer, keeping panic
                // cleanup bounded if the driving test exits early.
                let _ = finish_writer_rx.await;
                let completion = writer_session.finish().await;
                let observed = writer_observed.await.is_ok();
                writer_completed
                    .send((
                        completion.join_failures,
                        completion.report.attempted,
                        completion.report.succeeded,
                        completion.report.failed,
                        observed,
                    ))
                    .expect("fallback writer report should be observed");
            })
            .await
            .is_ok()
        });
        let _ = runtime_completed.send(completed);
    });

    let analysis_owner = owner_ready_rx
        .recv_timeout(WALL_CLOCK_WATCHDOG)
        .expect("fallback owner setup should meet the wall-clock watchdog");
    blocking_started_rx
        .recv_timeout(WALL_CLOCK_WATCHDOG)
        .expect("fallback blocking worker should start within the watchdog");
    assert_eq!(worker_calls.load(Ordering::Relaxed), 1);

    drop(analysis_owner);
    assert!(
        cancel.is_cancelled(),
        "no-runtime owner drop must synchronously cancel scheduling"
    );
    blocking_release
        .send(())
        .expect("fallback started worker should still accept release");
    cache_acknowledged_rx
        .recv_timeout(WALL_CLOCK_WATCHDOG)
        .expect("fallback cache acknowledgement should meet the watchdog");
    assert_eq!(
        coordinator_completed_rx
            .recv_timeout(WALL_CLOCK_WATCHDOG)
            .expect("fallback coordinator should meet the watchdog"),
        (2, 1, 1, 1, 0)
    );
    assert_eq!(worker_calls.load(Ordering::Relaxed), 1);

    finish_writer
        .send(())
        .expect("fallback runtime should retain its writer until explicit finish");
    assert_eq!(
        writer_completed_rx
            .recv_timeout(WALL_CLOCK_WATCHDOG)
            .expect("fallback writer should close within the watchdog"),
        (0, 1, 1, 0, true)
    );
    assert!(
        runtime_completed_rx
            .recv_timeout(WALL_CLOCK_WATCHDOG)
            .expect("fallback runtime should stop within the watchdog"),
        "fallback runtime scenario exceeded its internal deadline"
    );
    runtime_thread
        .join()
        .expect("fallback runtime thread should join cleanly");

    let connection = state::open(&store_path).expect("fallback writer store should reopen");
    let cached = state::get_audio_analysis(
        &connection,
        "/synthetic/no-runtime-analysis.flac",
        "no-runtime-analysis",
    )
    .expect("fallback analysis cache should read")
    .expect("fallback acknowledged row should be durable");
    assert_eq!(cached.analysis_version, "test-v2");
}

#[tokio::test]
async fn hydrate_token_cancellation_stops_scheduling_and_drains_started_workers() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let discogs_started = Arc::new(tokio::sync::Notify::new());
        let (discogs_dropped, mut discogs_dropped_rx) = tokio::sync::oneshot::channel();
        let (discogs_release, discogs_release_rx) = tokio::sync::oneshot::channel();
        let discogs_cancel = tokio_util::sync::CancellationToken::new();
        let (cache_tx, _cache_rx) =
            tokio::sync::mpsc::channel::<CacheWriteRequest<EnrichmentCacheWrite>>(1);
        let discogs_workers = tokio::spawn({
            let started = discogs_started.clone();
            let cancel = discogs_cancel.clone();
            let first = identity();
            let second = HydrationTrackIdentity::new(
                "unscheduled-track".to_string(),
                "Unscheduled Artist".to_string(),
                "Unscheduled Title".to_string(),
                "Unscheduled Album".to_string(),
            );
            async move {
                run_bounded_workers(
                    "discogs worker task",
                    vec![
                        (first, Some(discogs_release_rx), Some(discogs_dropped)),
                        (second, None, None),
                    ],
                    1,
                    cancel,
                    |(identity, _, _)| identity.clone(),
                    move |(identity, release, dropped)| {
                        let started = started.clone();
                        let cache_tx = cache_tx.clone();
                        async move {
                            let lookup = async move {
                                let _drop_signal = DropSignal(dropped);
                                started.notify_one();
                                release
                                    .expect("scheduled Discogs fixture should have a release")
                                    .await
                                    .expect("Discogs fixture should be released");
                                Err(discogs::LookupError::message(
                                    "released synthetic lookup failure",
                                ))
                            };
                            let outcome = hydrate_discogs_track(
                                &identity,
                                LookupFailurePersistence::DoNotCache,
                                None,
                                &cache_tx,
                                lookup,
                            )
                            .await;
                            HydrationWorkerCompletion::completed(outcome)
                        }
                    },
                )
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), discogs_started.notified())
            .await
            .expect("Discogs lookup-start barrier should be bounded");
        discogs_cancel.cancel();
        assert!(
            !discogs_workers.is_finished(),
            "started Discogs lookup must remain owned during graceful cancellation"
        );
        assert!(
            matches!(
                discogs_dropped_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "Discogs lookup future must not be dropped on token cancellation"
        );
        discogs_release
            .send(())
            .expect("Discogs fixture should still accept release");
        let discogs_report = tokio::time::timeout(Duration::from_secs(1), discogs_workers)
            .await
            .expect("Discogs cancellation join should be bounded")
            .expect("Discogs cancellation task should join");
        assert_eq!(discogs_report.selected, 2);
        assert_eq!(discogs_report.scheduled, 1);
        assert_eq!(discogs_report.terminal_workers, 1);
        assert_eq!(discogs_report.incomplete(), 1);
        assert_eq!(discogs_report.completed[0].1.operation_failures, 1);
        tokio::time::timeout(Duration::from_secs(1), discogs_dropped_rx)
            .await
            .expect("Discogs completion cleanup should be bounded")
            .expect("completed Discogs future should release its resources");

        let analysis_started = Arc::new(tokio::sync::Notify::new());
        let (analysis_dropped, mut analysis_dropped_rx) = tokio::sync::oneshot::channel();
        let (analysis_release, analysis_release_rx) = tokio::sync::oneshot::channel();
        let analysis_cancel = tokio_util::sync::CancellationToken::new();
        let analysis_workers = tokio::spawn({
            let started = analysis_started.clone();
            let cancel = analysis_cancel.clone();
            async move {
                run_bounded_workers(
                    "analysis worker task",
                    vec![
                        (1_u8, Some(analysis_release_rx), Some(analysis_dropped)),
                        (2_u8, None, None),
                    ],
                    1,
                    cancel,
                    |(identity, _, _)| *identity,
                    move |(_, release, dropped)| {
                        let started = started.clone();
                        async move {
                            let _drop_signal = DropSignal(dropped);
                            started.notify_one();
                            release
                                .expect("scheduled analysis fixture should have a release")
                                .await
                                .expect("analysis fixture should be released");
                            HydrationWorkerCompletion::completed(
                                HydrationAnalysisOutcome::default(),
                            )
                        }
                    },
                )
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), analysis_started.notified())
            .await
            .expect("analysis-start barrier should be bounded");
        analysis_cancel.cancel();
        assert!(
            !analysis_workers.is_finished(),
            "started analysis must remain owned until its blocking work drains"
        );
        assert!(
            matches!(
                analysis_dropped_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ),
            "analysis future must not be dropped on cancellation"
        );
        analysis_release
            .send(())
            .expect("analysis fixture should still accept release");
        let analysis_report = tokio::time::timeout(Duration::from_secs(1), analysis_workers)
            .await
            .expect("analysis cancellation join should be bounded")
            .expect("analysis cancellation task should join");
        assert_eq!(analysis_report.selected, 2);
        assert_eq!(analysis_report.scheduled, 1);
        assert_eq!(analysis_report.terminal_workers, 1);
        assert_eq!(analysis_report.incomplete(), 1);
        assert!(analysis_report.completed[0].1.succeeded());
        tokio::time::timeout(Duration::from_secs(1), analysis_dropped_rx)
            .await
            .expect("analysis future drop should be bounded")
            .expect("completed analysis future should release its resources");
    })
    .await
    .expect("injected hydration cancellation matrix should finish within five seconds");
}

fn identity() -> HydrationTrackIdentity {
    HydrationTrackIdentity::new(
        "join-track".to_string(),
        "Join Artist".to_string(),
        "Join Title".to_string(),
        "Join Album".to_string(),
    )
}

#[tokio::test]
async fn hydrate_inner_outer_and_status_panics_retain_join_and_incomplete_accounting() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let workers = run_bounded_workers(
            "discogs worker task",
            vec![identity()],
            1,
            tokio_util::sync::CancellationToken::new(),
            Clone::clone,
            |_| async {
                panic!("injected inner provider panic");
                #[allow(unreachable_code)]
                HydrationWorkerCompletion::completed(EnrichmentTrackOutcome::default())
            },
        )
        .await;
        let inner_report = discogs_stage_report(workers);
        assert_eq!(inner_report.worker_join_failures, 1);
        assert_eq!(inner_report.incomplete(), 1);
        assert_eq!(
            inner_report.error_summaries,
            ["discogs worker task panicked"]
        );

        let counters = ProviderCounters::new();
        counters.record_terminal();
        let outer = HydrateTask::spawn(async {
            panic!("injected outer provider panic");
            #[allow(unreachable_code)]
            HydrationStageReport::default()
        });
        let outer_report = resolve_provider_task_join("discogs", 3, &counters, outer.join().await);
        assert_eq!(outer_report.worker_join_failures, 1);
        assert_eq!(outer_report.terminal_workers, 1);
        assert_eq!(outer_report.incomplete(), 2);
        assert_eq!(
            outer_report.error_summaries,
            ["discogs provider task panicked"]
        );

        let status = HydrateTask::spawn(async {
            panic!("injected status panic");
        });
        let status_error = status
            .join()
            .await
            .expect_err("status panic should fail its task");
        assert_eq!(
            crate::application::batch::task_join_error_summary(
                "hydrate status task",
                &status_error
            ),
            "hydrate status task panicked"
        );
    })
    .await
    .expect("join-failure scenario should finish within five seconds");
}

#[test]
fn hydrate_final_summary_sections_keep_their_human_order() {
    let discogs = HydrationStageReport {
        selected: 3,
        terminal_workers: 3,
        enriched: 1,
        no_match: 1,
        failed: 1,
        ..HydrationStageReport::default()
    };
    let analysis = HydrationStageReport {
        selected: 2,
        terminal_workers: 1,
        enriched: 1,
        failed: 1,
        ..HydrationStageReport::default()
    };
    let writer = CacheWriterReport {
        attempted: 2,
        succeeded: 1,
        failed: 1,
        ..CacheWriterReport::default()
    };
    let lines = final_lines(FinalPresentation {
        elapsed: Duration::from_secs(62),
        want_discogs: true,
        want_analysis: true,
        discogs: &discogs,
        analysis: &analysis,
        writer: &writer,
        writer_join_failures: 1,
        incomplete: 1,
        user_cancelled: true,
    });
    assert_eq!(lines.len(), 7);
    for (line, marker) in lines.iter().zip([
        "Done (1m 2s)",
        "Discogs:",
        "Analysis:",
        "Cache writes:",
        "Cache writer task:",
        "Incomplete:",
        "Cancelled",
    ]) {
        assert!(line.contains(marker), "expected {marker:?} in {line:?}");
    }
}

#[test]
fn hydrate_batch_outcome_preserves_every_application_and_cli_failure_category() {
    let application = HydrationApplicationReport::assemble(
        HydrationStageReport {
            selected: 5,
            terminal_workers: 3,
            operation_failures: 2,
            worker_join_failures: 1,
            error_summaries: vec!["discogs join".to_string()],
            ..HydrationStageReport::default()
        },
        HydrationStageReport {
            selected: 4,
            terminal_workers: 1,
            operation_failures: 3,
            worker_join_failures: 2,
            error_summaries: vec!["analysis join".to_string()],
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
    let failure = hydrate_batch_outcome(&application, 2, true, vec!["status join".to_string()])
        .finish()
        .expect_err("every terminal category should fail hydrate");

    assert_eq!(failure.track_or_provider_failures, 5);
    assert_eq!(failure.worker_join_failures, 6);
    assert_eq!(failure.writer_failures, 4);
    assert_eq!(failure.incomplete, 5);
    assert!(failure.user_cancelled);
    assert_eq!(
        failure.error_summaries,
        [
            "discogs join",
            "analysis join",
            "writer join",
            "status join"
        ]
    );
}
