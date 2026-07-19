use std::sync::Arc;
use std::time::Instant;

use console::style;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio_util::sync::CancellationToken;

use crate::adapters::audio as audio_adapter;
#[cfg(test)]
use crate::adapters::audio;
use crate::adapters::{rekordbox as db, state as store};
use crate::application::analysis::batch::run_analysis_cache_writer;
use crate::application::analysis::identity::{cache_probe_for_path, cache_status_for_track};
use crate::application::analysis::model::AnalysisCacheWrite as CliCacheWriteMsg;
use crate::application::batch::{BatchOutcome, task_join_error_summary};
use crate::application::cache_writer::{CacheWriteRequest, CacheWriterReport, send_cache_message};

use super::runtime::resources::{
    CpuPreset, analysis_concurrency_for_preset, apply_cpu_niceness, cpu_preset_summary,
    memory_budget_mb, memory_preset_summary, track_memory_cost_mb,
};
use super::runtime::signals::{CliCancellationState, spawn_signal_handlers};

#[derive(clap::Args)]
pub(crate) struct AnalyzeArgs {
    /// Filter by playlist ID
    #[arg(long)]
    playlist: Option<String>,
    /// Filter by artist name (partial match)
    #[arg(long)]
    artist: Option<String>,
    /// Filter by genre name (partial match)
    #[arg(long)]
    genre: Option<String>,
    /// Minimum BPM
    #[arg(long)]
    bpm_min: Option<f64>,
    /// Maximum BPM
    #[arg(long)]
    bpm_max: Option<f64>,
    /// Filter by musical key
    #[arg(long)]
    key: Option<String>,
    /// Filter by label name (partial match)
    #[arg(long)]
    label: Option<String>,
    /// Filter by file path/folder (partial match)
    #[arg(long)]
    path: Option<String>,
    /// Search query matching title or artist
    #[arg(long)]
    query: Option<String>,
    /// Only tracks added on or after this date (ISO date)
    #[arg(long)]
    added_after: Option<String>,
    /// Only tracks added on or before this date (ISO date)
    #[arg(long)]
    added_before: Option<String>,
    /// Minimum star rating (1-5)
    #[arg(long)]
    rating_min: Option<u8>,
    /// Max tracks to process
    #[arg(long, default_value = "200")]
    max_tracks: u32,
    /// Don't skip already-cached tracks
    #[arg(long)]
    no_skip_cached: bool,
    /// Skip Essentia analysis, only run stratum-dsp
    #[arg(long)]
    stratum_only: bool,
    /// CPU scheduling preset
    #[arg(long, value_enum, default_value_t = CpuPreset::Background)]
    cpu: CpuPreset,
    /// Override analysis concurrency (overrides --cpu preset, min 1, max 16)
    #[arg(long, short = 'j')]
    concurrency: Option<u32>,
}

fn record_analyze_worker_join(
    result: Result<(), tokio::task::JoinError>,
    error_summaries: &mut Vec<String>,
) -> u32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            let summary = task_join_error_summary("analysis worker task", &error);
            tracing::error!("{summary}: {error}");
            error_summaries.push(summary);
            1
        }
    }
}

pub(crate) async fn run_analyze(args: AnalyzeArgs) -> Result<(), Box<dyn std::error::Error>> {
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

    let essentia_python = if args.stratum_only {
        None
    } else {
        audio_adapter::probe_essentia_python_path()
    };

    tracing::info!(
        "Essentia: {}",
        if args.stratum_only {
            "skipped (--stratum-only)".to_string()
        } else {
            match &essentia_python {
                Some(p) => format!("available ({p})"),
                None => "not found (stratum-dsp only)".to_string(),
            }
        }
    );

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
        limit: Some(args.max_tracks),
        offset: None,
    };
    let tracks = db::search_tracks_unbounded(&conn, &params)?;

    if tracks.is_empty() {
        tracing::info!("No tracks match the given filters.");
        return Ok(());
    }

    let skip_cached = !args.no_skip_cached;
    let mut to_analyze = Vec::new();
    let mut cached_count = 0;

    for track in &tracks {
        let cache_probe = cache_probe_for_path(&track.file_path, skip_cached);
        let (has_stratum, has_essentia) = cache_status_for_track(
            &store_conn,
            cache_probe.as_ref(),
            skip_cached,
            essentia_python.is_some(),
        )?;

        if has_stratum && has_essentia {
            cached_count += 1;
        } else {
            to_analyze.push((track.clone(), !has_stratum, !has_essentia));
        }
    }

    let total = tracks.len();

    // LPT scheduling: longest tracks first so short tracks fill gaps at the tail
    to_analyze.sort_by_key(|b| std::cmp::Reverse(b.0.length));

    let pending = to_analyze.len();

    let cpu_preset = args.cpu;
    apply_cpu_niceness(cpu_preset);
    let concurrency = match args.concurrency {
        Some(n) => n.clamp(1, 16) as usize,
        None => analysis_concurrency_for_preset(cpu_preset),
    };
    let analysis_budget_mb = memory_budget_mb(cpu_preset);

    tracing::info!("{}", cpu_preset_summary(cpu_preset, concurrency));
    tracing::info!("{}", memory_preset_summary(analysis_budget_mb));
    tracing::info!(
        "Scanning {total} tracks ({cached_count} cached, {pending} to analyze, concurrency={concurrency})"
    );

    if to_analyze.is_empty() {
        tracing::info!("All tracks already cached. Nothing to do.");
        return Ok(());
    }

    let mp = MultiProgress::new();
    let pb = mp.add(ProgressBar::new(pending as u64));
    pb.set_style(
        ProgressStyle::with_template(
            "Analyzing [{bar:40.cyan/blue}] {pos}/{len}  {percent}%  ETA {eta}",
        )
        .unwrap()
        .progress_chars("##-"),
    );

    // Writer task opens its own connection
    drop(store_conn);

    let cancel = CancellationToken::new();
    let cancellation_state = CliCancellationState::default();

    spawn_signal_handlers(&mp, &cancel, &cancellation_state);

    let batch_start = Instant::now();
    let analyzed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let failed = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let completed_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let (cache_tx, cache_rx) =
        tokio::sync::mpsc::channel::<CacheWriteRequest<CliCacheWriteMsg>>(concurrency * 4);
    let writer_store_path = store_path_str.clone();
    let writer_cancel = cancel.clone();
    let writer_handle = tokio::task::spawn_blocking(move || {
        run_analysis_cache_writer(writer_store_path, cache_rx, writer_cancel)
    });

    // Dual semaphore: CPU concurrency + memory budget
    let cpu_sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mem_sem = Arc::new(tokio::sync::Semaphore::new(analysis_budget_mb as usize));
    let mut handles = Vec::with_capacity(pending);

    for (track, needs_stratum, needs_essentia) in to_analyze {
        if cancel.is_cancelled() {
            break;
        }
        // Memory cost clamped to budget so a single huge track can still run (solo)
        let cost_mb = track_memory_cost_mb(track.length).min(analysis_budget_mb);
        let cpu_permit = tokio::select! {
            result = cpu_sem.clone().acquire_owned() => match result {
                Ok(p) => p,
                Err(_) => break,
            },
            _ = cancel.cancelled() => break,
        };
        let mem_permit = tokio::select! {
            result = mem_sem.clone().acquire_many_owned(cost_mb) => match result {
                Ok(p) => p,
                Err(_) => break,
            },
            _ = cancel.cancelled() => {
                drop(cpu_permit);
                break;
            },
        };
        let label = format!("{} - {}", track.artist, track.title);
        let essentia_python = essentia_python.clone();
        let cache_tx = cache_tx.clone();
        let analyzed = analyzed.clone();
        let failed = failed.clone();
        let completed_count = completed_count.clone();
        let mp = mp.clone();
        let pb = pb.clone();
        let cancel = cancel.clone();

        handles.push(tokio::spawn(async move {
            if cancel.is_cancelled() {
                drop(cpu_permit);
                drop(mem_permit);
                return;
            }

            let result = cli_analyze_single_track(
                &track.file_path,
                needs_stratum,
                needs_essentia,
                essentia_python.as_deref(),
                &cache_tx,
            )
            .await;

            let idx = completed_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

            match result {
                Ok(outcome) => {
                    let elapsed = outcome.elapsed;
                    match outcome.kind {
                        CliTrackOutcome::StratumAndEssentia {
                            bpm,
                            key_camelot,
                            essentia_ok,
                        } => {
                            let essentia_status = if essentia_ok {
                                " +essentia"
                            } else {
                                " (essentia failed)"
                            };
                            mp.println(format!(
                                "[{idx}/{pending}] {label} ... BPM={bpm:.1} Key={key_camelot}{essentia_status} ({elapsed:.1}s)"
                            )).ok();
                            if essentia_ok {
                                analyzed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                        CliTrackOutcome::StratumOnly { bpm, key_camelot } => {
                            mp.println(format!(
                                "[{idx}/{pending}] {label} ... BPM={bpm:.1} Key={key_camelot} ({elapsed:.1}s)"
                            )).ok();
                            analyzed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        CliTrackOutcome::EssentiaOnly { ok } => {
                            if ok {
                                mp.println(format!(
                                    "[{idx}/{pending}] {label} ... +essentia ({elapsed:.1}s)"
                                )).ok();
                                analyzed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            } else {
                                mp.println(format!(
                                    "[{idx}/{pending}] {} {label}: Essentia error ({elapsed:.1}s)",
                                    style("FAIL").red()
                                )).ok();
                                failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
                Err(error) => {
                    mp.println(format!(
                        "[{idx}/{pending}] {} {label}: {error}",
                        style("SKIP").yellow()
                    )).ok();
                    if matches!(error, CliTrackFailure::Processing(_)) {
                        failed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }

            pb.inc(1);
            drop(cpu_permit);
            drop(mem_permit);
        }));
    }

    let mut worker_join_failures = 0_u32;
    let mut error_summaries = Vec::new();
    for handle in handles {
        worker_join_failures += record_analyze_worker_join(handle.await, &mut error_summaries);
    }

    let user_cancelled = cancellation_state.user_requested();
    cancel.cancel();
    pb.finish_and_clear();

    drop(cache_tx);
    let (writer_report, writer_join_failures) = match writer_handle.await {
        Ok(report) => (report, 0_u32),
        Err(error) => {
            let summary = task_join_error_summary("cache writer task", &error);
            tracing::error!("{summary}: {error}");
            error_summaries.push(summary);
            (CacheWriterReport::default(), 1_u32)
        }
    };
    error_summaries.extend(writer_report.error_summaries.iter().cloned());

    let analyzed = analyzed.load(std::sync::atomic::Ordering::Relaxed);
    let track_failures = failed.load(std::sync::atomic::Ordering::Relaxed);
    let failed = track_failures + worker_join_failures;
    let terminal = completed_count.load(std::sync::atomic::Ordering::Relaxed);
    let incomplete = pending.saturating_sub(terminal);
    let total_time = batch_start.elapsed();
    let mins = total_time.as_secs() / 60;
    let secs = total_time.as_secs() % 60;
    println!();
    if failed == 0 {
        println!(
            "Done: {} analyzed ({mins}m {secs}s)",
            style(analyzed).green()
        );
    } else {
        println!(
            "Done: {} analyzed, {} failed ({mins}m {secs}s)",
            style(analyzed).green(),
            style(failed).red()
        );
    }

    if writer_report.attempted > 0 || writer_report.failed > 0 {
        println!(
            "Cache writes: {} succeeded, {} failed",
            style(writer_report.succeeded).green(),
            if writer_report.failed > 0 {
                style(writer_report.failed).red()
            } else {
                style(writer_report.failed).dim()
            }
        );
    }
    if writer_join_failures > 0 {
        println!("Cache writer task: {}", style("failed").red());
    }
    if incomplete > 0 {
        println!("Incomplete: {} selected tracks", style(incomplete).red());
    }
    if user_cancelled {
        println!("Cancelled");
    }

    BatchOutcome {
        command: "analyze",
        operation_failures: track_failures,
        worker_join_failures: worker_join_failures + writer_join_failures,
        writer_failures: writer_report.failed,
        incomplete,
        user_cancelled,
        error_summaries,
    }
    .finish()
    .map_err(|error| Box::new(error) as Box<dyn std::error::Error>)
}

enum CliTrackOutcome {
    StratumAndEssentia {
        bpm: f64,
        key_camelot: String,
        essentia_ok: bool,
    },
    StratumOnly {
        bpm: f64,
        key_camelot: String,
    },
    EssentiaOnly {
        ok: bool,
    },
}

enum CliTrackFailure {
    Processing(String),
    // Reported once by CacheWriterReport instead of double-counted as a track failure.
    CacheWrite(String),
}

impl std::fmt::Display for CliTrackFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Processing(message) | Self::CacheWrite(message) => f.write_str(message),
        }
    }
}

impl From<String> for CliTrackFailure {
    fn from(message: String) -> Self {
        Self::Processing(message)
    }
}

struct CliTrackResult {
    kind: CliTrackOutcome,
    elapsed: f64,
}

async fn cli_analyze_single_track(
    raw_file_path: &str,
    needs_stratum: bool,
    needs_essentia: bool,
    essentia_python: Option<&str>,
    cache_tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<CliCacheWriteMsg>>,
) -> Result<CliTrackResult, CliTrackFailure> {
    let report = crate::application::analysis::job::run(
        raw_file_path,
        needs_stratum,
        needs_essentia,
        essentia_python,
        false,
    )
    .await?;
    for message in report.cache_messages {
        let analyzer = message.analyzer.clone();
        send_cache_message(cache_tx, message, &format!("{analyzer} analysis"))
            .await
            .map_err(|error| CliTrackFailure::CacheWrite(error.to_string()))?;
    }

    match report.stratum {
        Some(Ok(stratum)) => {
            if report.essentia.is_some() {
                let essentia_ok = report.essentia.is_some_and(|result| {
                    if let Err(error) = &result {
                        tracing::error!("{error}");
                    }
                    result.is_ok()
                });
                return Ok(CliTrackResult {
                    kind: CliTrackOutcome::StratumAndEssentia {
                        bpm: stratum.bpm,
                        key_camelot: stratum.key_camelot,
                        essentia_ok,
                    },
                    elapsed: report.elapsed_seconds,
                });
            }
            Ok(CliTrackResult {
                kind: CliTrackOutcome::StratumOnly {
                    bpm: stratum.bpm,
                    key_camelot: stratum.key_camelot,
                },
                elapsed: report.elapsed_seconds,
            })
        }
        Some(Err(error)) => Err(CliTrackFailure::Processing(error)),
        None => match report.essentia {
            Some(result) => {
                let ok = result.is_ok();
                if let Err(error) = result {
                    tracing::error!("{error}");
                }
                Ok(CliTrackResult {
                    kind: CliTrackOutcome::EssentiaOnly { ok },
                    elapsed: report.elapsed_seconds,
                })
            }
            None => Err("Essentia not available".to_string().into()),
        },
    }
}

#[cfg(test)]
pub(super) fn handle_decode_result(
    decode_result: Result<Result<(Vec<f32>, u32), audio::AudioError>, tokio::task::JoinError>,
    track_index: usize,
    pending: usize,
    label: &str,
    failed: &mut u32,
) -> Option<(Vec<f32>, u32)> {
    match decode_result {
        Ok(Ok(value)) => Some(value),
        Ok(Err(e)) => {
            tracing::error!("[{track_index}/{pending}] FAIL {label}: Decode error: {e}");
            *failed += 1;
            None
        }
        Err(e) => {
            tracing::error!("[{track_index}/{pending}] FAIL {label}: Decode task failed: {e}");
            *failed += 1;
            None
        }
    }
}

#[cfg(test)]
pub(super) fn handle_analysis_result(
    analysis_result: Result<
        Result<audio::StratumResult, audio::AudioError>,
        tokio::task::JoinError,
    >,
    idx: usize,
    pending: usize,
    label: &str,
    failed: &mut u32,
) -> Option<audio::StratumResult> {
    match analysis_result {
        Ok(Ok(result)) => Some(result),
        Ok(Err(e)) => {
            tracing::error!("[{idx}/{pending}] FAIL {label}: Analysis error: {e}");
            *failed += 1;
            None
        }
        Err(e) => {
            tracing::error!("[{idx}/{pending}] FAIL {label}: Analysis task failed: {e}");
            *failed += 1;
            None
        }
    }
}

#[cfg(test)]
pub(super) fn mark_track_outcome(analyzed: &mut u32, failed: &mut u32, success: bool) {
    if success {
        *analyzed += 1;
    } else {
        *failed += 1;
    }
}

#[cfg(test)]
mod batch_tests {
    use super::*;
    use crate::cli::runtime::test_support::{TEST_WATCHDOG, TaskGuard, bounded};

    fn cache_message(analyzer: &str, id: u32) -> CliCacheWriteMsg {
        CliCacheWriteMsg {
            file_path: format!("/tmp/track-{id}.flac"),
            analyzer: analyzer.to_string(),
            file_size: i64::from(id) + 1,
            file_mtime: i64::from(id) + 2,
            analyzer_version: "test-v1".to_string(),
            input_fingerprint: if analyzer == audio::ANALYZER_STRATUM {
                audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string()
            } else {
                String::new()
            },
            features_json: "{}".to_string(),
        }
    }

    fn temp_store() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("cache.sqlite3");
        let path = path.to_string_lossy().to_string();
        let conn = store::open(&path).expect("create store");
        drop(conn);
        (dir, path)
    }

    fn install_failure_trigger(path: &str) {
        let conn = store::open(path).expect("open store for trigger");
        conn.execute_batch(
            "CREATE TRIGGER reject_failed_analysis
             BEFORE INSERT ON audio_analysis_cache
             WHEN NEW.analyzer = 'fail'
             BEGIN
               SELECT RAISE(FAIL, 'injected cache write failure');
             END;",
        )
        .expect("install failure trigger");
    }

    async fn run_writer_requests(
        path: String,
        messages: Vec<CliCacheWriteMsg>,
    ) -> (
        Vec<Result<(), String>>,
        CacheWriterReport,
        CancellationToken,
    ) {
        let cancel = CancellationToken::new();
        let writer_cancel = cancel.clone();
        let (tx, rx) = tokio::sync::mpsc::channel(messages.len().max(1));
        let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
            run_analysis_cache_writer(path, rx, writer_cancel)
        }));
        let mut results = Vec::with_capacity(messages.len());
        for message in messages {
            match bounded(
                send_cache_message(&tx, message, "analysis test"),
                "analysis writer acknowledgement",
            )
            .await
            {
                Ok(result) => results.push(result.map_err(|error| error.to_string())),
                Err(error) => {
                    results.push(Err(error));
                    break;
                }
            }
        }
        drop(tx);
        let report = writer
            .join("analysis writer join")
            .await
            .expect("guarded writer join");
        (results, report, cancel)
    }

    #[tokio::test]
    async fn analyze_cache_writer_acknowledges_durable_success() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            let (results, report, cancel) =
                run_writer_requests(path.clone(), vec![cache_message("ok", 1)]).await;
            assert_eq!(results, vec![Ok(())]);
            assert_eq!(report.attempted, 1);
            assert_eq!(report.succeeded, 1);
            assert_eq!(report.failed, 0);
            assert!(!cancel.is_cancelled());

            let conn = store::open(&path).expect("reopen store");
            assert!(
                store::get_audio_analysis(&conn, "/tmp/track-1.flac", "ok")
                    .expect("read cache")
                    .is_some(),
                "success acknowledgement must follow durable persistence"
            );
            cancel.cancel();
        })
        .await
        .expect("writer success test watchdog expired");
    }

    #[tokio::test]
    async fn analyze_cache_writer_reports_recoverable_failure() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            install_failure_trigger(&path);
            let (results, report, cancel) =
                run_writer_requests(path, vec![cache_message("fail", 1), cache_message("ok", 2)])
                    .await;
            assert!(results[0].is_err());
            assert_eq!(results[1], Ok(()));
            assert_eq!(report.attempted, 2);
            assert_eq!(report.succeeded, 1);
            assert_eq!(report.failed, 1);
            assert!(!report.threshold_cancelled);
            assert!(!cancel.is_cancelled());
            cancel.cancel();
        })
        .await
        .expect("recoverable writer test watchdog expired");
    }

    #[tokio::test]
    async fn analyze_cache_writer_threshold_cancels_and_drains() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            install_failure_trigger(&path);
            let messages = (0..4)
                .map(|id| cache_message("fail", id))
                .collect::<Vec<_>>();
            let (results, report, cancel) = run_writer_requests(path, messages).await;
            assert!(results.iter().all(Result::is_err));
            assert_eq!(report.attempted, 4);
            assert_eq!(report.failed, 4);
            assert!(report.threshold_cancelled);
            assert!(cancel.is_cancelled());
            assert!(
                results[3]
                    .as_ref()
                    .expect_err("drained write should be rejected")
                    .contains("cache writer stopped")
            );
        })
        .await
        .expect("threshold writer test watchdog expired");
    }

    #[tokio::test]
    async fn analyze_cache_writer_open_failure_rejects_and_drains() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let dir = tempfile::tempdir().expect("temp dir");
            let invalid_path = dir.path().to_string_lossy().to_string();
            let (results, report, cancel) =
                run_writer_requests(invalid_path, vec![cache_message("ok", 1)]).await;
            assert!(
                results[0]
                    .as_ref()
                    .expect_err("open failure")
                    .contains("cache store open failed")
            );
            assert_eq!(report.attempted, 1);
            assert_eq!(report.failed, 1);
            assert!(cancel.is_cancelled());
        })
        .await
        .expect("open-failure writer test watchdog expired");
    }

    #[tokio::test]
    async fn analyze_cache_writer_tolerates_dropped_ack_receiver() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let (_dir, path) = temp_store();
            let cancel = CancellationToken::new();
            let writer_cancel = cancel.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
                run_analysis_cache_writer(path, rx, writer_cancel)
            }));
            let (acknowledgement, result) = tokio::sync::oneshot::channel();
            drop(result);
            let send_result = bounded(
                tx.send(CacheWriteRequest {
                    payload: cache_message("ok", 1),
                    acknowledgement,
                }),
                "dropped-ack queue send",
            )
            .await;
            drop(tx);
            let report = writer.join("dropped-ack writer join").await;
            send_result
                .expect("bounded queue send")
                .expect("queue request");
            let report = report.expect("guarded writer join");
            assert_eq!(report.succeeded, 1);
            assert!(!cancel.is_cancelled());
            cancel.cancel();
        })
        .await
        .expect("dropped-ack writer test watchdog expired");
    }

    #[tokio::test]
    async fn analyze_worker_join_error_is_counted() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let handle = TaskGuard::new(tokio::spawn(async { panic!("injected worker panic") }));
            let join_result = handle
                .join_raw("injected analysis worker join")
                .await
                .expect("bounded worker join");
            let mut errors = Vec::new();
            assert_eq!(record_analyze_worker_join(join_result, &mut errors), 1);
            assert_eq!(errors.len(), 1);
            assert_eq!(errors[0], "analysis worker task panicked");
        })
        .await
        .expect("worker-join test watchdog expired");
    }
}
#[cfg(test)]
mod task_tests {
    use super::{handle_analysis_result, handle_decode_result, mark_track_outcome};
    use crate::adapters::audio::{AudioError, StratumResult};
    use crate::cli::runtime::test_support::{TEST_WATCHDOG, TaskGuard};
    use std::time::Duration;

    fn sample_stratum_result() -> StratumResult {
        StratumResult {
            bpm: 120.0,
            bpm_confidence: 0.9,
            key: "Am".to_string(),
            key_camelot: "8A".to_string(),
            key_confidence: 0.8,
            key_clarity: 0.7,
            grid_stability: 0.95,
            grid_source: "hmm".to_string(),
            duration_seconds: 180.0,
            processing_time_ms: 42.0,
            analyzer_version: "1.0.0".to_string(),
            mod_centroid: Some(12.5),
            harmonic_proportion: Some(0.65),
            decay_mid_tau: Some(180.0),
            decay_mid_r2: Some(0.92),
            decay_high_tau: Some(95.0),
            decay_high_r2: Some(0.88),
            dub_stab_onset_count: None,
            dub_stab_onset_rate: None,
            dub_stab_rate_basis: None,
            dub_stab_histogram: None,
            dub_stab_template: None,
            dub_stab_template_score: None,
            kick_pattern: None,
            kick_pattern_confidence: None,
            kick_kicks_per_bar: None,
            kick_onset_count: None,
            kick_rate_basis: None,
            kick_histogram: None,
            sections: None,
            flags: vec![],
            warnings: vec![],
        }
    }

    #[tokio::test]
    async fn decode_join_error_marks_failed_and_allows_next_track() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let handle = TaskGuard::new(tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok::<(Vec<f32>, u32), AudioError>((vec![0.0], 44_100))
            }));
            handle.abort();
            let join_err = handle
                .join_raw("aborted decode task join")
                .await
                .expect("bounded decode join")
                .expect_err("aborted task should produce JoinError");

            let mut failed = 0;
            assert!(handle_decode_result(Err(join_err), 1, 2, "a - b", &mut failed).is_none());
            assert_eq!(failed, 1);

            let next =
                handle_decode_result(Ok(Ok((vec![0.0], 44_100))), 2, 2, "c - d", &mut failed);
            assert!(
                next.is_some(),
                "next track should continue after prior join error"
            );
            assert_eq!(failed, 1);
        })
        .await
        .expect("decode-join test watchdog expired");
    }

    #[tokio::test]
    async fn analysis_join_error_marks_failed_and_allows_next_track() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let handle = TaskGuard::new(tokio::spawn(async {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok::<StratumResult, AudioError>(sample_stratum_result())
            }));
            handle.abort();
            let join_err = handle
                .join_raw("aborted analysis task join")
                .await
                .expect("bounded analysis join")
                .expect_err("aborted task should produce JoinError");

            let mut failed = 0;
            assert!(handle_analysis_result(Err(join_err), 1, 2, "a - b", &mut failed).is_none());
            assert_eq!(failed, 1);

            let next =
                handle_analysis_result(Ok(Ok(sample_stratum_result())), 2, 2, "c - d", &mut failed);
            assert!(
                next.is_some(),
                "next track should continue after prior analysis join error"
            );
            assert_eq!(failed, 1);
        })
        .await
        .expect("analysis-join test watchdog expired");
    }

    #[test]
    fn mark_track_outcome_counts_success_and_failure_consistently() {
        let mut analyzed = 0;
        let mut failed = 0;

        mark_track_outcome(&mut analyzed, &mut failed, true);
        mark_track_outcome(&mut analyzed, &mut failed, false);

        assert_eq!(analyzed, 1);
        assert_eq!(failed, 1);
    }
}
