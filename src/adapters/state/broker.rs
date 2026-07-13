use rusqlite::{Connection, ffi, params};

pub struct BrokerDiscogsSession {
    #[allow(dead_code)]
    pub broker_url: String,
    pub session_token: String,
    pub expires_at: i64,
    #[allow(dead_code)]
    pub created_at: String,
    #[allow(dead_code)]
    pub updated_at: String,
}

pub fn get_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<Option<BrokerDiscogsSession>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT broker_url, session_token, expires_at, created_at, updated_at
         FROM broker_discogs_session
         WHERE broker_url = ?1",
    )?;
    let mut rows = stmt.query_map(params![broker_url], |row| {
        Ok(BrokerDiscogsSession {
            broker_url: row.get(0)?,
            session_token: row.get(1)?,
            expires_at: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;
    let mut session = match rows.next() {
        Some(Ok(entry)) => entry,
        Some(Err(e)) => return Err(e),
        None => return Ok(None),
    };

    // Migrate legacy plaintext tokens to Keychain.
    if !session.session_token.is_empty() {
        crate::keychain::set_session_token(broker_url, &session.session_token).map_err(|msg| {
            rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_ERROR), Some(msg))
        })?;
        conn.execute(
            "UPDATE broker_discogs_session SET session_token = '' WHERE broker_url = ?1",
            params![broker_url],
        )?;
    }

    match crate::keychain::get_session_token(broker_url) {
        Ok(Some(token)) => {
            session.session_token = token;
            Ok(Some(session))
        }
        Ok(None) => Ok(None),
        Err(msg) => Err(rusqlite::Error::SqliteFailure(
            ffi::Error::new(ffi::SQLITE_ERROR),
            Some(msg),
        )),
    }
}

pub fn set_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
    session_token: &str,
    expires_at: i64,
) -> Result<(), rusqlite::Error> {
    // Store the secret in Keychain first; if this fails, don't persist metadata.
    crate::keychain::set_session_token(broker_url, session_token).map_err(|msg| {
        rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_ERROR), Some(msg))
    })?;

    conn.execute(
        "INSERT INTO broker_discogs_session (broker_url, session_token, expires_at)
         VALUES (?1, '', ?2)
         ON CONFLICT(broker_url)
         DO UPDATE SET
            session_token = '',
            expires_at = ?2,
            updated_at = datetime('now')",
        params![broker_url, expires_at],
    )?;
    Ok(())
}

pub fn clear_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<(), rusqlite::Error> {
    crate::keychain::delete_session_token(broker_url).map_err(|msg| {
        rusqlite::Error::SqliteFailure(ffi::Error::new(ffi::SQLITE_ERROR), Some(msg))
    })?;

    conn.execute(
        "DELETE FROM broker_discogs_session WHERE broker_url = ?1",
        params![broker_url],
    )?;
    Ok(())
}
