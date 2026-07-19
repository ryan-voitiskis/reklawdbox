//! Shared cache-writer and batch failure-accounting policy.

use tokio_util::sync::CancellationToken;

use crate::adapters::state;

use super::model::AnalysisCacheWrite;

/// Abort analysis scheduling after this many consecutive cache write failures.
pub(crate) const MAX_CONSECUTIVE_CACHE_WRITE_FAILURES: u32 = 3;

pub(crate) struct CacheWriteRequest<T, E = String> {
    pub payload: T,
    pub acknowledgement: tokio::sync::oneshot::Sender<Result<(), E>>,
}

#[derive(Debug, Default)]
pub(crate) struct CacheWriterReport {
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub threshold_cancelled: bool,
    pub error_summaries: Vec<String>,
}

impl CacheWriterReport {
    pub(crate) fn record_success(&mut self) {
        self.attempted += 1;
        self.succeeded += 1;
    }

    pub(crate) fn record_failure(&mut self, summary: String) {
        self.attempted += 1;
        self.failed += 1;
        if self.error_summaries.len() < 10 && !self.error_summaries.contains(&summary) {
            self.error_summaries.push(summary);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BatchFailure {
    pub command: &'static str,
    pub track_or_provider_failures: u32,
    pub worker_join_failures: u32,
    pub writer_failures: u32,
    pub incomplete: usize,
    pub user_cancelled: bool,
    pub error_summaries: Vec<String>,
}

impl std::fmt::Display for BatchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} batch failed: {} track/provider failures, {} task join failures, {} cache write failures, {} incomplete",
            self.command,
            self.track_or_provider_failures,
            self.worker_join_failures,
            self.writer_failures,
            self.incomplete,
        )?;
        if self.user_cancelled {
            write!(f, ", cancelled by user")?;
        }
        if !self.error_summaries.is_empty() {
            write!(f, ": {}", self.error_summaries.join("; "))?;
        }
        Ok(())
    }
}

impl std::error::Error for BatchFailure {}

#[allow(clippy::too_many_arguments)]
pub(crate) fn batch_outcome(
    command: &'static str,
    track_or_provider_failures: u32,
    worker_join_failures: u32,
    writer_failures: u32,
    incomplete: usize,
    user_cancelled: bool,
    error_summaries: Vec<String>,
) -> Result<(), BatchFailure> {
    if track_or_provider_failures == 0
        && worker_join_failures == 0
        && writer_failures == 0
        && incomplete == 0
        && !user_cancelled
    {
        Ok(())
    } else {
        Err(BatchFailure {
            command,
            track_or_provider_failures,
            worker_join_failures,
            writer_failures,
            incomplete,
            user_cancelled,
            error_summaries,
        })
    }
}

pub(crate) fn task_join_error_summary(task: &str, error: &tokio::task::JoinError) -> String {
    if error.is_cancelled() {
        format!("{task} was cancelled")
    } else if error.is_panic() {
        format!("{task} panicked")
    } else {
        format!("{task} failed")
    }
}

pub(crate) fn serialize_cache_payload<T: serde::Serialize>(
    value: &T,
    context: &str,
) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("{context} cache serialization failed: {e}"))
}

pub(crate) async fn send_cache_message<T>(
    tx: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
    message: T,
    context: &str,
) -> Result<(), String> {
    let (acknowledgement, result) = tokio::sync::oneshot::channel();
    tx.send(CacheWriteRequest {
        payload: message,
        acknowledgement,
    })
    .await
    .map_err(|e| format!("{context} cache queue send failed: {e}"))?;
    result
        .await
        .map_err(|_| format!("{context} cache acknowledgement canceled"))?
}

pub(crate) fn persist_analysis_cache_write(
    conn: &rusqlite::Connection,
    message: &AnalysisCacheWrite,
) -> Result<(), rusqlite::Error> {
    state::set_audio_analysis_with_fingerprint(
        conn,
        &message.file_path,
        &message.analyzer,
        message.file_size,
        message.file_mtime,
        &message.analyzer_version,
        &message.input_fingerprint,
        &message.features_json,
    )
}

pub(crate) fn run_analysis_cache_writer(
    store_path: String,
    mut cache_rx: tokio::sync::mpsc::Receiver<CacheWriteRequest<AnalysisCacheWrite>>,
    cancel: CancellationToken,
) -> CacheWriterReport {
    let mut report = CacheWriterReport::default();
    let conn = match state::open(&store_path) {
        Ok(conn) => conn,
        Err(error) => {
            let summary = format!("cache store open failed: {error}");
            tracing::error!("Cache writer: {summary} — rejecting queued writes");
            report.error_summaries.push(summary.clone());
            cancel.cancel();
            while let Some(request) = cache_rx.blocking_recv() {
                report.record_failure(summary.clone());
                request.acknowledgement.send(Err(summary.clone())).ok();
            }
            return report;
        }
    };

    let mut consecutive_failures: u32 = 0;
    let mut fatal_error: Option<String> = None;
    while let Some(request) = cache_rx.blocking_recv() {
        if let Some(summary) = &fatal_error {
            report.record_failure(summary.clone());
            request.acknowledgement.send(Err(summary.clone())).ok();
            continue;
        }

        let message = request.payload;
        match persist_analysis_cache_write(&conn, &message) {
            Ok(()) => {
                consecutive_failures = 0;
                report.record_success();
                request.acknowledgement.send(Ok(())).ok();
            }
            Err(error) => {
                consecutive_failures += 1;
                let summary = format!("{} cache write failed: {error}", message.analyzer);
                tracing::error!(
                    "Cache writer: {summary} ({consecutive_failures}/{MAX_CONSECUTIVE_CACHE_WRITE_FAILURES})"
                );
                report.record_failure(summary.clone());
                request.acknowledgement.send(Err(summary)).ok();
                if consecutive_failures >= MAX_CONSECUTIVE_CACHE_WRITE_FAILURES {
                    let fatal = format!(
                        "cache writer stopped after {MAX_CONSECUTIVE_CACHE_WRITE_FAILURES} consecutive failures"
                    );
                    tracing::error!("Cache writer: {fatal} — draining queued writes");
                    report.threshold_cancelled = true;
                    if report.error_summaries.len() < 10 && !report.error_summaries.contains(&fatal)
                    {
                        report.error_summaries.push(fatal.clone());
                    }
                    fatal_error = Some(fatal);
                    cancel.cancel();
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::audio;
    use std::future::Future;
    use std::time::Duration;

    const STEP_TIMEOUT: Duration = Duration::from_secs(1);
    const TEST_WATCHDOG: Duration = Duration::from_secs(5);

    struct TaskGuard<T> {
        handle: Option<tokio::task::JoinHandle<T>>,
    }

    impl<T> TaskGuard<T> {
        fn new(handle: tokio::task::JoinHandle<T>) -> Self {
            Self {
                handle: Some(handle),
            }
        }

        async fn join(mut self, context: &str) -> Result<T, String> {
            let mut handle = self.handle.take().expect("task guard handle");
            match tokio::time::timeout(STEP_TIMEOUT, &mut handle).await {
                Ok(result) => result.map_err(|error| format!("{context} failed: {error}")),
                Err(_) => {
                    handle.abort();
                    tokio::time::timeout(STEP_TIMEOUT, &mut handle)
                        .await
                        .map_err(|_| format!("{context} cleanup timed out"))
                        .map(|_| ())?;
                    Err(format!("{context} timed out"))
                }
            }
        }
    }

    impl<T> Drop for TaskGuard<T> {
        fn drop(&mut self) {
            if let Some(handle) = &self.handle {
                handle.abort();
            }
        }
    }

    async fn bounded<F: Future>(future: F, context: &str) -> Result<F::Output, String> {
        tokio::time::timeout(STEP_TIMEOUT, future)
            .await
            .map_err(|_| format!("{context} timed out"))
    }

    fn cache_write(analyzer: &str, id: u32) -> AnalysisCacheWrite {
        AnalysisCacheWrite {
            file_path: format!("/tmp/shared-{id}.flac"),
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

    fn install_failure_trigger(path: &str) {
        let conn = state::open(path).expect("open store for failure trigger");
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

    #[tokio::test]
    async fn shared_analysis_batch_acknowledges_cache_writes() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let dir = tempfile::tempdir().expect("temporary cache directory");
            let store_path = dir
                .path()
                .join("cache.sqlite3")
                .to_string_lossy()
                .to_string();
            let cancel = CancellationToken::new();
            let writer_cancel = cancel.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let writer_path = store_path.clone();
            let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
                run_analysis_cache_writer(writer_path, rx, writer_cancel)
            }));

            let acknowledgement = bounded(
                send_cache_message(&tx, cache_write("ok", 1), "analysis"),
                "successful cache acknowledgement",
            )
            .await
            .expect("cache acknowledgement wait should be bounded");
            assert_eq!(acknowledgement, Ok(()));
            drop(tx);
            let report = writer
                .join("successful cache writer join")
                .await
                .expect("cache writer should join");
            assert_eq!(report.attempted, 1);
            assert_eq!(report.succeeded, 1);
            assert_eq!(report.failed, 0);
            assert!(!report.threshold_cancelled);
            assert!(!cancel.is_cancelled());

            let conn = state::open(&store_path).expect("reopen cache after acknowledgement");
            assert!(
                state::get_audio_analysis(&conn, "/tmp/shared-1.flac", "ok")
                    .expect("read acknowledged cache entry")
                    .is_some(),
                "success acknowledgement must follow persistence"
            );
        })
        .await
        .expect("shared cache acknowledgement scenario timed out");
    }

    #[tokio::test]
    async fn analysis_cache_writer_threshold_cancels_and_drains() {
        tokio::time::timeout(TEST_WATCHDOG, async {
            let dir = tempfile::tempdir().expect("temporary cache directory");
            let store_path = dir
                .path()
                .join("cache.sqlite3")
                .to_string_lossy()
                .to_string();
            let conn = state::open(&store_path).expect("initialize cache store");
            drop(conn);
            install_failure_trigger(&store_path);

            let cancel = CancellationToken::new();
            let writer_cancel = cancel.clone();
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let mut acknowledgements = Vec::new();
            for id in 0..4 {
                let (acknowledgement, result) = tokio::sync::oneshot::channel();
                bounded(
                    tx.send(CacheWriteRequest {
                        payload: cache_write("fail", id),
                        acknowledgement,
                    }),
                    "threshold queue send",
                )
                .await
                .expect("queue send wait should be bounded")
                .expect("threshold request should queue");
                acknowledgements.push(result);
            }
            drop(tx);

            let writer_path = store_path.clone();
            let writer = TaskGuard::new(tokio::task::spawn_blocking(move || {
                run_analysis_cache_writer(writer_path, rx, writer_cancel)
            }));
            let mut results = Vec::new();
            for acknowledgement in acknowledgements {
                results.push(
                    bounded(acknowledgement, "threshold cache acknowledgement")
                        .await
                        .expect("acknowledgement wait should be bounded")
                        .expect("writer should deliver an acknowledgement"),
                );
            }
            let report = writer
                .join("threshold cache writer join")
                .await
                .expect("threshold writer should join");

            assert!(results.iter().all(Result::is_err));
            assert!(results[..3].iter().all(|result| {
                result
                    .as_ref()
                    .unwrap_err()
                    .contains("injected cache write failure")
            }));
            assert!(
                results[3]
                    .as_ref()
                    .unwrap_err()
                    .contains("cache writer stopped"),
                "the queued request after the threshold must be drained and acknowledged"
            );
            assert_eq!(report.attempted, 4);
            assert_eq!(report.succeeded, 0);
            assert_eq!(report.failed, 4);
            assert!(report.threshold_cancelled);
            assert!(cancel.is_cancelled());
        })
        .await
        .expect("shared cache threshold scenario timed out");
    }

    #[test]
    fn shared_analysis_batch_counts_join_and_writer_failures() {
        let failure = batch_outcome(
            "analyze",
            0,
            2,
            3,
            4,
            false,
            vec!["sentinel failure".to_string()],
        )
        .unwrap_err();
        assert_eq!(failure.command, "analyze");
        assert_eq!(failure.track_or_provider_failures, 0);
        assert_eq!(failure.worker_join_failures, 2);
        assert_eq!(failure.writer_failures, 3);
        assert_eq!(failure.incomplete, 4);
        assert!(!failure.user_cancelled);
        assert_eq!(failure.error_summaries, ["sentinel failure"]);
        assert!(failure.to_string().contains("2 task join failures"));
        assert!(failure.to_string().contains("3 cache write failures"));
    }
}
