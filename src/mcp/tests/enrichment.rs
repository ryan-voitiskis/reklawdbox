use crate::mcp::enrichment::{
    BatchPage, DiscogsAuthTestDependencies, EnrichTracksParams, InMemoryDiscogsSessionPersistence,
    LookupDiscogsParams, ResolveFormat, ResolveTrackDataParams, ResolveTracksDataParams,
    auth_remediation_message, lookup_discogs_remote, lookup_output_with_cache_metadata,
    resolve_discogs_auth_transition_for_test, resolve_pending_tracks, resolve_single_track,
    set_test_discogs_lookup_override,
};
use crate::mcp::library::SearchFilterParams;
use crate::mcp::server::ReklawdboxServer;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rusqlite::{Connection, params};

use crate::adapters::state as store;
use crate::domain::metadata::{EditableField, TrackChange};

use super::common::{
    create_enrich_cache_writer_test_server, create_real_server_with_temp_store,
    create_selector_pagination_test_db, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track, make_test_track, sample_real_tracks, set_test_audio_analysis,
    track_ids, write_test_audio_file,
};

const DISCOGS_AUTH_TEST_NOW: i64 = 2_000_000_000;

const DISCOGS_AUTH_TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DiscogsBrokerEndpoint {
    Start,
    Status,
    Finalize,
    Search,
}

#[derive(Clone)]
struct DiscogsBrokerFailure {
    status: String,
    body: String,
}

struct DiscogsBrokerDelay {
    endpoint: DiscogsBrokerEndpoint,
    entered: Option<tokio::sync::oneshot::Sender<()>>,
    release: Option<tokio::sync::oneshot::Receiver<()>>,
}

struct DiscogsBrokerDelayControl {
    entered: tokio::sync::oneshot::Receiver<()>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

impl DiscogsBrokerDelayControl {
    async fn wait_until_entered(&mut self, phase: &str) -> Result<(), String> {
        tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, &mut self.entered)
            .await
            .map_err(|_| format!("{phase} did not reach the delayed broker response"))?
            .map_err(|_| format!("{phase} delay notification was canceled"))
    }

    fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

struct DiscogsBrokerFixtureState {
    starts: AtomicUsize,
    statuses: AtomicUsize,
    finalizes: AtomicUsize,
    searches: AtomicUsize,
    request_changed: tokio::sync::Notify,
    status: Mutex<String>,
    rejected_tokens: Mutex<HashSet<String>>,
    failures: Mutex<HashMap<DiscogsBrokerEndpoint, DiscogsBrokerFailure>>,
    delay: Mutex<Option<DiscogsBrokerDelay>>,
}

impl DiscogsBrokerFixtureState {
    fn new(status: &str) -> Self {
        Self {
            starts: AtomicUsize::new(0),
            statuses: AtomicUsize::new(0),
            finalizes: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
            request_changed: tokio::sync::Notify::new(),
            status: Mutex::new(status.to_string()),
            rejected_tokens: Mutex::new(HashSet::new()),
            failures: Mutex::new(HashMap::new()),
            delay: Mutex::new(None),
        }
    }

    fn count(&self, endpoint: DiscogsBrokerEndpoint) -> usize {
        match endpoint {
            DiscogsBrokerEndpoint::Start => self.starts.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Status => self.statuses.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Finalize => self.finalizes.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Search => self.searches.load(Ordering::SeqCst),
        }
    }

    fn record(&self, endpoint: DiscogsBrokerEndpoint) {
        match endpoint {
            DiscogsBrokerEndpoint::Start => &self.starts,
            DiscogsBrokerEndpoint::Status => &self.statuses,
            DiscogsBrokerEndpoint::Finalize => &self.finalizes,
            DiscogsBrokerEndpoint::Search => &self.searches,
        }
        .fetch_add(1, Ordering::SeqCst);
        self.request_changed.notify_waiters();
    }

    async fn wait_for_count(
        &self,
        endpoint: DiscogsBrokerEndpoint,
        expected: usize,
        phase: &str,
    ) -> Result<(), String> {
        let wait = async {
            loop {
                let notified = self.request_changed.notified();
                if self.count(endpoint) >= expected {
                    return;
                }
                notified.await;
            }
        };
        tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, wait)
            .await
            .map_err(|_| format!("{phase} did not observe {expected} broker requests"))
    }

    fn reject_token(&self, token: &str) {
        self.rejected_tokens
            .lock()
            .expect("broker fixture rejected-token mutex should not be poisoned")
            .insert(token.to_string());
    }

    fn fail_endpoint(&self, endpoint: DiscogsBrokerEndpoint, body: &str) {
        self.failures
            .lock()
            .expect("broker fixture failure mutex should not be poisoned")
            .insert(
                endpoint,
                DiscogsBrokerFailure {
                    status: "500 Internal Server Error".to_string(),
                    body: body.to_string(),
                },
            );
    }

    fn failure(&self, endpoint: DiscogsBrokerEndpoint) -> Option<DiscogsBrokerFailure> {
        self.failures
            .lock()
            .expect("broker fixture failure mutex should not be poisoned")
            .get(&endpoint)
            .cloned()
    }

    fn delay_next(&self, endpoint: DiscogsBrokerEndpoint) -> DiscogsBrokerDelayControl {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let mut delay = self
            .delay
            .lock()
            .expect("broker fixture delay mutex should not be poisoned");
        assert!(delay.is_none(), "only one broker delay may be active");
        *delay = Some(DiscogsBrokerDelay {
            endpoint,
            entered: Some(entered_tx),
            release: Some(release_rx),
        });
        DiscogsBrokerDelayControl {
            entered: entered_rx,
            release: Some(release_tx),
        }
    }

    async fn apply_delay(&self, endpoint: DiscogsBrokerEndpoint) -> Result<(), String> {
        let delayed = {
            let mut delay = self
                .delay
                .lock()
                .map_err(|_| "broker fixture delay mutex poisoned".to_string())?;
            if delay
                .as_ref()
                .is_some_and(|delay| delay.endpoint == endpoint)
            {
                delay.take()
            } else {
                None
            }
        };
        if let Some(mut delayed) = delayed {
            if let Some(entered) = delayed.entered.take() {
                let _ = entered.send(());
            }
            let release = delayed
                .release
                .take()
                .ok_or_else(|| "broker fixture release channel missing".to_string())?;
            tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, release)
                .await
                .map_err(|_| "broker fixture delayed response timed out".to_string())?
                .map_err(|_| "broker fixture delayed response canceled".to_string())?;
        }
        Ok(())
    }
}

struct DiscogsBrokerFixture {
    base_url: String,
    state: Arc<DiscogsBrokerFixtureState>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<(), String>>>,
}

impl DiscogsBrokerFixture {
    async fn start(status: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("local Discogs broker fixture should bind");
        let address = listener
            .local_addr()
            .expect("local Discogs broker fixture should have an address");
        let state = Arc::new(DiscogsBrokerFixtureState::new(status));
        let server_state = Arc::clone(&state);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutdown_rx => break,
                    accepted = listener.accept() => {
                        let (stream, _) = accepted
                            .map_err(|error| format!("broker fixture accept failed: {error}"))?;
                        let state = Arc::clone(&server_state);
                        connections.spawn(async move {
                            let _ = serve_discogs_broker_connection(stream, state).await;
                        });
                    }
                }
            }
            connections.abort_all();
            while connections.join_next().await.is_some() {}
            Ok(())
        });
        Self {
            base_url: format!("http://{address}"),
            state,
            shutdown: Some(shutdown_tx),
            task: Some(task),
        }
    }

    fn config(&self) -> crate::adapters::providers::discogs::BrokerConfig {
        crate::adapters::providers::discogs::BrokerConfig {
            base_url: self.base_url.clone(),
            broker_token: None,
        }
    }

    async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let mut task = self.task.take().expect("broker fixture task should exist");
        match tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, &mut task).await {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(error))) => panic!("broker fixture shutdown failed: {error}"),
            Ok(Err(error)) => panic!("broker fixture task failed: {error}"),
            Err(_) => {
                task.abort();
                let _ = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, &mut task).await;
                panic!("broker fixture shutdown timed out");
            }
        }
    }
}

impl Drop for DiscogsBrokerFixture {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn read_discogs_broker_request(stream: &mut tokio::net::TcpStream) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut buffer)
            .await
            .map_err(|error| format!("broker fixture request read failed: {error}"))?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 64 * 1024 {
            return Err("broker fixture request headers too large".to_string());
        }
    }
    String::from_utf8(request).map_err(|_| "broker fixture request was not UTF-8".to_string())
}

fn discogs_bearer_token(request: &str) -> Option<&str> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("authorization") {
            value.trim().strip_prefix("Bearer ")
        } else {
            None
        }
    })
}

async fn write_discogs_broker_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| format!("broker fixture response write failed: {error}"))
}

async fn serve_discogs_broker_connection(
    mut stream: tokio::net::TcpStream,
    state: Arc<DiscogsBrokerFixtureState>,
) -> Result<(), String> {
    let request = read_discogs_broker_request(&mut stream).await?;
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "broker fixture request line missing".to_string())?;

    let endpoint = if path.starts_with("/v1/device/session/start") {
        DiscogsBrokerEndpoint::Start
    } else if path.starts_with("/v1/device/session/status") {
        DiscogsBrokerEndpoint::Status
    } else if path.starts_with("/v1/device/session/finalize") {
        DiscogsBrokerEndpoint::Finalize
    } else if path.starts_with("/v1/discogs/proxy/search") {
        DiscogsBrokerEndpoint::Search
    } else {
        return write_discogs_broker_response(
            &mut stream,
            "404 Not Found",
            r#"{"error":"not found"}"#,
        )
        .await;
    };

    state.record(endpoint);
    state.apply_delay(endpoint).await?;
    if let Some(failure) = state.failure(endpoint) {
        return write_discogs_broker_response(&mut stream, &failure.status, &failure.body).await;
    }

    match endpoint {
        DiscogsBrokerEndpoint::Start => {
            write_discogs_broker_response(
                &mut stream,
                "200 OK",
                r#"{"device_id":"device-test-value","pending_token":"pending-test-value","auth_url":"https://auth.example/device","poll_interval_seconds":1,"expires_at":2000003600}"#,
            )
            .await
        }
        DiscogsBrokerEndpoint::Status => {
            let status = state
                .status
                .lock()
                .map_err(|_| "broker fixture status mutex poisoned".to_string())?
                .clone();
            write_discogs_broker_response(
                &mut stream,
                "200 OK",
                &format!(r#"{{"status":"{status}","expires_at":2000003600}}"#),
            )
            .await
        }
        DiscogsBrokerEndpoint::Finalize => {
            write_discogs_broker_response(
                &mut stream,
                "200 OK",
                r#"{"session_token":"session-test-value","expires_at":2000003600}"#,
            )
            .await
        }
        DiscogsBrokerEndpoint::Search => {
            let rejected = discogs_bearer_token(&request).is_some_and(|token| {
                state
                    .rejected_tokens
                    .lock()
                    .expect("broker fixture rejected-token mutex should not be poisoned")
                    .contains(token)
            });
            if rejected {
                write_discogs_broker_response(
                    &mut stream,
                    "401 Unauthorized",
                    r#"{"error":"authorization required"}"#,
                )
                .await
            } else {
                write_discogs_broker_response(&mut stream, "200 OK", r#"{"result":null}"#).await
            }
        }
    }
}

fn install_discogs_auth_test_dependencies(
    server: &ReklawdboxServer,
    fixture: &DiscogsBrokerFixture,
    persistence: Arc<InMemoryDiscogsSessionPersistence>,
    entry_barrier: Option<Arc<tokio::sync::Barrier>>,
) {
    let mut dependencies =
        DiscogsAuthTestDependencies::new(fixture.config(), DISCOGS_AUTH_TEST_NOW, persistence);
    if let Some(barrier) = entry_barrier {
        dependencies = dependencies.with_entry_barrier(barrier);
    }
    server
        .set_discogs_auth_test_dependencies(dependencies)
        .expect("per-server Discogs auth dependencies should install");
}

fn discogs_auth_url(
    result: &Result<
        Option<crate::adapters::providers::discogs::DiscogsResult>,
        crate::adapters::providers::discogs::LookupError,
    >,
) -> Option<&str> {
    match result {
        Err(crate::adapters::providers::discogs::LookupError::AuthRequired(remediation)) => {
            remediation.auth_url.as_deref()
        }
        Ok(_) | Err(_) => None,
    }
}

struct DiscogsAuthTasks<T> {
    handles: Vec<Option<tokio::task::JoinHandle<T>>>,
}

impl<T> Default for DiscogsAuthTasks<T> {
    fn default() -> Self {
        Self {
            handles: Vec::new(),
        }
    }
}

impl<T: Send + 'static> DiscogsAuthTasks<T> {
    fn spawn(&mut self, future: impl Future<Output = T> + Send + 'static) -> usize {
        let index = self.handles.len();
        self.handles.push(Some(tokio::spawn(future)));
        index
    }

    async fn join(&mut self, index: usize, phase: &str) -> Result<T, String> {
        let joined = {
            let handle = self
                .handles
                .get_mut(index)
                .and_then(Option::as_mut)
                .ok_or_else(|| format!("{phase} task handle is unavailable"))?;
            tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, handle).await
        };

        match joined {
            Ok(Ok(output)) => {
                self.handles[index] = None;
                Ok(output)
            }
            Ok(Err(error)) => {
                self.handles[index] = None;
                Err(format!("{phase} task failed: {error}"))
            }
            Err(_) => {
                let cleanup = self.abort_and_reap(index, phase).await;
                match cleanup {
                    Ok(()) => Err(format!(
                        "{phase} did not finish within five seconds; task aborted and reaped"
                    )),
                    Err(cleanup_error) => Err(format!(
                        "{phase} did not finish within five seconds; {cleanup_error}"
                    )),
                }
            }
        }
    }

    async fn abort_and_reap(&mut self, index: usize, phase: &str) -> Result<(), String> {
        let Some(handle) = self.handles.get_mut(index).and_then(Option::as_mut) else {
            return Ok(());
        };
        handle.abort();
        let reaped = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, handle).await;
        match reaped {
            Ok(_) => {
                self.handles[index] = None;
                Ok(())
            }
            Err(_) => Err(format!(
                "{phase} task did not reap within five seconds after abort"
            )),
        }
    }

    async fn abort_all_and_reap(&mut self, phase: &str) -> Result<(), String> {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }

        let mut cleanup_errors = Vec::new();
        for index in 0..self.handles.len() {
            if let Err(error) = self.abort_and_reap(index, phase).await {
                cleanup_errors.push(error);
            }
        }
        if cleanup_errors.is_empty() {
            Ok(())
        } else {
            Err(cleanup_errors.join("; "))
        }
    }

    fn has_active_tasks(&self) -> bool {
        self.handles.iter().any(Option::is_some)
    }
}

impl<T> Drop for DiscogsAuthTasks<T> {
    fn drop(&mut self) {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }
    }
}

async fn finish_discogs_auth_scenario<T: Send + 'static>(
    tasks: &mut DiscogsAuthTasks<T>,
    outcome: Result<Result<(), String>, tokio::time::error::Elapsed>,
    phase: &str,
) {
    let failure = match outcome {
        Ok(Ok(())) if !tasks.has_active_tasks() => return,
        Ok(Ok(())) => format!("{phase} left an auth task active"),
        Ok(Err(error)) => format!("{phase} failed: {error}"),
        Err(_) => format!("{phase} did not finish within five seconds"),
    };

    let cleanup = tasks.abort_all_and_reap(phase).await;
    match cleanup {
        Ok(()) => panic!("{failure}; remaining tasks aborted and reaped"),
        Err(error) => panic!("{failure}; cleanup failed: {error}"),
    }
}

fn set_discogs_pending(server: &ReklawdboxServer, device_id: &str, expires_at: i64) {
    *server
        .context
        .enrichment
        .discogs_pending
        .lock()
        .expect("Discogs pending state should not be poisoned") =
        Some(crate::adapters::providers::discogs::PendingDeviceSession {
            device_id: device_id.to_string(),
            pending_token: "pending-fixture-value".to_string(),
            auth_url: "https://auth.example/device".to_string(),
            poll_interval_seconds: 1,
            expires_at,
        });
}

fn assert_sanitized_discogs_transition_error(
    result: Result<
        Option<crate::adapters::providers::discogs::AuthRemediation>,
        crate::adapters::providers::discogs::LookupError,
    >,
    expected: &str,
) {
    match result {
        Err(crate::adapters::providers::discogs::LookupError::Message(message)) => assert!(
            message == expected,
            "Discogs transition failure should use the stable sanitized message"
        ),
        Ok(_) | Err(_) => panic!("Discogs transition should return a sanitized message error"),
    }
}

fn discogs_batch_params(track_ids: &[&str]) -> EnrichTracksParams {
    EnrichTracksParams {
        filters: SearchFilterParams::default(),
        track_ids: Some(
            track_ids
                .iter()
                .map(|track_id| (*track_id).to_string())
                .collect(),
        ),
        playlist_id: None,
        max_tracks: Some(u32::try_from(track_ids.len()).expect("test track count should fit u32")),
        offset: None,
        providers: Some(vec![
            crate::application::enrichment::model::EnrichmentProvider::Discogs,
        ]),
        skip_cached: Some(false),
        force_refresh: Some(true),
        concurrency: Some(2),
    }
}

async fn run_discogs_batch_with_timeout(
    server: &ReklawdboxServer,
    track_ids: &[&str],
    context: &str,
) -> CallToolResult {
    tokio::time::timeout(
        Duration::from_secs(5),
        server.enrich_tracks(Parameters(discogs_batch_params(track_ids))),
    )
    .await
    .unwrap_or_else(|_| panic!("{context} should finish within five seconds"))
    .unwrap_or_else(|error| panic!("{context} should return a batch payload: {error:?}"))
}

fn set_enrich_test_track_title(conn: &Connection, track_id: &str, title: &str) {
    conn.execute(
        "UPDATE djmdContent SET Title = ?2 WHERE ID = ?1",
        params![track_id, title],
    )
    .expect("enrichment test track title should update");
}

fn install_enrichment_insert_failure(server: &ReklawdboxServer, raw_title: &str) {
    let normalized_title = crate::domain::metadata::normalize_for_matching(raw_title);
    let escaped_title = normalized_title.replace('\'', "''");
    let sql = format!(
        "CREATE TRIGGER fail_selected_enrichment
         BEFORE INSERT ON enrichment_cache
         WHEN NEW.query_title = '{escaped_title}'
         BEGIN
             SELECT RAISE(FAIL, 'selected cache write failure');
         END;"
    );
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available for trigger setup");
    conn.execute_batch(&sql)
        .expect("selective cache-write trigger should install");
}

fn discogs_match(title: &str) -> crate::adapters::providers::discogs::DiscogsResult {
    crate::adapters::providers::discogs::DiscogsResult {
        title: title.to_string(),
        year: "2026".to_string(),
        label: "Cache Ack Records".to_string(),
        genres: vec!["Electronic".to_string()],
        styles: vec!["Techno".to_string()],
        url: "https://www.discogs.com/release/test".to_string(),
        cover_image: String::new(),
        fuzzy_match: false,
    }
}

fn assert_cache_write_summary(
    payload: &serde_json::Value,
    attempted: u64,
    succeeded: u64,
    failed: u64,
) {
    assert_eq!(payload["summary"]["cache_writes"]["attempted"], attempted);
    assert_eq!(payload["summary"]["cache_writes"]["succeeded"], succeeded);
    assert_eq!(payload["summary"]["cache_writes"]["failed"], failed);
    assert_eq!(attempted, succeeded + failed);
}

fn assert_cache_write_failure_context(
    payload: &serde_json::Value,
    track_id: &str,
    title: &str,
    error_prefix: &str,
) {
    let failure = payload["failures"]
        .as_array()
        .expect("failures should be an array")
        .iter()
        .find(|failure| failure["track_id"] == track_id)
        .unwrap_or_else(|| panic!("failure for {track_id} should be present"));
    assert_eq!(failure["artist"], "Aníbal");
    assert_eq!(failure["title"], title);
    assert_eq!(failure["provider"], "discogs");
    assert!(
        failure["error"]
            .as_str()
            .expect("cache-write failure should include an error string")
            .starts_with(error_prefix),
        "cache-write error should start with {error_prefix}: {failure:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_concurrent_single_start() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            persistence,
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let first_server = server.clone();
        let first = tasks.spawn(async move {
            lookup_discogs_remote(&first_server, "Race Artist", "Race Start One", None).await
        });
        let second_server = server.clone();
        let second = tasks.spawn(async move {
            lookup_discogs_remote(&second_server, "Race Artist", "Race Start Two", None).await
        });

        delayed_start
            .wait_until_entered("concurrent device-session start")
            .await?;
        delayed_start.release();

        let first = tasks.join(first, "first concurrent auth lookup").await?;
        let second = tasks.join(second, "second concurrent auth lookup").await?;
        assert_eq!(
            discogs_auth_url(&first),
            Some("https://auth.example/device")
        );
        assert_eq!(
            discogs_auth_url(&second),
            Some("https://auth.example/device")
        );
        assert_eq!(
            fixture.state.count(DiscogsBrokerEndpoint::Start),
            1,
            "concurrent unauthenticated calls must start one device session"
        );
        let pending_device_id = server
            .context
            .enrichment
            .discogs_pending
            .lock()
            .expect("Discogs pending state should not be poisoned")
            .as_ref()
            .map(|pending| pending.device_id.clone());
        assert_eq!(pending_device_id.as_deref(), Some("device-test-value"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "concurrent start scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_concurrent_single_finalize() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("authorized").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "authorized-device", DISCOGS_AUTH_TEST_NOW + 3_600);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            Arc::clone(&persistence),
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_finalize = fixture.state.delay_next(DiscogsBrokerEndpoint::Finalize);

        let first_server = server.clone();
        let first = tasks.spawn(async move {
            lookup_discogs_remote(
                &first_server,
                "Finalize Artist",
                "Finalize Search One",
                None,
            )
            .await
        });
        let second_server = server.clone();
        let second = tasks.spawn(async move {
            lookup_discogs_remote(
                &second_server,
                "Finalize Artist",
                "Finalize Search Two",
                None,
            )
            .await
        });

        delayed_finalize
            .wait_until_entered("authorized device-session finalize")
            .await?;
        delayed_finalize.release();

        let first = tasks.join(first, "first post-finalize search").await?;
        let second = tasks.join(second, "second post-finalize search").await?;
        assert!(matches!(first, Ok(None)));
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Finalize), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.store_count(), 1);
        assert!(persistence.has_session(&fixture.base_url));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_none()
        );
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "concurrent finalize scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_transition_expired_pending_starts_one_replacement() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "expired-device", DISCOGS_AUTH_TEST_NOW);
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            persistence,
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let first_server = server.clone();
        let first = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&first_server).await });
        let second_server = server.clone();
        let second = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&second_server).await });

        delayed_start
            .wait_until_entered("expired pending replacement start")
            .await?;
        delayed_start.release();

        let first = tasks.join(first, "first expired transition").await?;
        let second = tasks.join(second, "second expired transition").await?;
        assert!(matches!(first, Ok(Some(_))));
        assert!(matches!(second, Ok(Some(_))));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        let pending_device_id = server
            .context
            .enrichment
            .discogs_pending
            .lock()
            .expect("Discogs pending state should not be poisoned")
            .as_ref()
            .map(|pending| pending.device_id.clone());
        assert_eq!(pending_device_id.as_deref(), Some("device-test-value"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "expired pending scenario").await;
}

#[tokio::test]
async fn discogs_auth_transition_persistence_errors_keep_recoverable_state() {
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("authorized").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.fail_next_store();
        let server = ReklawdboxServer::new(None);
        set_discogs_pending(&server, "persistence-device", DISCOGS_AUTH_TEST_NOW + 3_600);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);

        let first = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(
            first,
            Err(crate::adapters::providers::discogs::LookupError::Message(_))
        ));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        assert!(!persistence.has_session(&fixture.base_url));

        let second = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Finalize), 2);
        assert_eq!(persistence.store_count(), 1);
        assert!(persistence.has_session(&fixture.base_url));
        assert!(
            server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_none()
        );

        persistence.set_session(
            &fixture.base_url,
            "expired-session-fixture",
            DISCOGS_AUTH_TEST_NOW,
        );
        persistence.fail_next_clear();
        let clear_failure = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(
            clear_failure,
            Err(crate::adapters::providers::discogs::LookupError::Message(_))
        ));
        assert!(persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
    };

    tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario)
        .await
        .expect("persistence error scenario should finish within five seconds");
}

#[tokio::test]
async fn discogs_auth_transition_errors_are_sanitized() {
    let scenario = async {
        let start_fixture = DiscogsBrokerFixture::start("pending").await;
        start_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Start,
            r#"{"pending_token":"fixture-private-start","broker_credential":"fixture-private-broker"}"#,
        );
        let start_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let start_server = ReklawdboxServer::new(None);
        start_server
            .set_discogs_auth_test_dependencies(DiscogsAuthTestDependencies::new(
                crate::adapters::providers::discogs::BrokerConfig {
                    base_url: start_fixture.base_url.clone(),
                    broker_token: Some("fixture-private-broker".to_string()),
                },
                DISCOGS_AUTH_TEST_NOW,
                Arc::clone(&start_persistence),
            ))
            .expect("start-error dependencies should install");
        let start_result = resolve_discogs_auth_transition_for_test(&start_server).await;
        assert_sanitized_discogs_transition_error(
            start_result,
            "Discogs authentication start failed. Retry the lookup to start authorization.",
        );
        assert_eq!(start_fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert!(!start_persistence.has_session(&start_fixture.base_url));
        start_fixture.shutdown().await;

        let status_fixture = DiscogsBrokerFixture::start("pending").await;
        status_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Status,
            r#"{"pending_token":"fixture-private-status","response_header":"fixture-private-header"}"#,
        );
        let status_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let status_server = ReklawdboxServer::new(None);
        set_discogs_pending(
            &status_server,
            "status-error-device",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        install_discogs_auth_test_dependencies(
            &status_server,
            &status_fixture,
            status_persistence,
            None,
        );
        let status_result = resolve_discogs_auth_transition_for_test(&status_server).await;
        assert_sanitized_discogs_transition_error(
            status_result,
            "Discogs authentication status check failed. Retry the lookup to continue authorization.",
        );
        assert_eq!(status_fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert!(
            status_server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        status_fixture.shutdown().await;

        let finalize_fixture = DiscogsBrokerFixture::start("authorized").await;
        finalize_fixture.state.fail_endpoint(
            DiscogsBrokerEndpoint::Finalize,
            r#"{"session_token":"fixture-private-session","pending_token":"fixture-private-finalize"}"#,
        );
        let finalize_persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let finalize_server = ReklawdboxServer::new(None);
        set_discogs_pending(
            &finalize_server,
            "finalize-error-device",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        install_discogs_auth_test_dependencies(
            &finalize_server,
            &finalize_fixture,
            Arc::clone(&finalize_persistence),
            None,
        );
        let finalize_result = resolve_discogs_auth_transition_for_test(&finalize_server).await;
        assert_sanitized_discogs_transition_error(
            finalize_result,
            "Discogs authentication finalization failed. Retry the lookup to complete authorization.",
        );
        assert_eq!(
            finalize_fixture
                .state
                .count(DiscogsBrokerEndpoint::Finalize),
            1
        );
        assert!(!finalize_persistence.has_session(&finalize_fixture.base_url));
        assert!(
            finalize_server
                .context
                .enrichment
                .discogs_pending
                .lock()
                .expect("Discogs pending state should not be poisoned")
                .is_some()
        );
        finalize_fixture.shutdown().await;

        let closed_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("closed-endpoint listener should bind");
        let closed_address = closed_listener
            .local_addr()
            .expect("closed-endpoint listener should have an address");
        drop(closed_listener);
        let connection_server = ReklawdboxServer::new(None);
        connection_server
            .set_discogs_auth_test_dependencies(DiscogsAuthTestDependencies::new(
                crate::adapters::providers::discogs::BrokerConfig {
                    base_url: format!("http://{closed_address}"),
                    broker_token: None,
                },
                DISCOGS_AUTH_TEST_NOW,
                Arc::new(InMemoryDiscogsSessionPersistence::default()),
            ))
            .expect("connection-error dependencies should install");
        let connection_result = resolve_discogs_auth_transition_for_test(&connection_server).await;
        assert_sanitized_discogs_transition_error(
            connection_result,
            "Discogs authentication start failed. Retry the lookup to start authorization.",
        );
    };

    tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario)
        .await
        .expect("sanitized transition-error scenario should finish within five seconds");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_transition_cancelled_waiter_reuses_committed_state() {
    let mut tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let owner_server = server.clone();
        let owner = tasks
            .spawn(async move { resolve_discogs_auth_transition_for_test(&owner_server).await });
        delayed_start
            .wait_until_entered("owner device-session start")
            .await?;

        let (waiter_entered_tx, waiter_entered_rx) = tokio::sync::oneshot::channel();
        let waiter_server = server.clone();
        let waiter = tasks.spawn(async move {
            let _ = waiter_entered_tx.send(());
            resolve_discogs_auth_transition_for_test(&waiter_server).await
        });
        tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, waiter_entered_rx)
            .await
            .map_err(|_| "waiting lookup should enter within five seconds".to_string())?
            .map_err(|_| "waiting lookup entry notification should arrive".to_string())?;
        tasks
            .abort_and_reap(waiter, "cancelled auth-lock waiter")
            .await?;

        delayed_start.release();
        let owner = tasks.join(owner, "auth transition owner").await?;
        assert!(matches!(owner, Ok(Some(_))));

        let next = resolve_discogs_auth_transition_for_test(&server).await;
        assert!(matches!(next, Ok(Some(_))));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(&mut tasks, outcome, "cancelled waiter scenario").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discogs_auth_rejected_token_compare_clear_and_searches_run_outside_lock() {
    let mut concurrent_search_tasks = DiscogsAuthTasks::default();
    let concurrent_searches = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "concurrent-search-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let first_server = server.clone();
        let first = concurrent_search_tasks.spawn(async move {
            lookup_discogs_remote(&first_server, "Search Artist", "First Search", None).await
        });
        delayed_search
            .wait_until_entered("first ordinary Discogs search")
            .await?;

        let second_server = server.clone();
        let second = concurrent_search_tasks.spawn(async move {
            lookup_discogs_remote(&second_server, "Search Artist", "Second Search", None).await
        });
        fixture
            .state
            .wait_for_count(
                DiscogsBrokerEndpoint::Search,
                2,
                "second ordinary Discogs search while the first is delayed",
            )
            .await?;
        delayed_search.release();

        let first = concurrent_search_tasks
            .join(first, "first ordinary Discogs search")
            .await?;
        let second = concurrent_search_tasks
            .join(second, "second ordinary Discogs search")
            .await?;
        assert!(matches!(first, Ok(None)));
        assert!(matches!(second, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.clear_count(), 0);
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, concurrent_searches).await;
    finish_discogs_auth_scenario(
        &mut concurrent_search_tasks,
        outcome,
        "ordinary searches outside the auth lock",
    )
    .await;

    let mut replacement_tasks = DiscogsAuthTasks::default();
    let replacement_survives = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "rejected-old-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        fixture.state.reject_token("rejected-old-session-fixture");
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let lookup_server = server.clone();
        let lookup = replacement_tasks.spawn(async move {
            lookup_discogs_remote(&lookup_server, "Race Artist", "Replacement Wins", None).await
        });
        delayed_search
            .wait_until_entered("rejected old-token search")
            .await?;
        persistence.set_session(
            &fixture.base_url,
            "new-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        delayed_search.release();

        let result = replacement_tasks
            .join(lookup, "replacement-token retry")
            .await?;
        assert!(matches!(result, Ok(None)));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(persistence.clear_count(), 0);
        assert!(persistence.session_matches(&fixture.base_url, "new-session-fixture"));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, replacement_survives).await;
    finish_discogs_auth_scenario(
        &mut replacement_tasks,
        outcome,
        "new persisted session surviving stale rejection",
    )
    .await;

    let mut bounded_retry_tasks = DiscogsAuthTasks::default();
    let retry_is_bounded = async {
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        persistence.set_session(
            &fixture.base_url,
            "first-rejected-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        fixture.state.reject_token("first-rejected-session-fixture");
        fixture
            .state
            .reject_token("second-rejected-session-fixture");
        let server = ReklawdboxServer::new(None);
        install_discogs_auth_test_dependencies(&server, &fixture, Arc::clone(&persistence), None);
        let mut delayed_search = fixture.state.delay_next(DiscogsBrokerEndpoint::Search);

        let lookup_server = server.clone();
        let lookup = bounded_retry_tasks.spawn(async move {
            lookup_discogs_remote(&lookup_server, "Race Artist", "Bounded Retry", None).await
        });
        delayed_search
            .wait_until_entered("first rejected search in bounded retry")
            .await?;
        persistence.set_session(
            &fixture.base_url,
            "second-rejected-session-fixture",
            DISCOGS_AUTH_TEST_NOW + 3_600,
        );
        delayed_search.release();

        let result = bounded_retry_tasks
            .join(lookup, "bounded rejected-token retry")
            .await?;
        assert!(matches!(
            result,
            Err(crate::adapters::providers::discogs::LookupError::AuthRequired(_))
        ));
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Search), 2);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 0);
        assert_eq!(persistence.clear_count(), 1);
        assert!(!persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };
    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, retry_is_bounded).await;
    finish_discogs_auth_scenario(
        &mut bounded_retry_tasks,
        outcome,
        "rejected-token recovery bounded to one retry",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enrich_tracks_discogs_auth_concurrency_starts_one_session_without_cache_writes() {
    let mut batch_tasks = DiscogsAuthTasks::default();
    let scenario = async {
        let db_conn = create_single_track_test_db(
            "discogs-auth-batch-one",
            "/tmp/discogs-auth-batch-one.flac",
        );
        insert_test_track(
            &db_conn,
            "discogs-auth-batch-two",
            "Concurrent Auth Two",
            "g1",
            "/tmp/discogs-auth-batch-two.flac",
        );
        let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
        let fixture = DiscogsBrokerFixture::start("pending").await;
        let persistence = Arc::new(InMemoryDiscogsSessionPersistence::default());
        install_discogs_auth_test_dependencies(
            &server,
            &fixture,
            Arc::clone(&persistence),
            Some(Arc::new(tokio::sync::Barrier::new(2))),
        );
        let mut delayed_start = fixture.state.delay_next(DiscogsBrokerEndpoint::Start);

        let batch_server = server.clone();
        let batch = batch_tasks.spawn(async move {
            batch_server
                .enrich_tracks(Parameters(EnrichTracksParams {
                    filters: SearchFilterParams::default(),
                    track_ids: Some(vec![
                        "discogs-auth-batch-one".to_string(),
                        "discogs-auth-batch-two".to_string(),
                    ]),
                    playlist_id: None,
                    max_tracks: Some(2),
                    offset: None,
                    providers: Some(vec![
                        crate::application::enrichment::model::EnrichmentProvider::Discogs,
                    ]),
                    skip_cached: Some(false),
                    force_refresh: Some(true),
                    concurrency: Some(2),
                }))
                .await
        });

        delayed_start
            .wait_until_entered("concurrent enrichment device-session start")
            .await?;
        delayed_start.release();
        let result = batch_tasks
            .join(batch, "concurrent Discogs enrichment batch")
            .await
            .map_err(|error| format!("concurrent Discogs enrichment task failed: {error}"))?
            .map_err(|error| {
                format!("concurrent Discogs enrichment should return a batch payload: {error:?}")
            })?;
        let payload = extract_json(&result);

        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Start), 1);
        assert_eq!(fixture.state.count(DiscogsBrokerEndpoint::Status), 1);
        assert_eq!(payload["summary"]["tracks_total"], 2);
        assert_eq!(payload["summary"]["total"], 2);
        assert_eq!(payload["summary"]["enriched"], 0);
        assert_eq!(payload["summary"]["cached"], 0);
        assert_eq!(payload["summary"]["skipped"], 0);
        assert_eq!(payload["summary"]["failed"], 2);
        assert_cache_write_summary(&payload, 0, 0, 0);

        let failures = payload["failures"]
            .as_array()
            .expect("concurrent Discogs failures should be an array");
        assert_eq!(failures.len(), 2);
        let errors = failures
            .iter()
            .map(|failure| {
                assert_eq!(failure["provider"], "discogs");
                failure["error"]
                    .as_str()
                    .expect("Discogs auth failure should include an error")
            })
            .collect::<Vec<_>>();
        assert!(
            errors
                .iter()
                .all(|error| error.contains("Auth URL: https://auth.example/device"))
        );
        assert_eq!(errors[0], errors[1]);

        let serialized = payload.to_string();
        assert!(!serialized.contains("pending-test-value"));
        assert!(!serialized.contains("session-test-value"));
        assert!(!persistence.has_session(&fixture.base_url));
        fixture.shutdown().await;
        Ok::<(), String>(())
    };

    let outcome = tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, scenario).await;
    finish_discogs_auth_scenario(
        &mut batch_tasks,
        outcome,
        "concurrent Discogs enrichment scenario",
    )
    .await;
}

#[test]
fn pending_batch_page_explicit_ids_keep_caller_order_and_apply_cap() {
    let conn = create_selector_pagination_test_db();
    let ids = vec![
        "t3".to_string(),
        "t1".to_string(),
        "t1".to_string(),
        "t2".to_string(),
    ];
    let selection = resolve_pending_tracks(
        &conn,
        Some(&ids),
        None,
        SearchFilterParams::default(),
        Some(10),
        Some(0),
        50,
        2,
        false,
        |tracks| Ok(vec![false; tracks.len()]),
    )
    .expect("pending explicit-ID selector should resolve");

    assert_eq!(track_ids(&selection.selected), ["t3", "t1"]);
    assert_eq!(
        selection.page,
        BatchPage {
            matched_tracks: 3,
            start_offset: 0,
            examined_tracks: 2,
            selected_tracks: 2,
            fully_cached_skipped: 0,
            next_offset: Some(2),
            has_more: true,
        }
    );
}

#[test]
fn lookup_output_wraps_non_object_in_result_envelope() {
    let output = lookup_output_with_cache_metadata(serde_json::Value::Null, false, None);
    assert_eq!(output["result"], serde_json::Value::Null);
    assert_eq!(output["cache_hit"], false);
    assert!(
        output.get("cached_at").is_none(),
        "live payload should not include cached_at"
    );
}

#[test]
fn lookup_output_with_cache_metadata_keeps_object_payload_shape() {
    let output = lookup_output_with_cache_metadata(
        serde_json::json!({
            "genre": "Techno"
        }),
        true,
        Some("2026-02-20T10:00:00Z"),
    );
    assert_eq!(output["genre"], "Techno");
    assert_eq!(output["cache_hit"], true);
    assert_eq!(output["cached_at"], "2026-02-20T10:00:00Z");
    assert!(
        output.get("result").is_none(),
        "object payloads should not be wrapped in a result envelope"
    );
}

#[test]
fn auth_remediation_message_marks_discogs_auth_as_agent_actionable() {
    let remediation = crate::adapters::providers::discogs::AuthRemediation {
        message: "Discogs auth required (not a lookup miss).".to_string(),
        auth_url: Some("https://discogs.example/auth/device".to_string()),
        poll_interval_seconds: Some(5),
        expires_at: Some(1_777_000_000),
    };

    let message = auth_remediation_message(&remediation);

    assert!(message.contains("not a lookup miss"));
    assert!(message.contains("Auth URL: https://discogs.example/auth/device"));
    assert!(message.contains("open 'https://discogs.example/auth/device'"));
    assert!(message.contains("Poll interval if polling instead of browser: 5s"));
    assert!(message.contains("Auth session expires_at (unix): 1777000000"));
}

#[test]
fn enrich_tracks_invalid_provider_rejected_by_serde() {
    let json = serde_json::json!({
        "providers": ["spotify"],
    });
    let result = serde_json::from_value::<EnrichTracksParams>(json);
    assert!(
        result.is_err(),
        "serde should reject unknown provider variant"
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_persists_no_match_before_skipped() {
    let db_conn = create_single_track_test_db("ack-no-match", "/tmp/ack-no-match.flac");
    let title = "Ack No Match";
    set_enrich_test_track_title(&db_conn, "ack-no-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-no-match"],
        "acknowledged no-match enrichment",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 1);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 1, 1, 0);

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry = store::get_enrichment(
        &conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("skipped no-match should have a durable negative cache row");
    assert_eq!(entry.match_quality.as_deref(), Some("none"));
    assert!(entry.response_json.is_none());
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_failed_no_match_counts_only_failed() {
    let db_conn = create_single_track_test_db("ack-failed-no-match", "/tmp/ack-failed.flac");
    let title = "Ack Failed No Match";
    set_enrich_test_track_title(&db_conn, "ack-failed-no-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    install_enrichment_insert_failure(&server, title);
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-failed-no-match"],
        "failed no-match cache write",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 1, 0, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-failed-no-match",
        title,
        "cache write failed:",
    );

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &norm_title,
            Some(&norm_album),
            false,
        )
        .expect("cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_open_failure_is_per_attempt() {
    let db_conn = create_single_track_test_db("ack-open-failure", "/tmp/ack-open-failure.flac");
    let title = "Ack Writer Open Failure";
    set_enrich_test_track_title(&db_conn, "ack-open-failure", title);
    let initialized_store_dir = tempfile::tempdir().expect("initialized store dir should create");
    let initialized_store_path = initialized_store_dir.path().join("internal.sqlite3");
    let initialized_store_path_str = initialized_store_path.to_string_lossy().to_string();
    let store_conn =
        store::open(&initialized_store_path_str).expect("initialized store should open");
    let writer_directory = tempfile::tempdir().expect("writer directory should create");
    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(writer_directory.path().to_string_lossy().to_string()),
    );
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result =
        run_discogs_batch_with_timeout(&server, &["ack-open-failure"], "writer-open cache failure")
            .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 1, 0, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-open-failure",
        title,
        "cache writer open failed:",
    );

    let conn =
        store::open(&initialized_store_path_str).expect("initialized store should remain readable");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &norm_title,
            Some(&norm_album),
            false,
        )
        .expect("cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_mixed_success_and_failure_are_exact() {
    let db_conn = create_single_track_test_db("ack-mixed-success", "/tmp/ack-mixed-success.flac");
    let successful_raw_title = "Ack Mixed Success";
    set_enrich_test_track_title(&db_conn, "ack-mixed-success", successful_raw_title);
    insert_test_track(
        &db_conn,
        "ack-mixed-failure",
        "Matched But Uncached",
        "g1",
        "/tmp/ack-mixed-failure.flac",
    );
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    install_enrichment_insert_failure(&server, "Matched But Uncached");
    set_test_discogs_lookup_override(
        "Aníbal",
        successful_raw_title,
        Some("Encoded Paths"),
        Ok(None),
    );
    set_test_discogs_lookup_override(
        "Aníbal",
        "Matched But Uncached",
        Some("Encoded Paths"),
        Ok(Some(discogs_match("Matched But Uncached"))),
    );

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-mixed-success", "ack-mixed-failure"],
        "mixed cache-write outcomes",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 1);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 2, 1, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-mixed-failure",
        "Matched But Uncached",
        "cache write failed:",
    );

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let success_title = crate::domain::metadata::normalize_for_matching(successful_raw_title);
    let failed_title = crate::domain::metadata::normalize_for_matching("Matched But Uncached");
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &success_title,
            Some(&norm_album),
            false,
        )
        .expect("successful cache read should succeed")
        .is_some()
    );
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &failed_title,
            Some(&norm_album),
            false,
        )
        .expect("failed cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_persists_match_before_enriched() {
    let db_conn = create_single_track_test_db("ack-match", "/tmp/ack-match.flac");
    let title = "Ack Persisted Match";
    set_enrich_test_track_title(&db_conn, "ack-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_discogs_lookup_override(
        "Aníbal",
        title,
        Some("Encoded Paths"),
        Ok(Some(discogs_match(title))),
    );

    let result =
        run_discogs_batch_with_timeout(&server, &["ack-match"], "acknowledged match enrichment")
            .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 1);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 1, 1, 0);

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry = store::get_enrichment(
        &conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("enriched match should have a durable cache row");
    assert_eq!(entry.match_quality.as_deref(), Some("exact"));
    assert!(entry.response_json.is_some());
}

#[tokio::test]
async fn lookup_discogs_no_match_payload_is_consistent_across_live_and_cache_paths() {
    let db_conn =
        create_single_track_test_db("discogs-no-match-track", "/tmp/discogs-no-match.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let artist = "Discogs NoMatch Artist";
    let title = "Discogs NoMatch Title";
    set_test_discogs_lookup_override(artist, title, None, Ok(None));

    let live_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(true),
        }))
        .await
        .expect("live discogs no-match should succeed");
    let live_payload = extract_json(&live_result);
    assert_eq!(live_payload["result"], serde_json::Value::Null);
    assert_eq!(live_payload["cache_hit"], false);
    assert!(
        live_payload.get("cached_at").is_none(),
        "live payload should omit cached_at"
    );

    let cache_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(false),
        }))
        .await
        .expect("cached discogs no-match should succeed");
    let cache_payload = extract_json(&cache_result);
    assert_eq!(cache_payload["result"], serde_json::Value::Null);
    assert_eq!(cache_payload["cache_hit"], true);

    let cache_hit_timestamp = cache_payload
        .get("cached_at")
        .and_then(serde_json::Value::as_str)
        .expect("cached no-match payload should include cached_at");
    let norm_artist = crate::domain::metadata::normalize_for_matching(artist);
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let cache_entry = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "discogs", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("discogs no-match lookup should create cache entry")
    };
    assert!(
        cache_entry.response_json.is_none(),
        "discogs no-match cache entry should store null response as no payload"
    );
    assert_eq!(
        cache_hit_timestamp,
        cache_entry.created_at.as_str(),
        "cached_at should match persisted cache timestamp"
    );
}

#[tokio::test]
async fn enrich_tracks_discogs_skip_cached_reports_cached_counts() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                    12700, 153, 'cached batch test', 2025, 230, ?3, '0', 1411,
                    44100, 5, '2025-01-01', 0
                )",
            params![
                "cached-track-2",
                "Corazón Cached",
                "/tmp/cached-track-2.flac"
            ],
        )
        .expect("second test track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let artist = "Aníbal";
    let title_one = "Señorita";
    let title_two = "Corazón Cached";
    let norm_artist = crate::domain::metadata::normalize_for_matching(artist);
    let norm_title_one = crate::domain::metadata::normalize_for_matching(title_one);
    let norm_title_two = crate::domain::metadata::normalize_for_matching(title_two);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");

    let cached_one = serde_json::json!({
        "title": "Anibal - Senorita",
        "genres": ["Electronic"],
        "styles": ["Deep House"],
        "fuzzy_match": false
    })
    .to_string();
    let cached_two = serde_json::json!({
        "title": "Anibal - Corazon Cached",
        "genres": ["Electronic"],
        "styles": ["House"],
        "fuzzy_match": false
    })
    .to_string();

    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_one),
    )
    .expect("first sentinel discogs cache entry should write");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_two),
    )
    .expect("second sentinel discogs cache entry should write");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );

    let params = EnrichTracksParams {
        filters: SearchFilterParams::default(),
        track_ids: Some(vec![
            "cached-track-1".to_string(),
            "cached-track-2".to_string(),
        ]),
        playlist_id: None,
        max_tracks: Some(10),
        offset: None,
        providers: Some(vec![
            crate::application::enrichment::model::EnrichmentProvider::Discogs,
        ]),
        skip_cached: Some(true),
        force_refresh: Some(false),
        concurrency: None,
    };

    let first_result = server
        .enrich_tracks(Parameters(params))
        .await
        .expect("enrich_tracks should succeed when everything is cached");
    let first_payload = extract_json(&first_result);
    assert_eq!(first_payload["summary"]["tracks_total"], 2);
    assert_eq!(first_payload["summary"]["total"], 2);
    assert_eq!(first_payload["summary"]["enriched"], 0);
    assert_eq!(first_payload["summary"]["cached"], 2);
    assert_eq!(first_payload["summary"]["skipped"], 0);
    assert_eq!(first_payload["summary"]["failed"], 0);
    assert_cache_write_summary(&first_payload, 0, 0, 0);
    assert_eq!(first_payload["page"]["matched_tracks"], 2);
    assert_eq!(first_payload["page"]["examined_tracks"], 2);
    assert_eq!(first_payload["page"]["selected_tracks"], 0);
    assert_eq!(first_payload["page"]["fully_cached_skipped"], 2);
    assert_eq!(
        first_payload["page"]["next_offset"],
        serde_json::Value::Null
    );
    assert_eq!(first_payload["page"]["has_more"], false);
    assert_eq!(
        first_payload["failures"]
            .as_array()
            .expect("failures should be an array")
            .len(),
        0
    );

    let second_result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "cached-track-1".to_string(),
                "cached-track-2".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(10),
            offset: None,
            providers: Some(vec![
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
            ]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("second enrich_tracks run should also be fully cached");
    let second_payload = extract_json(&second_result);
    assert_eq!(second_payload["summary"]["tracks_total"], 2);
    assert_eq!(second_payload["summary"]["total"], 2);
    assert_eq!(second_payload["summary"]["enriched"], 0);
    assert_eq!(second_payload["summary"]["cached"], 2);
    assert_eq!(second_payload["summary"]["skipped"], 0);
    assert_eq!(second_payload["summary"]["failed"], 0);
    assert_cache_write_summary(&second_payload, 0, 0, 0);
    assert_eq!(second_payload["page"]["fully_cached_skipped"], 2);

    let store = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry_one = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("first cache entry should still exist");
    let entry_two = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("second cache entry should still exist");
    assert_eq!(
        entry_one.response_json.as_deref(),
        Some(cached_one.as_str())
    );
    assert_eq!(
        entry_two.response_json.as_deref(),
        Some(cached_two.as_str())
    );
}

#[tokio::test]
async fn enrich_tracks_summary_uses_provider_attempt_totals() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching("Señorita");
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .expect("discogs cache should seed");
    store::set_enrichment(
        &store_conn,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        Some("exact"),
        Some(r#"{"genre":"Deep House"}"#),
    )
    .expect("bandcamp cache should seed");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );
    let result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["cached-track-1".to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            offset: None,
            providers: Some(vec![
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("enrich_tracks should resolve from cache for both providers");
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["tracks_total"], 1);
    assert_eq!(payload["summary"]["total"], 2);
    assert_eq!(payload["summary"]["cached"], 2);
    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 0, 0, 0);
    assert_eq!(payload["page"]["matched_tracks"], 1);
    assert_eq!(payload["page"]["fully_cached_skipped"], 1);
}

#[tokio::test]
async fn resolve_track_data_uses_decoded_path_for_audio_cache_lookup() {
    let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let decoded_path = temp_audio_dir.path().join("Aníbal Track.flac");
    std::fs::write(&decoded_path, b"fake-audio-data")
        .expect("decoded path file should exist for resolve_file_path");
    let metadata = std::fs::metadata(&decoded_path).expect("decoded path metadata should load");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let decoded_path_str = decoded_path.to_string_lossy().to_string();
    let raw_path = decoded_path_str
        .replace("Aníbal", "An%C3%ADbal")
        .replace(' ', "%20");
    assert_ne!(
        raw_path, decoded_path_str,
        "raw path must differ from decoded path for this regression test"
    );
    assert!(
        std::fs::metadata(&decoded_path_str).is_ok(),
        "decoded file path should exist"
    );
    assert!(
        std::fs::metadata(&raw_path).is_err(),
        "raw encoded path should not exist"
    );

    let db_conn = create_single_track_test_db("encoded-track-1", &raw_path);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &decoded_path_str,
        "stratum-dsp",
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"4"}"#,
    )
    .expect("audio cache entry should write with decoded cache key");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: "encoded-track-1".to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let payload = extract_json(&result);

    assert_eq!(
        payload["data_completeness"]["stratum_dsp"], true,
        "decoded path lookup should find stratum cache entry"
    );
    assert!(
        payload["audio_analysis"]["stratum_dsp"].is_object(),
        "audio_analysis.stratum_dsp should be populated from cache"
    );
    assert_eq!(payload["audio_analysis"]["stratum_dsp"]["key"], "Am");
}

#[tokio::test]
async fn resolve_track_data_audio_cache_ignores_existing_file_stale_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("resolve-stale-single.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1000);
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("resolve-stale-single", &audio_path_str);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &audio_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime + 1,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"18"}"#,
    )
    .expect("stale-mtime stratum cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: "resolve-stale-single".to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let payload = extract_json(&result);

    assert_eq!(
        payload["data_completeness"]["stratum_dsp"], false,
        "current-schema row with stale file mtime must not count"
    );
    assert!(
        payload["audio_analysis"]["stratum_dsp"].is_null(),
        "stale stratum row must be excluded from single-track resolve payload"
    );
}

#[tokio::test]
async fn resolve_tracks_data_audio_cache_ignores_stale_file_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let fresh_path = audio_dir.path().join("resolve-fresh.flac");
    let stale_path = audio_dir.path().join("resolve-stale.flac");
    let (fresh_size, fresh_mtime) = write_test_audio_file(&fresh_path, 1000);
    let (stale_size, stale_mtime) = write_test_audio_file(&stale_path, 1001);
    let fresh_path_str = fresh_path.to_string_lossy().to_string();
    let stale_path_str = stale_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("resolve-fresh", &fresh_path_str);
    insert_test_track(
        &db_conn,
        "resolve-stale",
        "Resolve Stale",
        "g1",
        &stale_path_str,
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"18"}"#,
    )
    .expect("fresh stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":1.5}"#,
    )
    .expect("fresh essentia cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &stale_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        stale_size + 1,
        stale_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":140.0,"key":"Cm","analyzer_version":"18"}"#,
    )
    .expect("stale-size stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &stale_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        stale_size,
        stale_mtime + 1,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":2.5}"#,
    )
    .expect("stale-mtime essentia cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "resolve-fresh".to_string(),
                "resolve-stale".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(2),
            format: None,
        }))
        .await
        .expect("resolve_tracks_data should succeed");
    let payload = extract_json(&result);
    let items = payload
        .as_array()
        .expect("batch resolve should return array");
    let fresh = items
        .iter()
        .find(|item| item["track_id"] == "resolve-fresh")
        .expect("fresh track should be present");
    let stale = items
        .iter()
        .find(|item| item["track_id"] == "resolve-stale")
        .expect("stale track should be present");

    assert_eq!(fresh["data_completeness"]["stratum_dsp"], true);
    assert!(fresh["audio_analysis"]["stratum_dsp"].is_object());
    assert_eq!(fresh["data_completeness"]["essentia"], true);
    assert!(fresh["audio_analysis"]["essentia"].is_object());

    assert_eq!(
        stale["data_completeness"]["stratum_dsp"], false,
        "current-schema stratum row with stale file size must not count"
    );
    assert!(
        stale["audio_analysis"]["stratum_dsp"].is_null(),
        "stale stratum row must be excluded from batch resolve payload"
    );
    assert_eq!(
        stale["data_completeness"]["essentia"], false,
        "current-schema essentia row with stale file mtime must not count"
    );
    assert!(
        stale["audio_analysis"]["essentia"].is_null(),
        "stale essentia row must be excluded from batch resolve payload"
    );
}

#[tokio::test]
#[ignore]
async fn resolve_tracks_data_batch_consistency() {
    let Some((server, _store_dir)) =
        create_real_server_with_temp_store(default_http_client_for_tests())
    else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let tracks = sample_real_tracks(&server, 5);
    assert!(
        !tracks.is_empty(),
        "integration test needs tracks from real DB"
    );
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();

    let batch_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(track_ids.clone()),
            playlist_id: None,
            max_tracks: Some(track_ids.len() as u32),
            format: None,
        }))
        .await
        .expect("batch resolve should succeed");
    let batch_payload = extract_json(&batch_result);
    let batch_items = batch_payload
        .as_array()
        .expect("batch resolve payload should be an array");
    assert_eq!(
        batch_items.len(),
        track_ids.len(),
        "batch resolve should return one entry per requested track"
    );

    let mut by_track_id: HashMap<String, serde_json::Value> = HashMap::new();
    for item in batch_items {
        let track_id = item
            .get("track_id")
            .and_then(serde_json::Value::as_str)
            .expect("resolved track item should include track_id");
        by_track_id.insert(track_id.to_string(), item.clone());
    }

    for track_id in &track_ids {
        let single_result = server
            .resolve_track_data(Parameters(ResolveTrackDataParams {
                track_id: track_id.clone(),
            }))
            .await
            .expect("single resolve should succeed");
        let single_payload = extract_json(&single_result);
        assert_eq!(
            by_track_id
                .get(track_id)
                .expect("batch output should include every requested track"),
            &single_payload,
            "batch resolve output should match single-track resolve output"
        );
    }
}

#[test]
fn resolve_single_track_rekordbox_only() {
    let track = make_test_track("t1", "Deep House", 126.0, "Am");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let rb = result
        .get("rekordbox")
        .expect("rekordbox section should exist");
    assert_eq!(rb["title"], "Track t1");
    assert_eq!(rb["artist"], "Test Artist");
    assert_eq!(rb["genre"], "Deep House");
    assert_eq!(rb["bpm"], 126.0);
    assert_eq!(rb["key"], "Am");
    assert_eq!(rb["duration_s"], 300);
    assert_eq!(rb["year"], 2023);
    assert_eq!(rb["rating"], 3);
    assert_eq!(rb["label"], "Test Label");

    assert!(
        result["audio_analysis"].is_null(),
        "audio_analysis should be null without cache"
    );
    assert!(
        result["discogs"].is_null(),
        "discogs should be null without cache"
    );
    assert!(
        result["staged_changes"].is_null(),
        "staged_changes should be null without staged"
    );

    let dc = result
        .get("data_completeness")
        .expect("data_completeness should exist");
    assert_eq!(dc["rekordbox"], true);
    assert_eq!(dc["stratum_dsp"], false);
    assert_eq!(dc["essentia"], false);
    assert_eq!(dc["essentia_installed"], false);
    assert_eq!(dc["discogs"], false);

    let gt = result
        .get("genre_taxonomy")
        .expect("genre_taxonomy should exist");
    assert_eq!(gt["current_genre_canonical"], "Deep House");
}

#[test]
fn resolve_single_track_with_staged_changes() {
    let track = make_test_track("t2", "House", 128.0, "Cm");
    let staged = crate::domain::metadata::TrackChange {
        track_id: "t2".to_string(),
        genre: Some("Deep House".to_string()),
        comments: None,
        rating: Some(5),
        color: None,
        label: None,
        year: None,
        album: None,
    };
    let result = resolve_single_track(&track, None, None, None, false, Some(&staged));

    let sc = result
        .get("staged_changes")
        .expect("staged_changes should exist");
    assert!(
        !sc.is_null(),
        "staged_changes should not be null when changes are staged"
    );
    assert_eq!(sc["genre"], "Deep House");
    assert!(sc["comments"].is_null(), "unstaged field should be null");
    assert_eq!(sc["rating"], 5);
    assert!(sc["color"].is_null(), "unstaged field should be null");
    assert!(
        sc.get("year").is_some_and(serde_json::Value::is_null),
        "unstaged year should be present and null"
    );
    assert!(
        sc.get("album").is_some_and(serde_json::Value::is_null),
        "unstaged album should be present and null"
    );
}

#[tokio::test]
async fn resolve_tools_return_all_staged_fields_in_full_format() {
    let track_id = "resolve-all-staged-fields";
    let db_conn = create_single_track_test_db(track_id, "/tmp/resolve-all-staged-fields.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let staged = TrackChange {
        track_id: track_id.to_string(),
        genre: Some("Techno".to_string()),
        comments: Some("staged comments".to_string()),
        rating: Some(5),
        color: Some("Red".to_string()),
        label: Some("Staged Label".to_string()),
        year: Some(1997),
        album: Some("Staged Album".to_string()),
    };
    assert_eq!(server.context.mutation.changes.stage(vec![staged]), (1, 1));

    let expected_staged = serde_json::json!({
        "genre": "Techno",
        "comments": "staged comments",
        "rating": 5,
        "color": "Red",
        "label": "Staged Label",
        "year": 1997,
        "album": "Staged Album",
    });
    let expected_keys: HashSet<&str> = EditableField::ALL
        .iter()
        .map(EditableField::as_str)
        .collect();

    let single_result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: track_id.to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let single_payload = extract_json(&single_result);
    let single_staged = single_payload
        .get("staged_changes")
        .expect("resolve_track_data should include staged_changes");
    assert_eq!(single_staged, &expected_staged);
    let single_keys: HashSet<&str> = single_staged
        .as_object()
        .expect("staged_changes should be an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(single_keys, expected_keys);

    let batch_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![track_id.to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            format: Some(ResolveFormat::Full),
        }))
        .await
        .expect("full resolve_tracks_data should succeed");
    let batch_payload = extract_json(&batch_result);
    let batch_item = batch_payload
        .as_array()
        .and_then(|items| items.first())
        .expect("full resolve_tracks_data should return the requested track");
    let batch_staged = batch_item
        .get("staged_changes")
        .expect("full resolve_tracks_data should include staged_changes");
    assert_eq!(batch_staged, &expected_staged);
    assert_eq!(batch_staged, single_staged);

    let compact_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![track_id.to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            format: Some(ResolveFormat::Classification),
        }))
        .await
        .expect("classification resolve_tracks_data should succeed");
    let compact_payload = extract_json(&compact_result);
    let compact_item = compact_payload
        .as_array()
        .and_then(|items| items.first())
        .expect("classification resolve_tracks_data should return the requested track");
    assert_eq!(
        compact_item.get("track_id"),
        Some(&serde_json::json!(track_id))
    );
    assert!(compact_item.get("artist").is_some());
    assert!(compact_item.get("audio").is_some());
    assert!(compact_item.get("rekordbox").is_none());
    assert!(compact_item.get("staged_changes").is_none());
}

#[test]
fn resolve_single_track_taxonomy_mappings() {
    let track = make_test_track("t3", "Hip-Hop", 130.0, "Fm");

    let discogs_json = serde_json::json!({
        "title": "Some Release",
        "year": "2020",
        "label": "Some Label",
        "genres": ["Electronic"],
        "styles": ["Deep House", "Garage House", "Some Unknown Style"],
        "fuzzy_match": false,
    });
    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t3".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("exact".to_string()),
        response_json: Some(serde_json::to_string(&discogs_json).unwrap()),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, Some(&discogs_cache), None, None, false, None);

    let dc = &result["data_completeness"];
    assert_eq!(dc["discogs"], true);
    assert_eq!(dc["stratum_dsp"], false);

    let gt = &result["genre_taxonomy"];
    assert_eq!(gt["current_genre_canonical"], "Hip Hop");

    let dsm = gt["discogs_style_mappings"]
        .as_array()
        .expect("should be array");
    assert_eq!(dsm.len(), 3);

    let dh = dsm
        .iter()
        .find(|m| m["style"] == "Deep House")
        .expect("Deep House mapping");
    assert_eq!(dh["mapping_type"], "exact");
    assert_eq!(dh["maps_to"], "Deep House");

    let gh = dsm
        .iter()
        .find(|m| m["style"] == "Garage House")
        .expect("Garage House mapping");
    assert_eq!(gh["mapping_type"], "alias");
    assert_eq!(gh["maps_to"], "House");

    let unknown = dsm
        .iter()
        .find(|m| m["style"] == "Some Unknown Style")
        .expect("unknown mapping");
    assert_eq!(unknown["mapping_type"], "unknown");
    assert!(unknown["maps_to"].is_null());

    assert!(
        result["discogs"].is_object(),
        "discogs should be parsed object"
    );
}

#[test]
fn resolve_single_track_empty_genre_is_null() {
    let track = make_test_track("t4", "", 0.0, "");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "empty genre should map to null canonical"
    );
}

#[test]
fn resolve_single_track_unknown_genre_maps_to_null() {
    let track = make_test_track("t5", "Polka", 120.0, "C");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "unknown genre 'Polka' should map to null"
    );
}

#[test]
fn resolve_single_track_with_stratum_agreement() {
    let track = make_test_track("t6", "Techno", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 128.5,
        "key": "Am",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        input_fingerprint: "hmm:v1".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, Some(&stratum_cache), None, false, None);

    let aa = result
        .get("audio_analysis")
        .expect("audio_analysis should exist");
    assert!(
        !aa.is_null(),
        "audio_analysis should not be null with stratum cache"
    );
    assert_eq!(
        aa["bpm_agreement"], true,
        "BPM 128.0 vs 128.5 should agree (within 2.0)"
    );
    assert_eq!(aa["key_agreement"], true, "Key Am vs Am should agree");
    assert!(
        aa["stratum_dsp"].is_object(),
        "stratum_dsp should be the parsed features"
    );
    assert!(
        aa["essentia"].is_null(),
        "essentia should be null when not cached"
    );

    let dc = &result["data_completeness"];
    assert_eq!(dc["stratum_dsp"], true);
}

#[test]
fn resolve_single_track_with_essentia_cache() {
    let track = make_test_track("t6b", "Techno", 128.0, "Am");
    let essentia_json = serde_json::json!({
        "danceability": 0.82,
        "loudness_integrated": -8.4,
        "rhythm_regularity": 0.91,
        "analyzer_version": "2.1b6.dev1389"
    });
    let essentia_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "essentia".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "2.1b6.dev1389".to_string(),
        input_fingerprint: String::new(),
        features_json: serde_json::to_string(&essentia_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, None, Some(&essentia_cache), true, None);

    let aa = &result["audio_analysis"];
    assert!(
        aa.is_object(),
        "audio_analysis should be populated when essentia cache exists"
    );
    assert!(
        aa["stratum_dsp"].is_null(),
        "stratum_dsp should remain null when not cached"
    );
    assert!(
        aa["essentia"].is_object(),
        "essentia should expose cached analysis JSON"
    );
    assert_eq!(aa["essentia"]["danceability"], 0.82);

    let dc = &result["data_completeness"];
    assert_eq!(dc["essentia"], true);
    assert_eq!(dc["essentia_installed"], true);
}

#[test]
fn resolve_single_track_stratum_disagreement() {
    let track = make_test_track("t7", "House", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 64.0,
        "key": "Cm",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        input_fingerprint: "hmm:v1".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, Some(&stratum_cache), None, false, None);

    let aa = &result["audio_analysis"];
    assert_eq!(
        aa["bpm_agreement"], false,
        "BPM 128.0 vs 64.0 should disagree"
    );
    assert_eq!(aa["key_agreement"], false, "Key Am vs Cm should disagree");
}

#[test]
fn resolve_single_track_enrichment_no_match_returns_null() {
    let track = make_test_track("t8", "House", 126.0, "Am");

    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t8".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("none".to_string()),
        response_json: None,
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, Some(&discogs_cache), None, None, false, None);

    assert!(
        result["discogs"].is_null(),
        "discogs with no response_json should be null"
    );
    assert_eq!(
        result["data_completeness"]["discogs"], true,
        "cache entry exists so completeness is true"
    );
}
