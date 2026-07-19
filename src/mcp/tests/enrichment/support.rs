use super::*;

pub(super) const DISCOGS_AUTH_TEST_NOW: i64 = 2_000_000_000;

pub(super) const DISCOGS_AUTH_TEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DiscogsBrokerEndpoint {
    Start,
    Status,
    Finalize,
    Search,
}

#[derive(Clone)]
pub(super) struct DiscogsBrokerFailure {
    pub(super) status: String,
    pub(super) body: String,
}

pub(super) struct DiscogsBrokerDelay {
    pub(super) endpoint: DiscogsBrokerEndpoint,
    pub(super) entered: Option<tokio::sync::oneshot::Sender<()>>,
    pub(super) release: Option<tokio::sync::oneshot::Receiver<()>>,
}

pub(super) struct DiscogsBrokerDelayControl {
    pub(super) entered: tokio::sync::oneshot::Receiver<()>,
    pub(super) release: Option<tokio::sync::oneshot::Sender<()>>,
}

impl DiscogsBrokerDelayControl {
    pub(super) async fn wait_until_entered(&mut self, phase: &str) -> Result<(), String> {
        tokio::time::timeout(DISCOGS_AUTH_TEST_TIMEOUT, &mut self.entered)
            .await
            .map_err(|_| format!("{phase} did not reach the delayed broker response"))?
            .map_err(|_| format!("{phase} delay notification was canceled"))
    }

    pub(super) fn release(&mut self) {
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
    }
}

pub(super) struct DiscogsBrokerFixtureState {
    pub(super) starts: AtomicUsize,
    pub(super) statuses: AtomicUsize,
    pub(super) finalizes: AtomicUsize,
    pub(super) searches: AtomicUsize,
    pub(super) request_changed: tokio::sync::Notify,
    pub(super) status: Mutex<String>,
    pub(super) auth_url: Mutex<String>,
    pub(super) rejected_tokens: Mutex<HashSet<String>>,
    pub(super) failures: Mutex<HashMap<DiscogsBrokerEndpoint, DiscogsBrokerFailure>>,
    pub(super) delay: Mutex<Option<DiscogsBrokerDelay>>,
}

impl DiscogsBrokerFixtureState {
    pub(super) fn new(status: &str) -> Self {
        Self {
            starts: AtomicUsize::new(0),
            statuses: AtomicUsize::new(0),
            finalizes: AtomicUsize::new(0),
            searches: AtomicUsize::new(0),
            request_changed: tokio::sync::Notify::new(),
            status: Mutex::new(status.to_string()),
            auth_url: Mutex::new("https://auth.example/device".to_string()),
            rejected_tokens: Mutex::new(HashSet::new()),
            failures: Mutex::new(HashMap::new()),
            delay: Mutex::new(None),
        }
    }

    pub(super) fn count(&self, endpoint: DiscogsBrokerEndpoint) -> usize {
        match endpoint {
            DiscogsBrokerEndpoint::Start => self.starts.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Status => self.statuses.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Finalize => self.finalizes.load(Ordering::SeqCst),
            DiscogsBrokerEndpoint::Search => self.searches.load(Ordering::SeqCst),
        }
    }

    pub(super) fn record(&self, endpoint: DiscogsBrokerEndpoint) {
        match endpoint {
            DiscogsBrokerEndpoint::Start => &self.starts,
            DiscogsBrokerEndpoint::Status => &self.statuses,
            DiscogsBrokerEndpoint::Finalize => &self.finalizes,
            DiscogsBrokerEndpoint::Search => &self.searches,
        }
        .fetch_add(1, Ordering::SeqCst);
        self.request_changed.notify_waiters();
    }

    pub(super) async fn wait_for_count(
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

    pub(super) fn reject_token(&self, token: &str) {
        self.rejected_tokens
            .lock()
            .expect("broker fixture rejected-token mutex should not be poisoned")
            .insert(token.to_string());
    }

    pub(super) fn set_auth_url(&self, auth_url: &str) {
        *self
            .auth_url
            .lock()
            .expect("broker fixture auth URL mutex should not be poisoned") = auth_url.to_string();
    }

    pub(super) fn fail_endpoint(&self, endpoint: DiscogsBrokerEndpoint, body: &str) {
        self.respond_endpoint(endpoint, "500 Internal Server Error", body);
    }

    pub(super) fn respond_endpoint(
        &self,
        endpoint: DiscogsBrokerEndpoint,
        status: &str,
        body: &str,
    ) {
        self.failures
            .lock()
            .expect("broker fixture failure mutex should not be poisoned")
            .insert(
                endpoint,
                DiscogsBrokerFailure {
                    status: status.to_string(),
                    body: body.to_string(),
                },
            );
    }

    pub(super) fn failure(&self, endpoint: DiscogsBrokerEndpoint) -> Option<DiscogsBrokerFailure> {
        self.failures
            .lock()
            .expect("broker fixture failure mutex should not be poisoned")
            .get(&endpoint)
            .cloned()
    }

    pub(super) fn delay_next(&self, endpoint: DiscogsBrokerEndpoint) -> DiscogsBrokerDelayControl {
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

    pub(super) async fn apply_delay(&self, endpoint: DiscogsBrokerEndpoint) -> Result<(), String> {
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

pub(super) struct DiscogsBrokerFixture {
    pub(super) base_url: String,
    pub(super) state: Arc<DiscogsBrokerFixtureState>,
    pub(super) shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    pub(super) task: Option<tokio::task::JoinHandle<Result<(), String>>>,
}

impl DiscogsBrokerFixture {
    pub(super) async fn start(status: &str) -> Self {
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

    pub(super) fn config(&self) -> crate::adapters::providers::discogs::BrokerConfig {
        crate::adapters::providers::discogs::BrokerConfig {
            base_url: self.base_url.clone(),
            broker_token: None,
        }
    }

    pub(super) async fn shutdown(mut self) {
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

pub(super) async fn read_discogs_broker_request(
    stream: &mut tokio::net::TcpStream,
) -> Result<String, String> {
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

pub(super) fn discogs_bearer_token(request: &str) -> Option<&str> {
    request.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.eq_ignore_ascii_case("authorization") {
            value.trim().strip_prefix("Bearer ")
        } else {
            None
        }
    })
}

pub(super) async fn write_discogs_broker_response(
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

pub(super) async fn serve_discogs_broker_connection(
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
            let auth_url = state
                .auth_url
                .lock()
                .map_err(|_| "broker fixture auth URL mutex poisoned".to_string())?
                .clone();
            write_discogs_broker_response(
                &mut stream,
                "200 OK",
                &serde_json::json!({
                    "device_id": "device-test-value",
                    "pending_token": "pending-test-value",
                    "auth_url": auth_url,
                    "poll_interval_seconds": 1,
                    "expires_at": 2_000_003_600_i64,
                })
                .to_string(),
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

pub(super) fn install_discogs_auth_test_dependencies(
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

pub(super) fn discogs_auth_url(
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

pub(super) struct DiscogsAuthTasks<T> {
    pub(super) handles: Vec<Option<tokio::task::JoinHandle<T>>>,
}

impl<T> Default for DiscogsAuthTasks<T> {
    fn default() -> Self {
        Self {
            handles: Vec::new(),
        }
    }
}

impl<T: Send + 'static> DiscogsAuthTasks<T> {
    pub(super) fn spawn(&mut self, future: impl Future<Output = T> + Send + 'static) -> usize {
        let index = self.handles.len();
        self.handles.push(Some(tokio::spawn(future)));
        index
    }

    pub(super) async fn join(&mut self, index: usize, phase: &str) -> Result<T, String> {
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

    pub(super) async fn abort_and_reap(&mut self, index: usize, phase: &str) -> Result<(), String> {
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

    pub(super) async fn abort_all_and_reap(&mut self, phase: &str) -> Result<(), String> {
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

    pub(super) fn has_active_tasks(&self) -> bool {
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

pub(super) async fn finish_discogs_auth_scenario<T: Send + 'static>(
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

pub(super) fn set_discogs_pending(server: &ReklawdboxServer, device_id: &str, expires_at: i64) {
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

pub(super) fn assert_sanitized_discogs_transition_error(
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

pub(super) fn discogs_batch_params(track_ids: &[&str]) -> EnrichTracksParams {
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

pub(super) async fn run_discogs_batch_with_timeout(
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

pub(super) fn set_enrich_test_track_title(conn: &Connection, track_id: &str, title: &str) {
    conn.execute(
        "UPDATE djmdContent SET Title = ?2 WHERE ID = ?1",
        params![track_id, title],
    )
    .expect("enrichment test track title should update");
}

pub(super) fn install_enrichment_insert_failure(server: &ReklawdboxServer, raw_title: &str) {
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

pub(super) fn discogs_match(title: &str) -> crate::adapters::providers::discogs::DiscogsResult {
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

pub(super) fn assert_cache_write_summary(
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
