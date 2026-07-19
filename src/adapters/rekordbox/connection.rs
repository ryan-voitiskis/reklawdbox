use rusqlite::{Connection, OpenFlags};

/// The universal Rekordbox 6/7 SQLCipher key (publicly known, same for all installations).
pub(crate) const REKORDBOX_SQLCIPHER_KEY: &str =
    "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";

pub fn open(path: &str) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Key is passed as a passphrase — SQLCipher derives the encryption key via PBKDF2.
    conn.execute_batch(&format!("PRAGMA key = '{REKORDBOX_SQLCIPHER_KEY}'"))?;
    conn.execute_batch("PRAGMA busy_timeout = 5000;")?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_test() -> Connection {
    Connection::open_in_memory().unwrap()
}

pub fn default_db_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/Library/Pioneer/rekordbox/master.db");
    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

pub fn resolve_db_path() -> Option<String> {
    resolve_db_path_from(std::env::var_os("REKORDBOX_DB_PATH"), default_db_path)
}

pub(crate) fn resolve_db_path_from(
    configured: Option<std::ffi::OsString>,
    default: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if let Some(path) = configured {
        let path = std::path::PathBuf::from(path);
        return path.is_file().then(|| path.to_string_lossy().into_owned());
    }
    default()
}
