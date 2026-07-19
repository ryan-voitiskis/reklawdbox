//! Acknowledged cache-write protocol and bounded writer failure policy.

use tokio_util::sync::CancellationToken;

use crate::adapters::state;

/// Abort batch scheduling after this many consecutive cache write failures.
pub(crate) const MAX_CONSECUTIVE_CACHE_WRITE_FAILURES: u32 = 3;

pub(crate) struct CacheWriteRequest<T, E = String> {
    pub(crate) payload: T,
    pub(crate) acknowledgement: tokio::sync::oneshot::Sender<Result<(), E>>,
}

#[derive(Debug, Default)]
pub(crate) struct CacheWriterReport {
    pub(crate) attempted: u32,
    pub(crate) succeeded: u32,
    pub(crate) failed: u32,
    pub(crate) threshold_cancelled: bool,
    pub(crate) error_summaries: Vec<String>,
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
pub(crate) enum CacheMessageError {
    QueueClosed { context: String },
    AcknowledgementCanceled { context: String },
    WriteRejected(String),
}

impl std::fmt::Display for CacheMessageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QueueClosed { context } => {
                write!(
                    formatter,
                    "{context} cache queue send failed: channel closed"
                )
            }
            Self::AcknowledgementCanceled { context } => {
                write!(formatter, "{context} cache acknowledgement canceled")
            }
            Self::WriteRejected(summary) => formatter.write_str(summary),
        }
    }
}

impl std::error::Error for CacheMessageError {}

pub(crate) fn serialize_cache_payload<T: serde::Serialize>(
    value: &T,
    context: &str,
) -> Result<String, String> {
    serde_json::to_string(value)
        .map_err(|error| format!("{context} cache serialization failed: {error}"))
}

pub(crate) async fn send_cache_message<T>(
    sender: &tokio::sync::mpsc::Sender<CacheWriteRequest<T>>,
    message: T,
    context: &str,
) -> Result<(), CacheMessageError> {
    let (acknowledgement, result) = tokio::sync::oneshot::channel();
    sender
        .send(CacheWriteRequest {
            payload: message,
            acknowledgement,
        })
        .await
        .map_err(|_| CacheMessageError::QueueClosed {
            context: context.to_string(),
        })?;
    result
        .await
        .map_err(|_| CacheMessageError::AcknowledgementCanceled {
            context: context.to_string(),
        })?
        .map_err(CacheMessageError::WriteRejected)
}

pub(crate) fn run<T, Persist, Label>(
    store_path: String,
    mut receiver: tokio::sync::mpsc::Receiver<CacheWriteRequest<T>>,
    cancel: CancellationToken,
    persist: Persist,
    label: Label,
) -> CacheWriterReport
where
    Persist: Fn(&rusqlite::Connection, &T) -> Result<(), rusqlite::Error>,
    Label: Fn(&T) -> String,
{
    let mut report = CacheWriterReport::default();
    let connection = match state::open(&store_path) {
        Ok(connection) => connection,
        Err(error) => {
            let summary = format!("cache store open failed: {error}");
            tracing::error!("Cache writer: {summary} — rejecting queued writes");
            report.error_summaries.push(summary.clone());
            cancel.cancel();
            while let Some(request) = receiver.blocking_recv() {
                report.record_failure(summary.clone());
                request.acknowledgement.send(Err(summary.clone())).ok();
            }
            return report;
        }
    };

    let mut consecutive_failures: u32 = 0;
    let mut fatal_error: Option<String> = None;
    while let Some(request) = receiver.blocking_recv() {
        if let Some(summary) = &fatal_error {
            report.record_failure(summary.clone());
            request.acknowledgement.send(Err(summary.clone())).ok();
            continue;
        }

        let message = request.payload;
        match persist(&connection, &message) {
            Ok(()) => {
                consecutive_failures = 0;
                report.record_success();
                request.acknowledgement.send(Ok(())).ok();
            }
            Err(error) => {
                consecutive_failures += 1;
                let summary = format!("{} cache write failed: {error}", label(&message));
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
    use std::time::Duration;

    #[tokio::test]
    async fn typed_send_errors_preserve_existing_messages() {
        tokio::time::timeout(Duration::from_secs(5), async {
            let (sender, receiver) = tokio::sync::mpsc::channel::<CacheWriteRequest<()>>(1);
            drop(receiver);
            let error = send_cache_message(&sender, (), "analysis")
                .await
                .expect_err("closed queue should fail");

            assert_eq!(
                error,
                CacheMessageError::QueueClosed {
                    context: "analysis".to_string()
                }
            );
            assert_eq!(
                error.to_string(),
                "analysis cache queue send failed: channel closed"
            );
        })
        .await
        .expect("closed queue scenario timed out");
    }
}
