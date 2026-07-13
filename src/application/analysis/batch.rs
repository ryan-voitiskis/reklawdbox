//! Shared cache-writer and batch failure-accounting policy.

use tokio_util::sync::CancellationToken;

use crate::adapters::state;

use super::model::AnalysisCacheWrite;

/// Abort analysis scheduling after this many consecutive cache write failures.
pub(crate) const MAX_CONSECUTIVE_CACHE_WRITE_FAILURES: u32 = 3;

pub(crate) struct CacheWriteRequest<T> {
    pub payload: T,
    pub acknowledgement: tokio::sync::oneshot::Sender<Result<(), String>>,
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

    fn cache_write(path: String) -> AnalysisCacheWrite {
        AnalysisCacheWrite {
            file_path: path,
            analyzer: audio::ANALYZER_STRATUM.to_string(),
            file_size: 1,
            file_mtime: 2,
            analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
            input_fingerprint: audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            features_json: "{}".to_string(),
        }
    }

    #[tokio::test]
    async fn shared_analysis_batch_acknowledges_cache_writes() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir
            .path()
            .join("cache.sqlite3")
            .to_string_lossy()
            .to_string();
        let cancel = CancellationToken::new();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let writer_path = store_path.clone();
        let writer_cancel = cancel.clone();
        let writer = tokio::task::spawn_blocking(move || {
            run_analysis_cache_writer(writer_path, rx, writer_cancel)
        });

        send_cache_message(&tx, cache_write("/tmp/shared.flac".to_string()), "analysis")
            .await
            .unwrap();
        drop(tx);
        let report = writer.await.unwrap();
        assert_eq!(report.succeeded, 1);
        assert_eq!(report.failed, 0);
    }

    #[test]
    fn shared_analysis_batch_counts_join_and_writer_failures() {
        let failure = batch_outcome("analyze", 0, 2, 3, 4, false, vec![]).unwrap_err();
        assert_eq!(failure.track_or_provider_failures, 0);
        assert_eq!(failure.worker_join_failures, 2);
        assert_eq!(failure.writer_failures, 3);
        assert_eq!(failure.incomplete, 4);
    }
}
