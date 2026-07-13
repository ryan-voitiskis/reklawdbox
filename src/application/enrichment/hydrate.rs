//! Shared enrichment hydration cache-write workflow.

use crate::adapters::state;
use crate::application::analysis::batch::{CacheWriteRequest, send_cache_message};
use crate::application::analysis::{job as analysis_job, model::AnalysisCacheWrite};

use super::model::{EnrichmentProvider, HydrationStage};

pub(crate) fn provider_stages(providers: &[EnrichmentProvider]) -> Vec<HydrationStage> {
    providers
        .iter()
        .copied()
        .map(HydrationStage::Lookup)
        .collect()
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

/// Dispatch the `Analysis` hydration stage through the shared Plan 039 job.
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
                    provider: EnrichmentProvider::Beatport,
                    norm_artist: "shared artist".to_string(),
                    norm_title: "shared title".to_string(),
                    norm_album: None,
                    match_quality: Some("none".to_string()),
                    response_json: None,
                },
                "beatport hydration",
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
                "beatport",
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
