//! Discogs broker authentication state machine.

use crate::adapters::providers::discogs;

#[derive(Clone)]
pub(crate) struct PersistedDiscogsSession {
    pub(crate) session_token: String,
    pub(crate) expires_at: i64,
}

pub(crate) trait DiscogsSessionPersistence: Send + Sync {
    fn load(&self, broker_url: &str) -> Result<Option<PersistedDiscogsSession>, String>;
    fn store(&self, broker_url: &str, session_token: &str, expires_at: i64) -> Result<(), String>;
    fn clear(&self, broker_url: &str) -> Result<(), String>;
}

#[allow(async_fn_in_trait)]
pub(crate) trait DiscogsAuthGateway: Send + Sync {
    fn pending_session(
        &self,
    ) -> Result<Option<discogs::PendingDeviceSession>, discogs::LookupError>;
    fn replace_pending_session(
        &self,
        pending: Option<discogs::PendingDeviceSession>,
    ) -> Result<(), discogs::LookupError>;
    async fn pending_status(
        &self,
        cfg: &discogs::BrokerConfig,
        pending: &discogs::PendingDeviceSession,
    ) -> Result<String, String>;
    async fn finalize_pending(
        &self,
        cfg: &discogs::BrokerConfig,
        pending: &discogs::PendingDeviceSession,
    ) -> Result<discogs::FinalizedDeviceSession, String>;
    async fn start_pending(
        &self,
        cfg: &discogs::BrokerConfig,
    ) -> Result<discogs::PendingDeviceSession, String>;
}

enum SessionState {
    Valid(String),
    Expired,
    None,
}

enum PendingState {
    Authorized(discogs::PendingDeviceSession),
    Waiting(discogs::PendingDeviceSession),
    Expired,
    None,
}

fn resolve_session_state(persisted: Option<&PersistedDiscogsSession>, now: i64) -> SessionState {
    match persisted {
        Some(session) if session.expires_at > now => {
            SessionState::Valid(session.session_token.clone())
        }
        Some(_) => SessionState::Expired,
        None => SessionState::None,
    }
}

fn resolve_pending_state(
    pending: Option<&discogs::PendingDeviceSession>,
    status: Option<&str>,
    now: i64,
) -> PendingState {
    match pending {
        Some(pending) if pending.expires_at > now => match status {
            Some("authorized" | "finalized") => PendingState::Authorized(pending.clone()),
            Some("pending") => PendingState::Waiting(pending.clone()),
            _ => PendingState::Expired,
        },
        Some(_) => PendingState::Expired,
        None => PendingState::None,
    }
}

pub(crate) enum AuthResolution {
    ReadyToken(String),
    AuthRequired(discogs::AuthRemediation),
}

const DISCOGS_AUTH_START_FAILED: &str =
    "Discogs authentication start failed. Retry the lookup to start authorization.";
const DISCOGS_AUTH_STATUS_FAILED: &str =
    "Discogs authentication status check failed. Retry the lookup to continue authorization.";
const DISCOGS_AUTH_FINALIZE_FAILED: &str =
    "Discogs authentication finalization failed. Retry the lookup to complete authorization.";

pub(crate) async fn resolve_auth_transition_locked(
    gateway: &impl DiscogsAuthGateway,
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

    let pending = gateway.pending_session()?;
    let pending_state = if let Some(ref pending) = pending {
        if pending.expires_at > now {
            let status = gateway
                .pending_status(cfg, pending)
                .await
                .map_err(|_| discogs::LookupError::message(DISCOGS_AUTH_STATUS_FAILED))?;
            resolve_pending_state(Some(pending), Some(&status), now)
        } else {
            PendingState::Expired
        }
    } else {
        PendingState::None
    };

    match pending_state {
        PendingState::Authorized(pending) => {
            let finalized = gateway
                .finalize_pending(cfg, &pending)
                .await
                .map_err(|_| discogs::LookupError::message(DISCOGS_AUTH_FINALIZE_FAILED))?;
            persistence
                .store(
                    &cfg.base_url,
                    &finalized.session_token,
                    finalized.expires_at,
                )
                .map_err(discogs::LookupError::message)?;
            gateway.replace_pending_session(None)?;
            Ok(AuthResolution::ReadyToken(finalized.session_token))
        }
        PendingState::Waiting(pending) => Ok(AuthResolution::AuthRequired(
            discogs::pending_auth_remediation(&pending),
        )),
        PendingState::Expired | PendingState::None => {
            if matches!(pending_state, PendingState::Expired) {
                gateway.replace_pending_session(None)?;
            }
            let started = gateway.start_pending(cfg).await.map_err(|error| {
                if error == discogs::INVALID_BROKER_AUTHORIZATION_URL {
                    discogs::LookupError::message(error)
                } else {
                    discogs::LookupError::message(DISCOGS_AUTH_START_FAILED)
                }
            })?;
            gateway.replace_pending_session(Some(started.clone()))?;
            Ok(AuthResolution::AuthRequired(
                discogs::pending_auth_remediation(&started),
            ))
        }
    }
}

pub(crate) fn conditional_clear_rejected_token(
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
        assert!(
            matches!(resolve_session_state(Some(&session), 1000), SessionState::Valid(token) if token == "tok-abc")
        );
    }

    #[test]
    fn resolve_session_expired() {
        let session = make_session("tok-abc", 500);
        assert!(matches!(
            resolve_session_state(Some(&session), 1000),
            SessionState::Expired
        ));
    }

    #[test]
    fn resolve_session_none() {
        assert!(matches!(
            resolve_session_state(None, 1000),
            SessionState::None
        ));
    }

    #[test]
    fn resolve_pending_authorized() {
        let pending = make_pending(2000);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("authorized"), 1000),
            PendingState::Authorized(_)
        ));
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("finalized"), 1000),
            PendingState::Authorized(_)
        ));
    }

    #[test]
    fn resolve_pending_waiting() {
        let pending = make_pending(2000);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("pending"), 1000),
            PendingState::Waiting(_)
        ));
    }

    #[test]
    fn resolve_pending_expired_by_time() {
        let pending = make_pending(500);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("authorized"), 1000),
            PendingState::Expired
        ));
    }

    #[test]
    fn resolve_pending_expired_by_unknown_status() {
        let pending = make_pending(2000);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("unknown"), 1000),
            PendingState::Expired
        ));
        assert!(matches!(
            resolve_pending_state(Some(&pending), None, 1000),
            PendingState::Expired
        ));
    }

    #[test]
    fn resolve_session_expired_at_boundary() {
        let session = make_session("tok-abc", 1000);
        assert!(matches!(
            resolve_session_state(Some(&session), 1000),
            SessionState::Expired
        ));
    }

    #[test]
    fn resolve_pending_expired_at_boundary() {
        let pending = make_pending(1000);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("authorized"), 1000),
            PendingState::Expired
        ));
    }

    #[test]
    fn resolve_pending_none() {
        assert!(matches!(
            resolve_pending_state(None, None, 1000),
            PendingState::None
        ));
    }

    #[test]
    fn resolve_session_valid_expired_and_none() {
        let valid = make_session("tok-abc", 2000);
        assert!(
            matches!(resolve_session_state(Some(&valid), 1000), SessionState::Valid(token) if token == "tok-abc")
        );
        let expired = make_session("tok-abc", 1000);
        assert!(matches!(
            resolve_session_state(Some(&expired), 1000),
            SessionState::Expired
        ));
        assert!(matches!(
            resolve_session_state(None, 1000),
            SessionState::None
        ));
    }

    #[test]
    fn resolve_pending_authorized_waiting_expired_and_none() {
        let pending = make_pending(2000);
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("authorized"), 1000),
            PendingState::Authorized(_)
        ));
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("finalized"), 1000),
            PendingState::Authorized(_)
        ));
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("pending"), 1000),
            PendingState::Waiting(_)
        ));
        assert!(matches!(
            resolve_pending_state(Some(&pending), Some("unknown"), 1000),
            PendingState::Expired
        ));
        assert!(matches!(
            resolve_pending_state(Some(&make_pending(1000)), Some("authorized"), 1000),
            PendingState::Expired
        ));
        assert!(matches!(
            resolve_pending_state(None, None, 1000),
            PendingState::None
        ));
    }
}
