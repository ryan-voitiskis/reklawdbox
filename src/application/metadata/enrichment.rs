//! Acknowledged provider enrichment for metadata backfills.

use std::future::Future;

use schemars::JsonSchema;
use serde::Serialize;

use crate::adapters::state;
use crate::application::cache_writer::CacheWriteRequest;

const PROVIDER_CONCURRENCY: usize = 4;
const CACHE_QUEUE_CAPACITY: usize = 32;
const FAILURE_DETAIL_LIMIT: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MetadataEnrichmentProvider {
    Bandcamp,
    MusicBrainz,
}

impl MetadataEnrichmentProvider {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Bandcamp => "bandcamp",
            Self::MusicBrainz => "musicbrainz",
        }
    }
}

impl std::fmt::Display for MetadataEnrichmentProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataEnrichmentRequest {
    pub(crate) norm_artist: String,
    pub(crate) norm_title: String,
    pub(crate) raw_artist: String,
    pub(crate) raw_title: String,
}

impl MetadataEnrichmentRequest {
    pub(crate) fn new(
        norm_artist: String,
        norm_title: String,
        raw_artist: String,
        raw_title: String,
    ) -> Self {
        Self {
            norm_artist,
            norm_title,
            raw_artist,
            raw_title,
        }
    }
}

impl From<(String, String, String, String)> for MetadataEnrichmentRequest {
    fn from(value: (String, String, String, String)) -> Self {
        Self::new(value.0, value.1, value.2, value.3)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MetadataCacheWrite {
    pub(crate) provider: MetadataEnrichmentProvider,
    pub(crate) norm_artist: String,
    pub(crate) norm_title: String,
    pub(crate) match_quality: String,
    pub(crate) response_json: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MetadataCacheWriteError {
    WriterOpenFailed,
    CacheWriteFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetadataCacheRequestError {
    QueueSendFailed,
    AcknowledgementCanceled,
    WriterOpenFailed,
    CacheWriteFailed,
}

impl From<MetadataCacheWriteError> for MetadataCacheRequestError {
    fn from(error: MetadataCacheWriteError) -> Self {
        match error {
            MetadataCacheWriteError::WriterOpenFailed => Self::WriterOpenFailed,
            MetadataCacheWriteError::CacheWriteFailed => Self::CacheWriteFailed,
        }
    }
}

impl MetadataCacheRequestError {
    const fn failure(self) -> (MetadataEnrichmentFailureKind, &'static str) {
        match self {
            Self::QueueSendFailed => (
                MetadataEnrichmentFailureKind::QueueSendFailed,
                "metadata cache queue send failed",
            ),
            Self::AcknowledgementCanceled => (
                MetadataEnrichmentFailureKind::AcknowledgementCanceled,
                "metadata cache acknowledgement canceled",
            ),
            Self::WriterOpenFailed => (
                MetadataEnrichmentFailureKind::WriterOpenFailed,
                "metadata cache writer open failed",
            ),
            Self::CacheWriteFailed => (
                MetadataEnrichmentFailureKind::CacheWriteFailed,
                "metadata cache write failed",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetadataEnrichmentFailureKind {
    LookupFailed,
    SerializationFailed,
    QueueSendFailed,
    AcknowledgementCanceled,
    CacheWriteFailed,
    WriterOpenFailed,
    WorkerFailed,
    WriterTaskFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, JsonSchema)]
pub(crate) struct MetadataEnrichmentFailure {
    pub(crate) provider: MetadataEnrichmentProvider,
    pub(crate) normalized_artist: String,
    pub(crate) normalized_title: String,
    pub(crate) kind: MetadataEnrichmentFailureKind,
    pub(crate) summary: String,
}

impl MetadataEnrichmentFailure {
    fn new(
        provider: MetadataEnrichmentProvider,
        request: &MetadataEnrichmentRequest,
        kind: MetadataEnrichmentFailureKind,
        summary: &str,
    ) -> Self {
        Self {
            provider,
            normalized_artist: bounded_identity(&request.norm_artist),
            normalized_title: bounded_identity(&request.norm_title),
            kind,
            summary: sanitized_summary(summary),
        }
    }
}

fn bounded_identity(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(160)
        .collect()
}

fn sanitized_summary(value: &str) -> String {
    let collapsed = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let lowercase = collapsed.to_ascii_lowercase();
    if [
        "authorization",
        "bearer ",
        "token=",
        "api_key=",
        "apikey=",
        "secret=",
    ]
    .iter()
    .any(|marker| lowercase.contains(marker))
    {
        return "failure details redacted".to_string();
    }
    collapsed.chars().take(240).collect()
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, JsonSchema)]
pub(crate) struct MetadataProviderEnrichmentReport {
    pub(crate) operation_failed: bool,
    pub(crate) requested: usize,
    pub(crate) matched: usize,
    pub(crate) no_match: usize,
    pub(crate) lookup_failed: usize,
    pub(crate) cache_writes_succeeded: usize,
    pub(crate) cache_writes_failed: usize,
    pub(crate) serialization_failed: usize,
    pub(crate) worker_failed: usize,
}

impl MetadataProviderEnrichmentReport {
    fn absorb(&mut self, other: Self) {
        self.requested += other.requested;
        self.matched += other.matched;
        self.no_match += other.no_match;
        self.lookup_failed += other.lookup_failed;
        self.cache_writes_succeeded += other.cache_writes_succeeded;
        self.cache_writes_failed += other.cache_writes_failed;
        self.serialization_failed += other.serialization_failed;
        self.worker_failed += other.worker_failed;
        self.refresh_operation_failed();
    }

    fn refresh_operation_failed(&mut self) {
        self.operation_failed = self.lookup_failed > 0
            || self.cache_writes_failed > 0
            || self.serialization_failed > 0
            || self.worker_failed > 0;
    }

    fn validate(&self, context: &str) -> Result<(), String> {
        if self.requested != self.matched + self.no_match + self.lookup_failed + self.worker_failed
        {
            return Err(format!("{context} request accounting invariant failed"));
        }
        if self.matched + self.no_match != self.cache_writes_succeeded + self.cache_writes_failed {
            return Err(format!("{context} cache-write accounting invariant failed"));
        }
        if self.serialization_failed > self.cache_writes_failed {
            return Err(format!(
                "{context} serialization accounting invariant failed"
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, JsonSchema)]
pub(crate) struct MetadataEnrichmentByProvider {
    pub(crate) bandcamp: MetadataProviderEnrichmentReport,
    pub(crate) musicbrainz: MetadataProviderEnrichmentReport,
}

impl MetadataEnrichmentByProvider {
    fn get(&self, provider: MetadataEnrichmentProvider) -> &MetadataProviderEnrichmentReport {
        match provider {
            MetadataEnrichmentProvider::Bandcamp => &self.bandcamp,
            MetadataEnrichmentProvider::MusicBrainz => &self.musicbrainz,
        }
    }

    fn get_mut(
        &mut self,
        provider: MetadataEnrichmentProvider,
    ) -> &mut MetadataProviderEnrichmentReport {
        match provider {
            MetadataEnrichmentProvider::Bandcamp => &mut self.bandcamp,
            MetadataEnrichmentProvider::MusicBrainz => &mut self.musicbrainz,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, JsonSchema)]
pub(crate) struct MetadataAutoEnrichmentReport {
    pub(crate) operation_failed: bool,
    pub(crate) requested: usize,
    pub(crate) matched: usize,
    pub(crate) no_match: usize,
    pub(crate) lookup_failed: usize,
    pub(crate) cache_writes_succeeded: usize,
    pub(crate) cache_writes_failed: usize,
    pub(crate) serialization_failed: usize,
    pub(crate) worker_failed: usize,
    pub(crate) writer_failed: usize,
    pub(crate) by_provider: MetadataEnrichmentByProvider,
    pub(crate) failures: Vec<MetadataEnrichmentFailure>,
    pub(crate) failures_truncated: bool,
}

impl MetadataAutoEnrichmentReport {
    pub(crate) fn matched_by(&self, provider: MetadataEnrichmentProvider) -> usize {
        self.by_provider.get(provider).matched
    }

    pub(crate) fn absorb(&mut self, mut other: Self) {
        self.requested += other.requested;
        self.matched += other.matched;
        self.no_match += other.no_match;
        self.lookup_failed += other.lookup_failed;
        self.cache_writes_succeeded += other.cache_writes_succeeded;
        self.cache_writes_failed += other.cache_writes_failed;
        self.serialization_failed += other.serialization_failed;
        self.worker_failed += other.worker_failed;
        self.writer_failed += other.writer_failed;
        self.by_provider.bandcamp.absorb(other.by_provider.bandcamp);
        self.by_provider
            .musicbrainz
            .absorb(other.by_provider.musicbrainz);
        if other.failures_truncated {
            self.failures_truncated = true;
        }
        for failure in other.failures.drain(..) {
            self.push_failure(failure);
        }
        self.refresh_operation_failed();
    }

    fn push_failure(&mut self, failure: MetadataEnrichmentFailure) {
        if self.failures.len() < FAILURE_DETAIL_LIMIT {
            self.failures.push(failure);
        } else {
            self.failures_truncated = true;
        }
    }

    fn push_terminal_failure(&mut self, failure: MetadataEnrichmentFailure) {
        if self.failures.len() >= FAILURE_DETAIL_LIMIT {
            self.failures.pop();
            self.failures_truncated = true;
        }
        self.failures.push(failure);
    }

    fn refresh_operation_failed(&mut self) {
        self.by_provider.bandcamp.refresh_operation_failed();
        self.by_provider.musicbrainz.refresh_operation_failed();
        self.operation_failed = self.lookup_failed > 0
            || self.cache_writes_failed > 0
            || self.serialization_failed > 0
            || self.worker_failed > 0
            || self.writer_failed > 0;
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let global = MetadataProviderEnrichmentReport {
            operation_failed: self.operation_failed,
            requested: self.requested,
            matched: self.matched,
            no_match: self.no_match,
            lookup_failed: self.lookup_failed,
            cache_writes_succeeded: self.cache_writes_succeeded,
            cache_writes_failed: self.cache_writes_failed,
            serialization_failed: self.serialization_failed,
            worker_failed: self.worker_failed,
        };
        global.validate("global")?;
        self.by_provider.bandcamp.validate("bandcamp")?;
        self.by_provider.musicbrainz.validate("musicbrainz")?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MetadataCacheWriterReport {
    pub(crate) attempted: usize,
    pub(crate) succeeded: usize,
    pub(crate) failed: usize,
    pub(crate) dropped_ack_receivers: usize,
    pub(crate) open_failed: usize,
}

fn persist_metadata_cache_write(
    connection: &rusqlite::Connection,
    write: &MetadataCacheWrite,
) -> Result<(), rusqlite::Error> {
    state::set_enrichment(
        connection,
        write.provider.as_str(),
        &write.norm_artist,
        &write.norm_title,
        None,
        Some(&write.match_quality),
        write.response_json.as_deref(),
    )
}

pub(crate) fn run_metadata_cache_writer(
    store_path: &str,
    mut receiver: tokio::sync::mpsc::Receiver<
        CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>,
    >,
) -> MetadataCacheWriterReport {
    let _active_writer = ActiveMetadataWriterGuard::new(store_path);
    let connection = state::open(store_path);
    if let Err(error) = &connection {
        tracing::error!("metadata cache writer open failed: {error}");
    }
    let mut report = MetadataCacheWriterReport {
        open_failed: usize::from(connection.is_err()),
        ..MetadataCacheWriterReport::default()
    };

    while let Some(request) = receiver.blocking_recv() {
        report.attempted += 1;
        let result = match &connection {
            Ok(connection) => {
                persist_metadata_cache_write(connection, &request.payload).map_err(|error| {
                    tracing::warn!(
                        provider = request.payload.provider.as_str(),
                        artist = request.payload.norm_artist.as_str(),
                        title = request.payload.norm_title.as_str(),
                        "metadata cache write failed: {error}"
                    );
                    MetadataCacheWriteError::CacheWriteFailed
                })
            }
            Err(_) => Err(MetadataCacheWriteError::WriterOpenFailed),
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

pub(crate) struct MetadataEnrichmentWriterSession {
    sender: Option<
        tokio::sync::mpsc::Sender<CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>>,
    >,
    writer: Option<tokio::task::JoinHandle<MetadataCacheWriterReport>>,
    writer_failure_context: MetadataEnrichmentFailure,
}

impl MetadataEnrichmentWriterSession {
    pub(crate) fn start(
        store_path: String,
        provider: MetadataEnrichmentProvider,
        request: &MetadataEnrichmentRequest,
    ) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(CACHE_QUEUE_CAPACITY);
        let writer_path = store_path.clone();
        let writer =
            tokio::task::spawn_blocking(move || run_metadata_cache_writer(&writer_path, receiver));
        Self {
            sender: Some(sender),
            writer: Some(writer),
            writer_failure_context: MetadataEnrichmentFailure::new(
                provider,
                request,
                MetadataEnrichmentFailureKind::WriterTaskFailed,
                "metadata cache writer task failed",
            ),
        }
    }

    pub(crate) fn sender(
        &self,
    ) -> tokio::sync::mpsc::Sender<CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>>
    {
        self.sender
            .as_ref()
            .expect("metadata writer session sender should exist before finish")
            .clone()
    }

    pub(crate) async fn finish(
        mut self,
        mut report: MetadataAutoEnrichmentReport,
    ) -> Result<MetadataAutoEnrichmentReport, String> {
        self.sender.take();
        let writer = self
            .writer
            .take()
            .ok_or_else(|| "metadata cache writer handle missing".to_string())?;
        match writer.await {
            Ok(writer_report) => {
                report.writer_failed += writer_report.open_failed;
                if writer_report.dropped_ack_receivers > 0 {
                    report.writer_failed += writer_report.dropped_ack_receivers;
                }
            }
            Err(error) => {
                tracing::error!("metadata cache writer task failed: {error}");
                report.writer_failed += 1;
                // A writer task failure invalidates the persistence lifecycle,
                // so retain its typed context even when lookup details filled
                // the bounded failure list first.
                report.push_terminal_failure(self.writer_failure_context.clone());
            }
        }
        report.refresh_operation_failed();
        report.validate()?;
        Ok(report)
    }
}

impl Drop for MetadataEnrichmentWriterSession {
    fn drop(&mut self) {
        self.sender.take();
    }
}

struct RequestOutcome {
    report: MetadataProviderEnrichmentReport,
    failure: Option<MetadataEnrichmentFailure>,
}

impl RequestOutcome {
    fn new() -> Self {
        Self {
            report: MetadataProviderEnrichmentReport::default(),
            failure: None,
        }
    }
}

async fn send_metadata_cache_message(
    sender: &tokio::sync::mpsc::Sender<
        CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>,
    >,
    write: MetadataCacheWrite,
) -> Result<(), MetadataCacheRequestError> {
    let (acknowledgement, result) = tokio::sync::oneshot::channel();
    sender
        .send(CacheWriteRequest {
            payload: write,
            acknowledgement,
        })
        .await
        .map_err(|_| MetadataCacheRequestError::QueueSendFailed)?;
    result
        .await
        .map_err(|_| MetadataCacheRequestError::AcknowledgementCanceled)?
        .map_err(Into::into)
}

async fn run_request<T, Lookup, LookupFuture, Quality>(
    provider: MetadataEnrichmentProvider,
    request: MetadataEnrichmentRequest,
    sender: tokio::sync::mpsc::Sender<
        CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>,
    >,
    lookup: Lookup,
    quality: Quality,
) -> RequestOutcome
where
    T: Serialize,
    Lookup: FnOnce(MetadataEnrichmentRequest) -> LookupFuture,
    LookupFuture: Future<Output = Result<Option<T>, String>>,
    Quality: FnOnce(&T) -> &'static str,
{
    wait_for_test_lookup_pause(provider, &request).await;
    let mut outcome = RequestOutcome::new();
    let lookup_result = lookup(request.clone()).await;
    let (match_quality, response_json) = match lookup_result {
        Ok(Some(result)) => {
            outcome.report.matched = 1;
            let response_json = match serde_json::to_string(&result) {
                Ok(response_json) => response_json,
                Err(error) => {
                    tracing::warn!(
                        provider = provider.as_str(),
                        artist = request.raw_artist.as_str(),
                        title = request.raw_title.as_str(),
                        "metadata auto-enrichment serialization failed: {error}"
                    );
                    outcome.report.cache_writes_failed = 1;
                    outcome.report.serialization_failed = 1;
                    outcome.failure = Some(MetadataEnrichmentFailure::new(
                        provider,
                        &request,
                        MetadataEnrichmentFailureKind::SerializationFailed,
                        "provider response serialization failed",
                    ));
                    outcome.report.refresh_operation_failed();
                    return outcome;
                }
            };
            (quality(&result).to_string(), Some(response_json))
        }
        Ok(None) => {
            outcome.report.no_match = 1;
            ("none".to_string(), None)
        }
        Err(error) => {
            tracing::warn!(
                provider = provider.as_str(),
                artist = request.raw_artist.as_str(),
                title = request.raw_title.as_str(),
                "metadata auto-enrichment lookup failed: {error}"
            );
            outcome.report.lookup_failed = 1;
            outcome.failure = Some(MetadataEnrichmentFailure::new(
                provider,
                &request,
                MetadataEnrichmentFailureKind::LookupFailed,
                "provider lookup failed",
            ));
            outcome.report.refresh_operation_failed();
            return outcome;
        }
    };

    let write = MetadataCacheWrite {
        provider,
        norm_artist: request.norm_artist.clone(),
        norm_title: request.norm_title.clone(),
        match_quality,
        response_json,
    };
    match send_metadata_cache_message(&sender, write).await {
        Ok(()) => outcome.report.cache_writes_succeeded = 1,
        Err(error) => {
            outcome.report.cache_writes_failed = 1;
            let (kind, summary) = error.failure();
            outcome.failure = Some(MetadataEnrichmentFailure::new(
                provider, &request, kind, summary,
            ));
        }
    }
    outcome.report.refresh_operation_failed();
    outcome
}

pub(crate) async fn run_metadata_provider<T, Lookup, LookupFuture, Quality>(
    provider: MetadataEnrichmentProvider,
    requests: Vec<MetadataEnrichmentRequest>,
    sender: tokio::sync::mpsc::Sender<
        CacheWriteRequest<MetadataCacheWrite, MetadataCacheWriteError>,
    >,
    lookup: Lookup,
    quality: Quality,
) -> MetadataAutoEnrichmentReport
where
    T: Serialize + Send + 'static,
    Lookup: Fn(MetadataEnrichmentRequest) -> LookupFuture + Clone + Send + Sync + 'static,
    LookupFuture: Future<Output = Result<Option<T>, String>> + Send + 'static,
    Quality: Fn(&T) -> &'static str + Clone + Send + Sync + 'static,
{
    let mut report = MetadataAutoEnrichmentReport {
        requested: requests.len(),
        ..MetadataAutoEnrichmentReport::default()
    };
    report.by_provider.get_mut(provider).requested = requests.len();

    let mut pending = requests.into_iter();
    let mut tasks = tokio::task::JoinSet::new();
    let mut task_requests = std::collections::HashMap::new();
    for _ in 0..PROVIDER_CONCURRENCY {
        let Some(request) = pending.next() else {
            break;
        };
        let request_for_failure = request.clone();
        let sender = sender.clone();
        let lookup = lookup.clone();
        let quality = quality.clone();
        let task = tasks.spawn(run_request(provider, request, sender, lookup, quality));
        task_requests.insert(task.id(), request_for_failure);
    }

    while let Some(joined) = tasks.join_next_with_id().await {
        match joined {
            Ok((task_id, outcome)) => {
                task_requests.remove(&task_id);
                report
                    .by_provider
                    .get_mut(provider)
                    .absorb(outcome.report.clone());
                report.matched += outcome.report.matched;
                report.no_match += outcome.report.no_match;
                report.lookup_failed += outcome.report.lookup_failed;
                report.cache_writes_succeeded += outcome.report.cache_writes_succeeded;
                report.cache_writes_failed += outcome.report.cache_writes_failed;
                report.serialization_failed += outcome.report.serialization_failed;
                report.worker_failed += outcome.report.worker_failed;
                if let Some(failure) = outcome.failure {
                    report.push_failure(failure);
                }
            }
            Err(error) => {
                tracing::error!(
                    provider = provider.as_str(),
                    "metadata provider task failed: {error}"
                );
                let provider_report = report.by_provider.get_mut(provider);
                provider_report.worker_failed += 1;
                provider_report.refresh_operation_failed();
                report.worker_failed += 1;
                let request = task_requests.remove(&error.id()).unwrap_or_else(|| {
                    MetadataEnrichmentRequest::new(
                        "unknown".to_string(),
                        "unknown".to_string(),
                        String::new(),
                        String::new(),
                    )
                });
                report.push_failure(MetadataEnrichmentFailure::new(
                    provider,
                    &request,
                    MetadataEnrichmentFailureKind::WorkerFailed,
                    if error.is_panic() {
                        "provider worker panicked"
                    } else if error.is_cancelled() {
                        "provider worker was cancelled"
                    } else {
                        "provider worker task failed"
                    },
                ));
            }
        }

        if let Some(request) = pending.next() {
            let request_for_failure = request.clone();
            let sender = sender.clone();
            let lookup = lookup.clone();
            let quality = quality.clone();
            let task = tasks.spawn(run_request(provider, request, sender, lookup, quality));
            task_requests.insert(task.id(), request_for_failure);
        }
    }

    report.refresh_operation_failed();
    report
}

#[cfg(test)]
#[derive(Clone)]
struct TestLookupPause {
    provider: MetadataEnrichmentProvider,
    norm_artist: String,
    norm_title: String,
    reached: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(test)]
fn test_lookup_pause() -> &'static std::sync::Mutex<Option<TestLookupPause>> {
    static PAUSE: std::sync::OnceLock<std::sync::Mutex<Option<TestLookupPause>>> =
        std::sync::OnceLock::new();
    PAUSE.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg(test)]
pub(crate) struct TestLookupPauseGuard {
    release: std::sync::Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl Drop for TestLookupPauseGuard {
    fn drop(&mut self) {
        if let Ok(mut pause) = test_lookup_pause().lock() {
            pause.take();
        }
        // `notify_one` retains a permit if the worker cloned the pause but has
        // not reached `notified()` yet, so cleanup cannot strand provider work.
        self.release.notify_one();
    }
}

#[cfg(test)]
pub(crate) fn install_test_lookup_pause(
    provider: MetadataEnrichmentProvider,
    norm_artist: String,
    norm_title: String,
) -> (
    TestLookupPauseGuard,
    std::sync::Arc<tokio::sync::Notify>,
    std::sync::Arc<tokio::sync::Notify>,
) {
    let reached = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    let mut pause = test_lookup_pause()
        .lock()
        .expect("metadata lookup pause mutex should not be poisoned");
    assert!(
        pause.is_none(),
        "metadata lookup pause should be installed once"
    );
    *pause = Some(TestLookupPause {
        provider,
        norm_artist,
        norm_title,
        reached: reached.clone(),
        release: release.clone(),
    });
    (
        TestLookupPauseGuard {
            release: release.clone(),
        },
        reached,
        release,
    )
}

#[cfg(test)]
async fn wait_for_test_lookup_pause(
    provider: MetadataEnrichmentProvider,
    request: &MetadataEnrichmentRequest,
) {
    let pause = test_lookup_pause().lock().ok().and_then(|pause| {
        pause
            .as_ref()
            .filter(|pause| {
                pause.provider == provider
                    && pause.norm_artist == request.norm_artist
                    && pause.norm_title == request.norm_title
            })
            .cloned()
    });
    if let Some(pause) = pause {
        pause.reached.notify_one();
        pause.release.notified().await;
    }
}

#[cfg(not(test))]
async fn wait_for_test_lookup_pause(
    _provider: MetadataEnrichmentProvider,
    _request: &MetadataEnrichmentRequest,
) {
}

#[cfg(test)]
fn active_metadata_writers() -> &'static std::sync::Mutex<std::collections::HashMap<String, usize>>
{
    static ACTIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
struct ActiveMetadataWriterGuard {
    store_path: String,
}

#[cfg(test)]
impl ActiveMetadataWriterGuard {
    fn new(store_path: &str) -> Self {
        let mut active = active_metadata_writers()
            .lock()
            .expect("active metadata writer mutex should not be poisoned");
        *active.entry(store_path.to_string()).or_default() += 1;
        Self {
            store_path: store_path.to_string(),
        }
    }
}

#[cfg(test)]
impl Drop for ActiveMetadataWriterGuard {
    fn drop(&mut self) {
        let mut active = active_metadata_writers()
            .lock()
            .expect("active metadata writer mutex should not be poisoned");
        let count = active
            .get_mut(&self.store_path)
            .expect("active metadata writer path should exist");
        *count -= 1;
        if *count == 0 {
            active.remove(&self.store_path);
        }
    }
}

#[cfg(test)]
pub(crate) fn metadata_writer_active_for_test(store_path: &str) -> bool {
    active_metadata_writers()
        .lock()
        .ok()
        .and_then(|active| active.get(store_path).copied())
        .unwrap_or_default()
        > 0
}

#[cfg(not(test))]
struct ActiveMetadataWriterGuard;

#[cfg(not(test))]
impl ActiveMetadataWriterGuard {
    fn new(_store_path: &str) -> Self {
        Self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::ser::Error as _;
    use std::time::Duration;

    const TEST_TIMEOUT: Duration = Duration::from_secs(5);

    fn request(title: &str) -> MetadataEnrichmentRequest {
        MetadataEnrichmentRequest::new(
            "test artist".to_string(),
            title.to_string(),
            "Test Artist".to_string(),
            title.to_string(),
        )
    }

    fn write(title: &str) -> MetadataCacheWrite {
        MetadataCacheWrite {
            provider: MetadataEnrichmentProvider::Bandcamp,
            norm_artist: "test artist".to_string(),
            norm_title: title.to_string(),
            match_quality: "none".to_string(),
            response_json: None,
        }
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_writer_persists_and_acknowledges_rows() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let directory = tempfile::tempdir().expect("metadata writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let session = MetadataEnrichmentWriterSession::start(
                store_path.clone(),
                MetadataEnrichmentProvider::Bandcamp,
                &request("writer success"),
            );
            let result =
                send_metadata_cache_message(&session.sender(), write("writer success")).await;
            assert_eq!(result, Ok(()));
            let report = session
                .finish(MetadataAutoEnrichmentReport::default())
                .await
                .expect("metadata writer session should finish");
            assert!(!report.operation_failed);
            let connection = state::open(&store_path).expect("metadata store should reopen");
            assert!(
                state::get_enrichment(
                    &connection,
                    "bandcamp",
                    "test artist",
                    "writer success",
                    None,
                    false,
                )
                .expect("metadata cache should read")
                .is_some()
            );
        })
        .await
        .expect("metadata writer success should finish within five seconds");
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_writer_reports_selective_and_open_failures() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let directory = tempfile::tempdir().expect("metadata writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let connection = state::open(&store_path).expect("metadata store should initialize");
            connection
                .execute_batch(
                    "CREATE TRIGGER fail_selected_metadata_enrichment
                     BEFORE INSERT ON enrichment_cache
                     WHEN NEW.query_title = 'reject this'
                     BEGIN
                         SELECT RAISE(FAIL, 'queue send failed acknowledgement canceled writer open failed');
                     END;",
                )
                .expect("selective metadata trigger should install");
            drop(connection);

            let session = MetadataEnrichmentWriterSession::start(
                store_path,
                MetadataEnrichmentProvider::Bandcamp,
                &request("reject this"),
            );
            let rejected = send_metadata_cache_message(&session.sender(), write("reject this")).await;
            let accepted = send_metadata_cache_message(&session.sender(), write("accept this")).await;
            assert_eq!(
                rejected.expect_err("selected write should fail"),
                MetadataCacheRequestError::CacheWriteFailed,
            );
            assert_eq!(accepted, Ok(()));
            session
                .finish(MetadataAutoEnrichmentReport::default())
                .await
                .expect("selective metadata writer should finish");

            let invalid_path = directory.path().to_string_lossy().to_string();
            let session = MetadataEnrichmentWriterSession::start(
                invalid_path,
                MetadataEnrichmentProvider::Bandcamp,
                &request("open one"),
            );
            let first = send_metadata_cache_message(&session.sender(), write("open one")).await;
            let second = send_metadata_cache_message(&session.sender(), write("open two")).await;
            for result in [first, second] {
                assert_eq!(
                    result.expect_err("open failure should reject each row"),
                    MetadataCacheRequestError::WriterOpenFailed,
                );
            }
            let report = session
                .finish(MetadataAutoEnrichmentReport::default())
                .await
                .expect("open-failed metadata writer should finish");
            assert_eq!(report.writer_failed, 1);
            assert!(report.operation_failed);
        })
        .await
        .expect("metadata writer failures should finish within five seconds");
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_writer_counts_dropped_acknowledgement_receivers() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let directory = tempfile::tempdir().expect("metadata writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let session = MetadataEnrichmentWriterSession::start(
                store_path,
                MetadataEnrichmentProvider::Bandcamp,
                &request("dropped ack"),
            );
            let (acknowledgement, receiver) = tokio::sync::oneshot::channel();
            drop(receiver);
            session
                .sender()
                .send(CacheWriteRequest {
                    payload: write("dropped ack"),
                    acknowledgement,
                })
                .await
                .expect("dropped-ack request should enqueue");
            let next =
                send_metadata_cache_message(&session.sender(), write("after dropped ack")).await;
            assert_eq!(next, Ok(()));
            let report = session
                .finish(MetadataAutoEnrichmentReport::default())
                .await
                .expect("dropped-ack metadata writer should finish");
            assert_eq!(report.writer_failed, 1);
            assert!(report.operation_failed);
        })
        .await
        .expect("dropped-ack metadata writer should finish within five seconds");
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_writer_report_tracks_terminal_outcomes_and_join_failure() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let directory = tempfile::tempdir().expect("metadata writer directory should create");
            let store_path = directory
                .path()
                .join("internal.sqlite3")
                .to_string_lossy()
                .to_string();
            let connection = state::open(&store_path).expect("metadata store should initialize");
            connection
                .execute_batch(
                    "CREATE TRIGGER fail_writer_report_row
                     BEFORE INSERT ON enrichment_cache
                     WHEN NEW.query_title = 'writer report reject'
                     BEGIN
                         SELECT RAISE(FAIL, 'writer report rejection');
                     END;",
                )
                .expect("writer report trigger should install");
            drop(connection);

            let (sender, receiver) = tokio::sync::mpsc::channel(4);
            let writer_path = store_path.clone();
            let writer = tokio::task::spawn_blocking(move || {
                run_metadata_cache_writer(&writer_path, receiver)
            });
            assert_eq!(
                send_metadata_cache_message(&sender, write("writer report accept")).await,
                Ok(())
            );
            assert_eq!(
                send_metadata_cache_message(&sender, write("writer report reject"))
                    .await
                    .expect_err("writer report selected row should fail"),
                MetadataCacheRequestError::CacheWriteFailed,
            );
            let (acknowledgement, receiver) = tokio::sync::oneshot::channel();
            drop(receiver);
            sender
                .send(CacheWriteRequest {
                    payload: write("writer report dropped ack"),
                    acknowledgement,
                })
                .await
                .expect("writer report dropped-ack row should enqueue");
            drop(sender);
            let report = writer
                .await
                .expect("metadata writer report task should join");
            assert_eq!(
                report,
                MetadataCacheWriterReport {
                    attempted: 3,
                    succeeded: 2,
                    failed: 1,
                    dropped_ack_receivers: 1,
                    open_failed: 0,
                }
            );

            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            let session = MetadataEnrichmentWriterSession {
                sender: Some(sender),
                writer: Some(tokio::spawn(async {
                    panic!("synthetic metadata writer panic")
                })),
                writer_failure_context: MetadataEnrichmentFailure::new(
                    MetadataEnrichmentProvider::MusicBrainz,
                    &request("writer task context"),
                    MetadataEnrichmentFailureKind::WriterTaskFailed,
                    "metadata cache writer task failed",
                ),
            };
            let report = session
                .finish(MetadataAutoEnrichmentReport::default())
                .await
                .expect("writer join failure should remain structured");
            assert_eq!(report.writer_failed, 1);
            assert!(report.operation_failed);
            assert_eq!(report.failures.len(), 1);
            assert_eq!(
                report.failures[0],
                MetadataEnrichmentFailure {
                    provider: MetadataEnrichmentProvider::MusicBrainz,
                    normalized_artist: "test artist".to_string(),
                    normalized_title: "writer task context".to_string(),
                    kind: MetadataEnrichmentFailureKind::WriterTaskFailed,
                    summary: "metadata cache writer task failed".to_string(),
                }
            );

            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            let session = MetadataEnrichmentWriterSession {
                sender: Some(sender),
                writer: Some(tokio::spawn(async {
                    panic!("synthetic capped metadata writer panic")
                })),
                writer_failure_context: MetadataEnrichmentFailure::new(
                    MetadataEnrichmentProvider::Bandcamp,
                    &request("capped writer context"),
                    MetadataEnrichmentFailureKind::WriterTaskFailed,
                    "metadata cache writer task failed",
                ),
            };
            let mut capped = MetadataAutoEnrichmentReport::default();
            for index in 0..FAILURE_DETAIL_LIMIT {
                capped.push_failure(MetadataEnrichmentFailure::new(
                    MetadataEnrichmentProvider::Bandcamp,
                    &request(&format!("prior failure {index}")),
                    MetadataEnrichmentFailureKind::LookupFailed,
                    "provider lookup failed",
                ));
            }
            let capped = session
                .finish(capped)
                .await
                .expect("capped writer join failure should remain structured");
            assert_eq!(capped.failures.len(), FAILURE_DETAIL_LIMIT);
            assert!(capped.failures_truncated);
            assert!(capped.failures.iter().any(|failure| {
                failure.kind == MetadataEnrichmentFailureKind::WriterTaskFailed
                    && failure.provider == MetadataEnrichmentProvider::Bandcamp
                    && failure.normalized_artist == "test artist"
                    && failure.normalized_title == "capped writer context"
            }));
        })
        .await
        .expect("metadata writer report cases should finish within five seconds");
    }

    struct SerializationFailure;

    impl Serialize for SerializationFailure {
        fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            Err(S::Error::custom("synthetic secret=must-not-escape"))
        }
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_report_accounts_for_serialization_and_queue_failures() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let (sender, receiver) = tokio::sync::mpsc::channel(1);
            drop(receiver);
            let queue_report = run_metadata_provider(
                MetadataEnrichmentProvider::Bandcamp,
                vec![request("queue closed")],
                sender,
                |_| async { Ok::<Option<serde_json::Value>, String>(None) },
                |_| "exact",
            )
            .await;
            assert_eq!(queue_report.no_match, 1);
            assert_eq!(queue_report.cache_writes_failed, 1);
            assert_eq!(
                queue_report.failures[0].kind,
                MetadataEnrichmentFailureKind::QueueSendFailed
            );
            queue_report
                .validate()
                .expect("queue report invariants should hold");

            let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
            let writer = tokio::spawn(async move {
                if let Some(request) = receiver.recv().await {
                    drop(request);
                }
            });
            let ack_report = run_metadata_provider(
                MetadataEnrichmentProvider::Bandcamp,
                vec![request("ack canceled")],
                sender,
                |_| async { Ok::<Option<serde_json::Value>, String>(None) },
                |_| "exact",
            )
            .await;
            writer.await.expect("ack cancellation writer should join");
            assert_eq!(ack_report.cache_writes_failed, 1);
            assert_eq!(
                ack_report.failures[0].kind,
                MetadataEnrichmentFailureKind::AcknowledgementCanceled
            );
            ack_report
                .validate()
                .expect("ack report invariants should hold");

            let (sender, _receiver) = tokio::sync::mpsc::channel(1);
            let serialization_report = run_metadata_provider(
                MetadataEnrichmentProvider::Bandcamp,
                vec![request("serialize")],
                sender,
                |_| async { Ok::<_, String>(Some(SerializationFailure)) },
                |_| "exact",
            )
            .await;
            assert_eq!(serialization_report.matched, 1);
            assert_eq!(serialization_report.serialization_failed, 1);
            assert_eq!(serialization_report.cache_writes_failed, 1);
            assert!(!serialization_report.failures[0].summary.contains("secret"));
            serialization_report
                .validate()
                .expect("serialization report invariants should hold");
        })
        .await
        .expect("metadata report failure cases should finish within five seconds");
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_report_bounds_failures_and_preserves_invariants() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let (sender, _receiver) = tokio::sync::mpsc::channel(1);
            let requests = (0..55)
                .map(|index| request(&format!("failure {index}")))
                .collect();
            let report = run_metadata_provider::<serde_json::Value, _, _, _>(
                MetadataEnrichmentProvider::MusicBrainz,
                requests,
                sender,
                |_| async { Err("remote body that must not be returned".to_string()) },
                |_| "exact",
            )
            .await;
            assert_eq!(report.requested, 55);
            assert_eq!(report.lookup_failed, 55);
            assert_eq!(report.failures.len(), FAILURE_DETAIL_LIMIT);
            assert!(report.failures_truncated);
            assert!(report.operation_failed);
            assert!(
                report
                    .failures
                    .iter()
                    .all(|failure| failure.summary == "provider lookup failed")
            );
            report
                .validate()
                .expect("bounded failure report invariants should hold");
        })
        .await
        .expect("bounded metadata report should finish within five seconds");
    }

    #[tokio::test]
    async fn metadata_auto_enrichment_report_counts_worker_panics() {
        tokio::time::timeout(TEST_TIMEOUT, async {
            let (sender, _receiver) = tokio::sync::mpsc::channel(1);
            let report = run_metadata_provider::<serde_json::Value, _, _, _>(
                MetadataEnrichmentProvider::Bandcamp,
                vec![request("panic")],
                sender,
                |_| async { panic!("synthetic full panic prose") },
                |_| "exact",
            )
            .await;
            assert_eq!(report.worker_failed, 1);
            assert_eq!(report.failures[0].summary, "provider worker panicked");
            report
                .validate()
                .expect("worker failure report invariants should hold");
        })
        .await
        .expect("worker panic report should finish within five seconds");
    }
}
