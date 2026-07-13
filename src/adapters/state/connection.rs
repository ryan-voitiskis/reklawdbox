use std::path::PathBuf;

use rusqlite::{Connection, OpenFlags, ffi};

use super::migrations;

pub fn default_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reklawdbox")
        .join("internal.sqlite3")
}

pub fn resolve_path() -> PathBuf {
    resolve_path_from(std::env::var_os("CRATE_DIG_STORE_PATH"))
}

pub(crate) fn resolve_path_from(configured: Option<std::ffi::OsString>) -> PathBuf {
    configured.map_or_else(default_path, PathBuf::from)
}

pub fn open(path: &str) -> Result<Connection, rusqlite::Error> {
    let store_path = std::path::Path::new(path);
    if let Some(parent) = store_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_CANTOPEN),
                Some(format!(
                    "failed to create parent directory {} for {}: {}",
                    parent.display(),
                    store_path.display(),
                    err
                )),
            )
        })?;
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = NORMAL;",
    )?;
    migrations::migrate(&conn)?;
    Ok(conn)
}

/// Open a read-only connection (no migrations). For concurrent reader tasks.
pub fn open_read_only(path: &str) -> Result<Connection, rusqlite::Error> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    let conn = Connection::open_with_flags(path, flags)?;
    conn.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}
