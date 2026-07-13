use super::*;
use crate::application::enrichment::discogs_auth::{
    AuthResolution, DiscogsAuthGateway, DiscogsSessionPersistence, PersistedDiscogsSession,
    conditional_clear_rejected_token,
    resolve_auth_transition_locked as resolve_application_auth_transition,
};
use crate::bandcamp;
use crate::beatport;
use crate::discogs;
use crate::store;

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

struct ServerDiscogsAuthGateway<'a> {
    server: &'a ReklawdboxServer,
}

impl DiscogsAuthGateway for ServerDiscogsAuthGateway<'_> {
    fn pending_session(
        &self,
    ) -> Result<Option<discogs::PendingDeviceSession>, discogs::LookupError> {
        pending_session(self.server)
    }

    fn replace_pending_session(
        &self,
        pending: Option<discogs::PendingDeviceSession>,
    ) -> Result<(), discogs::LookupError> {
        replace_pending_session(self.server, pending)
    }

    async fn pending_status(
        &self,
        cfg: &discogs::BrokerConfig,
        pending: &discogs::PendingDeviceSession,
    ) -> Result<String, String> {
        discogs::device_session_status(&self.server.state.http, cfg, pending)
            .await
            .map(|status| status.status)
    }

    async fn finalize_pending(
        &self,
        cfg: &discogs::BrokerConfig,
        pending: &discogs::PendingDeviceSession,
    ) -> Result<discogs::FinalizedDeviceSession, String> {
        discogs::device_session_finalize(&self.server.state.http, cfg, pending).await
    }

    async fn start_pending(
        &self,
        cfg: &discogs::BrokerConfig,
    ) -> Result<discogs::PendingDeviceSession, String> {
        discogs::device_session_start(&self.server.state.http, cfg).await
    }
}

async fn resolve_auth_transition_locked(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    now: i64,
    persistence: &dyn DiscogsSessionPersistence,
) -> Result<AuthResolution, discogs::LookupError> {
    let gateway = ServerDiscogsAuthGateway { server };
    resolve_application_auth_transition(&gateway, cfg, now, persistence).await
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
