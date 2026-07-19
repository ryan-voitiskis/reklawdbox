//! Audio-analysis cache persistence for the shared writer protocol.

use tokio_util::sync::CancellationToken;

use crate::adapters::state;
use crate::application::cache_writer::{self, CacheWriteRequest, CacheWriterReport};

use super::model::AnalysisCacheWrite;

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
    cache_rx: tokio::sync::mpsc::Receiver<CacheWriteRequest<AnalysisCacheWrite>>,
    cancel: CancellationToken,
) -> CacheWriterReport {
    cache_writer::run(
        store_path,
        cache_rx,
        cancel,
        persist_analysis_cache_write,
        |message| message.analyzer.clone(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::audio;
    use crate::application::batch::BatchOutcome;
    use crate::application::cache_writer::send_cache_message;
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
        let failure = BatchOutcome {
            command: "analyze",
            operation_failures: 0,
            worker_join_failures: 2,
            writer_failures: 3,
            incomplete: 4,
            user_cancelled: false,
            error_summaries: vec!["sentinel failure".to_string()],
        }
        .finish()
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
