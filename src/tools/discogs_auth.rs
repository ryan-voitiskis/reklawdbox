use super::*;
use crate::bandcamp;
use crate::beatport;
use crate::discogs;
use crate::store;

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
fn resolve_session_state(
    persisted: Option<&store::BrokerDiscogsSession>,
    now: i64,
) -> SessionState {
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

/// Only makes a broker status call when a non-expired pending session exists.
async fn fetch_pending_state(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    now: i64,
) -> Result<PendingState, discogs::LookupError> {
    let pending = {
        let lock = server
            .state
            .discogs_pending
            .lock()
            .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))?;
        lock.clone()
    };

    let status = if let Some(ref p) = pending {
        if p.expires_at > now {
            Some(
                discogs::device_session_status(&server.state.http, cfg, p)
                    .await
                    .map_err(|e| {
                        discogs::LookupError::message(format!("Discogs broker status error: {e}"))
                    })?
                    .status,
            )
        } else {
            None
        }
    } else {
        None
    };

    Ok(resolve_pending_state(
        pending.as_ref(),
        status.as_deref(),
        now,
    ))
}

async fn dispatch_pending(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
    pending: PendingState,
    artist: &str,
    title: &str,
    album: Option<&str>,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    match pending {
        PendingState::Authorized(p) => {
            let finalized = discogs::device_session_finalize(&server.state.http, cfg, &p)
                .await
                .map_err(|e| {
                    discogs::LookupError::message(format!("Discogs broker finalize error: {e}"))
                })?;
            {
                let store = server.cache_store_conn().map_err(|e| {
                    discogs::LookupError::message(format!("Internal store error: {e}"))
                })?;
                store::set_broker_discogs_session(
                    &store,
                    &cfg.base_url,
                    &finalized.session_token,
                    finalized.expires_at,
                )
                .map_err(|e| {
                    discogs::LookupError::message(format!("Broker session cache write error: {e}"))
                })?;
            }
            {
                let mut lock = server.state.discogs_pending.lock().map_err(|_| {
                    discogs::LookupError::message("Discogs auth state lock poisoned")
                })?;
                *lock = None;
            }
            discogs::lookup_via_broker(
                &server.state.http,
                cfg,
                &finalized.session_token,
                artist,
                title,
                album,
            )
            .await
        }
        PendingState::Waiting(p) => Err(discogs::LookupError::AuthRequired(
            discogs::pending_auth_remediation(&p),
        )),
        PendingState::Expired => {
            {
                let mut lock = server.state.discogs_pending.lock().map_err(|_| {
                    discogs::LookupError::message("Discogs auth state lock poisoned")
                })?;
                *lock = None;
            }
            start_new_session(server, cfg).await
        }
        PendingState::None => start_new_session(server, cfg).await,
    }
}

/// Start a fresh device-auth session and return the auth-required error.
async fn start_new_session(
    server: &ReklawdboxServer,
    cfg: &discogs::BrokerConfig,
) -> Result<Option<discogs::DiscogsResult>, discogs::LookupError> {
    let started = discogs::device_session_start(&server.state.http, cfg)
        .await
        .map_err(|e| discogs::LookupError::message(format!("Discogs broker start error: {e}")))?;
    {
        let mut lock = server
            .state
            .discogs_pending
            .lock()
            .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))?;
        *lock = Some(started.clone());
    }
    Err(discogs::LookupError::AuthRequired(
        discogs::pending_auth_remediation(&started),
    ))
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

    match discogs::BrokerConfig::from_env() {
        discogs::BrokerConfigStatus::InvalidUrl(raw) => Err(discogs::LookupError::message(
            format!("Invalid broker URL in {}: {raw}", discogs::BROKER_URL_ENV),
        )),
        discogs::BrokerConfigStatus::Ok(cfg) => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let persisted_session = {
                let store = server.cache_store_conn().map_err(|e| {
                    discogs::LookupError::message(format!("Internal store error: {e}"))
                })?;
                store::get_broker_discogs_session(&store, &cfg.base_url).map_err(|e| {
                    discogs::LookupError::message(format!("Broker session cache read error: {e}"))
                })?
            };

            let session_state = resolve_session_state(persisted_session.as_ref(), now);

            match session_state {
                SessionState::Valid(token) => {
                    match discogs::lookup_via_broker(
                        &server.state.http,
                        &cfg,
                        &token,
                        artist,
                        title,
                        album,
                    )
                    .await
                    {
                        Ok(result) => return Ok(result),
                        Err(discogs::LookupError::AuthRequired(_)) => {
                            // Session rejected by broker — clear it and fall through
                            let store = server.cache_store_conn().map_err(|e| {
                                discogs::LookupError::message(format!("Internal store error: {e}"))
                            })?;
                            store::clear_broker_discogs_session(&store, &cfg.base_url).map_err(
                                |e| {
                                    discogs::LookupError::message(format!(
                                        "Broker session cache clear error: {e}"
                                    ))
                                },
                            )?;
                        }
                        Err(e) => return Err(e),
                    }
                }
                SessionState::Expired => {
                    let store = server.cache_store_conn().map_err(|e| {
                        discogs::LookupError::message(format!("Internal store error: {e}"))
                    })?;
                    store::clear_broker_discogs_session(&store, &cfg.base_url).map_err(|e| {
                        discogs::LookupError::message(format!(
                            "Broker session cache clear error: {e}"
                        ))
                    })?;
                }
                SessionState::None => {}
            }

            let pending = fetch_pending_state(server, &cfg, now).await?;
            return dispatch_pending(server, &cfg, pending, artist, title, album).await;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session(token: &str, expires_at: i64) -> store::BrokerDiscogsSession {
        store::BrokerDiscogsSession {
            broker_url: "https://broker.example.com".to_string(),
            session_token: token.to_string(),
            expires_at,
            created_at: "2024-01-01".to_string(),
            updated_at: "2024-01-01".to_string(),
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

pub(super) async fn lookup_bandcamp_remote(
    server: &ReklawdboxServer,
    artist: &str,
    title: &str,
) -> Result<Option<bandcamp::BandcampResult>, String> {
    bandcamp::lookup(&server.state.http, artist, title)
        .await
        .map_err(|e| e.to_string())
}
