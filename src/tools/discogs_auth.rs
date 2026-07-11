use super::*;
use crate::bandcamp;
use crate::beatport;
use crate::discogs;
use crate::store;

#[derive(Clone)]
struct PersistedDiscogsSession {
    session_token: String,
    expires_at: i64,
}

trait DiscogsSessionPersistence: Send + Sync {
    fn load(&self, broker_url: &str) -> Result<Option<PersistedDiscogsSession>, String>;
    fn store(&self, broker_url: &str, session_token: &str, expires_at: i64) -> Result<(), String>;
    fn clear(&self, broker_url: &str) -> Result<(), String>;
}

struct StoreDiscogsSessionPersistence<'a> {
    server: &'a ReklawdboxServer,
}

impl DiscogsSessionPersistence for StoreDiscogsSessionPersistence<'_> {
    fn load(&self, broker_url: &str) -> Result<Option<PersistedDiscogsSession>, String> {
        let store = self
            .server
            .cache_store_conn()
            .map_err(|e| format!("Internal store error: {e}"))?;
        store::get_broker_discogs_session(&store, broker_url)
            .map(|session| {
                session.map(|session| PersistedDiscogsSession {
                    session_token: session.session_token,
                    expires_at: session.expires_at,
                })
            })
            .map_err(|e| format!("Broker session cache read error: {e}"))
    }

    fn store(&self, broker_url: &str, session_token: &str, expires_at: i64) -> Result<(), String> {
        let store = self
            .server
            .cache_store_conn()
            .map_err(|e| format!("Internal store error: {e}"))?;
        store::set_broker_discogs_session(&store, broker_url, session_token, expires_at)
            .map_err(|e| format!("Broker session cache write error: {e}"))
    }

    fn clear(&self, broker_url: &str) -> Result<(), String> {
        let store = self
            .server
            .cache_store_conn()
            .map_err(|e| format!("Internal store error: {e}"))?;
        store::clear_broker_discogs_session(&store, broker_url)
            .map_err(|e| format!("Broker session cache clear error: {e}"))
    }
}

#[cfg(test)]
#[derive(Default)]
struct InMemoryDiscogsSessionState {
    sessions: std::collections::HashMap<String, PersistedDiscogsSession>,
    store_count: usize,
    clear_count: usize,
    fail_next_store: bool,
    fail_next_clear: bool,
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct InMemoryDiscogsSessionPersistence {
    state: Mutex<InMemoryDiscogsSessionState>,
}

#[cfg(test)]
impl InMemoryDiscogsSessionPersistence {
    pub(super) fn set_session(&self, broker_url: &str, session_token: &str, expires_at: i64) {
        let mut state = self
            .state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned");
        state.sessions.insert(
            broker_url.to_string(),
            PersistedDiscogsSession {
                session_token: session_token.to_string(),
                expires_at,
            },
        );
    }

    pub(super) fn has_session(&self, broker_url: &str) -> bool {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .sessions
            .contains_key(broker_url)
    }

    pub(super) fn session_matches(&self, broker_url: &str, expected_token: &str) -> bool {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .sessions
            .get(broker_url)
            .is_some_and(|session| session.session_token == expected_token)
    }

    pub(super) fn store_count(&self) -> usize {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .store_count
    }

    pub(super) fn clear_count(&self) -> usize {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .clear_count
    }

    pub(super) fn fail_next_store(&self) {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .fail_next_store = true;
    }

    pub(super) fn fail_next_clear(&self) {
        self.state
            .lock()
            .expect("in-memory Discogs session state should not be poisoned")
            .fail_next_clear = true;
    }
}

#[cfg(test)]
impl DiscogsSessionPersistence for InMemoryDiscogsSessionPersistence {
    fn load(&self, broker_url: &str) -> Result<Option<PersistedDiscogsSession>, String> {
        Ok(self
            .state
            .lock()
            .map_err(|_| "in-memory Discogs session state poisoned".to_string())?
            .sessions
            .get(broker_url)
            .cloned())
    }

    fn store(&self, broker_url: &str, session_token: &str, expires_at: i64) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "in-memory Discogs session state poisoned".to_string())?;
        if std::mem::take(&mut state.fail_next_store) {
            return Err("injected broker session cache write error".to_string());
        }
        state.store_count += 1;
        state.sessions.insert(
            broker_url.to_string(),
            PersistedDiscogsSession {
                session_token: session_token.to_string(),
                expires_at,
            },
        );
        Ok(())
    }

    fn clear(&self, broker_url: &str) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "in-memory Discogs session state poisoned".to_string())?;
        if std::mem::take(&mut state.fail_next_clear) {
            return Err("injected broker session cache clear error".to_string());
        }
        state.clear_count += 1;
        state.sessions.remove(broker_url);
        Ok(())
    }
}

#[cfg(test)]
#[derive(Clone)]
pub(super) struct DiscogsAuthTestDependencies {
    cfg: discogs::BrokerConfig,
    now: i64,
    persistence: Arc<dyn DiscogsSessionPersistence>,
    entry_barrier: Option<Arc<tokio::sync::Barrier>>,
}

#[cfg(test)]
impl DiscogsAuthTestDependencies {
    pub(super) fn new(
        cfg: discogs::BrokerConfig,
        now: i64,
        persistence: Arc<InMemoryDiscogsSessionPersistence>,
    ) -> Self {
        Self {
            cfg,
            now,
            persistence,
            entry_barrier: None,
        }
    }

    pub(super) fn with_entry_barrier(mut self, barrier: Arc<tokio::sync::Barrier>) -> Self {
        self.entry_barrier = Some(barrier);
        self
    }
}

#[cfg(test)]
impl ReklawdboxServer {
    pub(super) fn set_discogs_auth_test_dependencies(
        &self,
        dependencies: DiscogsAuthTestDependencies,
    ) -> Result<(), String> {
        let mut lock = self
            .state
            .discogs_auth_dependencies
            .lock()
            .map_err(|_| "Discogs auth test dependency lock poisoned".to_string())?;
        *lock = Some(dependencies);
        Ok(())
    }

    fn discogs_auth_test_dependencies(
        &self,
    ) -> Result<Option<DiscogsAuthTestDependencies>, discogs::LookupError> {
        self.state
            .discogs_auth_dependencies
            .lock()
            .map(|lock| lock.clone())
            .map_err(|_| {
                discogs::LookupError::message("Discogs auth test dependency lock poisoned")
            })
    }
}

#[cfg(test)]
async fn wait_for_discogs_auth_test_entry(
    dependencies: &DiscogsAuthTestDependencies,
) -> Result<(), discogs::LookupError> {
    if let Some(barrier) = dependencies.entry_barrier.as_ref() {
        tokio::time::timeout(std::time::Duration::from_secs(5), barrier.wait())
            .await
            .map_err(|_| {
                discogs::LookupError::message("Discogs auth test entry barrier timed out")
            })?;
    }
    Ok(())
}

enum SessionState {
    /// A persisted session token that hasn't expired yet.
    Valid(String),
    /// The persisted session has expired and should be cleared.
    Expired,
    /// No persisted session exists.
    None,
}

enum PendingState {
    /// User has authorized in-browser; ready to finalize.
    Authorized(discogs::PendingDeviceSession),
    /// Still waiting for browser authorization.
    Waiting(discogs::PendingDeviceSession),
    /// The pending flow has expired.
    Expired,
    /// No pending flow exists.
    None,
}

/// Pure: no I/O or mutation.
fn resolve_session_state(persisted: Option<&PersistedDiscogsSession>, now: i64) -> SessionState {
    match persisted {
        Some(session) if session.expires_at > now => {
            SessionState::Valid(session.session_token.clone())
        }
        Some(_) => SessionState::Expired,
        None => SessionState::None,
    }
}

/// Pure: no I/O or mutation.
fn resolve_pending_state(
    pending: Option<&discogs::PendingDeviceSession>,
    status: Option<&str>,
    now: i64,
) -> PendingState {
    match pending {
        Some(p) if p.expires_at > now => match status {
            Some("authorized" | "finalized") => PendingState::Authorized(p.clone()),
            Some("pending") => PendingState::Waiting(p.clone()),
            _ => PendingState::Expired,
        },
        Some(_) => PendingState::Expired,
        None => PendingState::None,
    }
}

enum AuthResolution {
    ReadyToken(String),
    AuthRequired(discogs::AuthRemediation),
}

const DISCOGS_AUTH_START_FAILED: &str =
    "Discogs authentication start failed. Retry the lookup to start authorization.";
const DISCOGS_AUTH_STATUS_FAILED: &str =
    "Discogs authentication status check failed. Retry the lookup to continue authorization.";
const DISCOGS_AUTH_FINALIZE_FAILED: &str =
    "Discogs authentication finalization failed. Retry the lookup to complete authorization.";

fn pending_session(
    server: &ReklawdboxServer,
) -> Result<Option<discogs::PendingDeviceSession>, discogs::LookupError> {
    server
        .state
        .discogs_pending
        .lock()
        .map(|pending| pending.clone())
        .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))
}

fn replace_pending_session(
    server: &ReklawdboxServer,
    pending: Option<discogs::PendingDeviceSession>,
) -> Result<(), discogs::LookupError> {
    let mut lock = server
        .state
        .discogs_pending
        .lock()
        .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))?;
    *lock = pending;
    Ok(())
}

async fn resolve_auth_transition_locked(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    now: i64,
    persistence: &dyn DiscogsSessionPersistence,
) -> Result<AuthResolution, discogs::LookupError> {
    let persisted = persistence
        .load(&cfg.base_url)
        .map_err(discogs::LookupError::message)?;
    match resolve_session_state(persisted.as_ref(), now) {
        SessionState::Valid(token) => return Ok(AuthResolution::ReadyToken(token)),
        SessionState::Expired => persistence
            .clear(&cfg.base_url)
            .map_err(discogs::LookupError::message)?,
        SessionState::None => {}
    }

    let pending = pending_session(server)?;
    let pending_state = if let Some(ref pending) = pending {
        if pending.expires_at > now {
            let status = discogs::device_session_status(&server.state.http, cfg, pending)
                .await
                .map_err(|_| discogs::LookupError::message(DISCOGS_AUTH_STATUS_FAILED))?;
            resolve_pending_state(Some(pending), Some(&status.status), now)
        } else {
            PendingState::Expired
        }
    } else {
        PendingState::None
    };

    match pending_state {
        PendingState::Authorized(pending) => {
            let finalized = discogs::device_session_finalize(&server.state.http, cfg, &pending)
                .await
                .map_err(|_| discogs::LookupError::message(DISCOGS_AUTH_FINALIZE_FAILED))?;
            persistence
                .store(
                    &cfg.base_url,
                    &finalized.session_token,
                    finalized.expires_at,
                )
                .map_err(discogs::LookupError::message)?;
            replace_pending_session(server, None)?;
            Ok(AuthResolution::ReadyToken(finalized.session_token))
        }
        PendingState::Waiting(pending) => Ok(AuthResolution::AuthRequired(
            discogs::pending_auth_remediation(&pending),
        )),
        PendingState::Expired | PendingState::None => {
            if matches!(pending_state, PendingState::Expired) {
                replace_pending_session(server, None)?;
            }
            let started = discogs::device_session_start(&server.state.http, cfg)
                .await
                .map_err(|_| discogs::LookupError::message(DISCOGS_AUTH_START_FAILED))?;
            replace_pending_session(server, Some(started.clone()))?;
            Ok(AuthResolution::AuthRequired(
                discogs::pending_auth_remediation(&started),
            ))
        }
    }
}

async fn resolve_auth_transition(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    now: i64,
    persistence: &dyn DiscogsSessionPersistence,
) -> Result<AuthResolution, discogs::LookupError> {
    let _transition = server.state.discogs_auth_lock.lock().await;
    resolve_auth_transition_locked(server, cfg, now, persistence).await
}

fn conditional_clear_rejected_token(
    cfg: &discogs::BrokerConfig,
    persistence: &dyn DiscogsSessionPersistence,
    rejected_token: &str,
) -> Result<(), discogs::LookupError> {
    let current = persistence
        .load(&cfg.base_url)
        .map_err(discogs::LookupError::message)?;
    if current
        .as_ref()
        .is_some_and(|session| session.session_token == rejected_token)
    {
        persistence
            .clear(&cfg.base_url)
            .map_err(discogs::LookupError::message)?;
    }
    Ok(())
}

#[cfg(test)]
pub(super) async fn resolve_discogs_auth_transition_for_test(
    server: &ReklawdboxServer,
) -> Result<Option<discogs::AuthRemediation>, discogs::LookupError> {
    let dependencies = server.discogs_auth_test_dependencies()?.ok_or_else(|| {
        discogs::LookupError::message("Discogs auth test dependencies are not installed")
    })?;
    wait_for_discogs_auth_test_entry(&dependencies).await?;
    match resolve_auth_transition(
        server,
        &dependencies.cfg,
        dependencies.now,
        dependencies.persistence.as_ref(),
    )
    .await?
    {
        AuthResolution::ReadyToken(_) => Ok(None),
        AuthResolution::AuthRequired(remediation) => Ok(Some(remediation)),
    }
}

async fn lookup_discogs_with_config(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    now: i64,
    persistence: &dyn DiscogsSessionPersistence,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    let first_token = match resolve_auth_transition(server, cfg, now, persistence).await? {
        AuthResolution::ReadyToken(token) => token,
        AuthResolution::AuthRequired(remediation) => {
            return Err(discogs::LookupError::AuthRequired(remediation));
        }
    };

    match discogs::lookup_via_broker(&server.state.http, cfg, &first_token, artist, title, album)
        .await
    {
        Ok(result) => return Ok(result),
        Err(discogs::LookupError::AuthRequired(_)) => {}
        Err(error) => return Err(error),
    }

    let retry_token = {
        let _transition = server.state.discogs_auth_lock.lock().await;
        conditional_clear_rejected_token(cfg, persistence, &first_token)?;
        match resolve_auth_transition_locked(server, cfg, now, persistence).await? {
            AuthResolution::ReadyToken(token) => token,
            AuthResolution::AuthRequired(remediation) => {
                return Err(discogs::LookupError::AuthRequired(remediation));
            }
        }
    };

    match discogs::lookup_via_broker(&server.state.http, cfg, &retry_token, artist, title, album)
        .await
    {
        Ok(result) => Ok(result),
        Err(discogs::LookupError::AuthRequired(remediation)) => {
            let _transition = server.state.discogs_auth_lock.lock().await;
            conditional_clear_rejected_token(cfg, persistence, &retry_token)?;
            Err(discogs::LookupError::AuthRequired(remediation))
        }
        Err(error) => Err(error),
    }
}

pub(super) async fn lookup_discogs_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    #[cfg(test)]
    if let Some(result) = take_test_discogs_lookup_override(artist, title, album) {
        return result;
    }

    #[cfg(test)]
    if let Some(dependencies) = server.discogs_auth_test_dependencies()? {
        wait_for_discogs_auth_test_entry(&dependencies).await?;
        return lookup_discogs_with_config(
            server,
            &dependencies.cfg,
            dependencies.now,
            dependencies.persistence.as_ref(),
            artist,
            title,
            album,
        )
        .await;
    }

    match discogs::BrokerConfig::from_env() {
        discogs::BrokerConfigStatus::InvalidUrl(raw) => Err(discogs::LookupError::message(
            format!("Invalid broker URL in {}: {raw}", discogs::BROKER_URL_ENV),
        )),
        discogs::BrokerConfigStatus::Ok(cfg) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let persistence = StoreDiscogsSessionPersistence { server };
            lookup_discogs_with_config(server, &cfg, now, &persistence, artist, title, album).await
        }
    }
}

pub(super) async fn lookup_beatport_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<beatport::BeatportResult>, String> {
    #[cfg(test)]
    if let Some(result) = take_test_beatport_lookup_override(artist, title) {
        return result;
    }

    beatport::lookup(&server.state.http, artist, title)
        .await
        .map_err(|e| e.to_string())
}

pub(super) async fn lookup_bandcamp_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<bandcamp::BandcampResult>, String> {
    bandcamp::lookup(&server.state.http, artist, title)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(token: &str, expires_at: i64) -> PersistedDiscogsSession {
        PersistedDiscogsSession {
            session_token: token.to_string(),
            expires_at,
        }
    }

    fn make_pending(expires_at: i64) -> discogs::PendingDeviceSession {
        discogs::PendingDeviceSession {
            device_id: "dev-123".to_string(),
            pending_token: "pend-456".to_string(),
            auth_url: "https://broker.example.com/auth".to_string(),
            poll_interval_seconds: 5,
            expires_at,
        }
    }

    #[test]
    fn resolve_session_valid() {
        let session = make_session("tok-abc", 2000);
        let state = resolve_session_state(Some(&session), 1000);
        assert!(matches!(state, SessionState::Valid(t) if t == "tok-abc"));
    }

    #[test]
    fn resolve_session_expired() {
        let session = make_session("tok-abc", 500);
        let state = resolve_session_state(Some(&session), 1000);
        assert!(matches!(state, SessionState::Expired));
    }

    #[test]
    fn resolve_session_none() {
        let state = resolve_session_state(None, 1000);
        assert!(matches!(state, SessionState::None));
    }

    #[test]
    fn resolve_pending_authorized() {
        let pending = make_pending(2000);
        let state = resolve_pending_state(Some(&pending), Some("authorized"), 1000);
        assert!(matches!(state, PendingState::Authorized(_)));

        // "finalized" also counts as authorized
        let state = resolve_pending_state(Some(&pending), Some("finalized"), 1000);
        assert!(matches!(state, PendingState::Authorized(_)));
    }

    #[test]
    fn resolve_pending_waiting() {
        let pending = make_pending(2000);
        let state = resolve_pending_state(Some(&pending), Some("pending"), 1000);
        assert!(matches!(state, PendingState::Waiting(_)));
    }

    #[test]
    fn resolve_pending_expired_by_time() {
        let pending = make_pending(500);
        let state = resolve_pending_state(Some(&pending), Some("authorized"), 1000);
        assert!(matches!(state, PendingState::Expired));
    }

    #[test]
    fn resolve_pending_expired_by_unknown_status() {
        let pending = make_pending(2000);
        let state = resolve_pending_state(Some(&pending), Some("unknown"), 1000);
        assert!(matches!(state, PendingState::Expired));

        // None status on a non-expired pending also maps to Expired
        let state = resolve_pending_state(Some(&pending), None, 1000);
        assert!(matches!(state, PendingState::Expired));
    }

    #[test]
    fn resolve_session_expired_at_boundary() {
        // expires_at == now is expired (strict greater-than check)
        let session = make_session("tok-abc", 1000);
        let state = resolve_session_state(Some(&session), 1000);
        assert!(matches!(state, SessionState::Expired));
    }

    #[test]
    fn resolve_pending_expired_at_boundary() {
        // expires_at == now is expired (strict greater-than check)
        let pending = make_pending(1000);
        let state = resolve_pending_state(Some(&pending), Some("authorized"), 1000);
        assert!(matches!(state, PendingState::Expired));
    }

    #[test]
    fn resolve_pending_none() {
        let state = resolve_pending_state(None, None, 1000);
        assert!(matches!(state, PendingState::None));
    }
}
