use rusqlite::{Connection, OpenFlags, ffi, params};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::db::escape_like;

/// (id, path, issue_type, detail)
pub type AuditIssueRow = (i64, String, String, Option<String>);

pub fn default_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reklawdbox")
        .join("internal.sqlite3")
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
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS enrichment_cache (
            provider TEXT NOT NULL,
            query_artist TEXT NOT NULL,
            query_title TEXT NOT NULL,
            match_quality TEXT,
            response_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (provider, query_artist, query_title)
        );
        CREATE TABLE IF NOT EXISTS audio_analysis_cache (
            file_path TEXT NOT NULL,
            analyzer TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            file_mtime INTEGER NOT NULL,
            analysis_version TEXT NOT NULL,
            features_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (file_path, analyzer)
        );
        CREATE TABLE IF NOT EXISTS broker_discogs_session (
            broker_url TEXT PRIMARY KEY,
            session_token TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS audit_files (
            path         TEXT PRIMARY KEY,
            last_audited TEXT NOT NULL,
            file_mtime   TEXT NOT NULL,
            file_size    INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS audit_issues (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            path        TEXT NOT NULL REFERENCES audit_files(path) ON DELETE CASCADE ON UPDATE CASCADE,
            issue_type  TEXT NOT NULL,
            detail      TEXT,
            status      TEXT NOT NULL DEFAULT 'open',
            resolution  TEXT,
            note        TEXT,
            created_at  TEXT NOT NULL,
            resolved_at TEXT,
            UNIQUE(path, issue_type)
        );
        CREATE INDEX IF NOT EXISTS idx_audit_issues_status ON audit_issues(status);
        CREATE INDEX IF NOT EXISTS idx_audit_issues_path ON audit_issues(path);
        PRAGMA user_version = 3;",
    )?;
    Ok(())
}

#[allow(dead_code)]
pub struct EnrichmentCacheEntry {
    pub provider: String,
    pub query_artist: String,
    pub query_title: String,
    pub match_quality: Option<String>,
    pub response_json: Option<String>,
    pub created_at: String,
}

pub fn get_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
) -> Result<Option<EnrichmentCacheEntry>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT provider, query_artist, query_title, match_quality, response_json, created_at
         FROM enrichment_cache
         WHERE provider = ?1 AND query_artist = ?2 AND query_title = ?3",
    )?;
    let mut rows = stmt.query_map(params![provider, artist, title], |row| {
        Ok(EnrichmentCacheEntry {
            provider: row.get(0)?,
            query_artist: row.get(1)?,
            query_title: row.get(2)?,
            match_quality: row.get(3)?,
            response_json: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// Batch existence check for enrichment cache entries.
/// Returns `(query_artist, query_title)` pairs present for the given provider.
/// Over-fetches by artist (all titles for matched artists), so the caller
/// filters via `HashSet::contains`.
pub fn batch_enrichment_existence(
    conn: &Connection,
    provider: &str,
    artists: &[&str],
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    if artists.is_empty() {
        return Ok(HashSet::new());
    }
    // Reserve 1 bind var for provider, rest for the IN list.
    const MAX_IN_VARS: usize = 899;
    let mut result = HashSet::new();
    for chunk in artists.chunks(MAX_IN_VARS) {
        let placeholders: Vec<String> = (2..=chunk.len() + 1).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT query_artist, query_title FROM enrichment_cache \
             WHERE provider = ?1 AND query_artist IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let mut bind_values: Vec<&dyn rusqlite::types::ToSql> = Vec::with_capacity(chunk.len() + 1);
        bind_values.push(&provider);
        for artist in chunk {
            bind_values.push(artist);
        }
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            result.insert(row?);
        }
    }
    Ok(result)
}

pub fn set_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
    match_quality: Option<&str>,
    response_json: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO enrichment_cache (provider, query_artist, query_title, match_quality, response_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(provider, query_artist, query_title)
         DO UPDATE SET match_quality = ?4, response_json = ?5, created_at = datetime('now')",
        params![provider, artist, title, match_quality, response_json],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub struct CachedAudioAnalysis {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analysis_version: String,
    pub features_json: String,
    pub created_at: String,
}

pub fn get_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
) -> Result<Option<CachedAudioAnalysis>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT file_path, analyzer, file_size, file_mtime, analysis_version, features_json, created_at
         FROM audio_analysis_cache
         WHERE file_path = ?1 AND analyzer = ?2",
    )?;
    let mut rows = stmt.query_map(params![file_path, analyzer], |row| {
        Ok(CachedAudioAnalysis {
            file_path: row.get(0)?,
            analyzer: row.get(1)?,
            file_size: row.get(2)?,
            file_mtime: row.get(3)?,
            analysis_version: row.get(4)?,
            features_json: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// Batch existence check for audio analysis cache entries.
/// Returns `(file_path, analyzer)` pairs present in the cache.
pub fn batch_audio_analysis_existence(
    conn: &Connection,
    file_paths: &[&str],
) -> Result<HashSet<(String, String)>, rusqlite::Error> {
    if file_paths.is_empty() {
        return Ok(HashSet::new());
    }
    const MAX_BIND_VARS: usize = 900;
    let mut result = HashSet::new();
    for chunk in file_paths.chunks(MAX_BIND_VARS) {
        let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT file_path, analyzer FROM audio_analysis_cache \
             WHERE file_path IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let bind_values: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(bind_values.as_slice(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            result.insert(row?);
        }
    }
    Ok(result)
}

pub fn set_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
    file_size: i64,
    file_mtime: i64,
    analysis_version: &str,
    features_json: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO audio_analysis_cache (file_path, analyzer, file_size, file_mtime, analysis_version, features_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(file_path, analyzer)
         DO UPDATE SET file_size = ?3, file_mtime = ?4, analysis_version = ?5, features_json = ?6, created_at = datetime('now')",
        params![file_path, analyzer, file_size, file_mtime, analysis_version, features_json],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub struct BrokerDiscogsSession {
    pub broker_url: String,
    pub session_token: String,
    pub expires_at: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn get_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<Option<BrokerDiscogsSession>, rusqlite::Error> {
    let mut stmt = conn.prepare(
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
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn set_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
    session_token: &str,
    expires_at: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO broker_discogs_session (broker_url, session_token, expires_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(broker_url)
         DO UPDATE SET
            session_token = ?2,
            expires_at = ?3,
            updated_at = datetime('now')",
        params![broker_url, session_token, expires_at],
    )?;
    Ok(())
}

/// Clear all caches except broker sessions. Returns row counts per table.
pub fn clear_caches(conn: &Connection) -> Result<ClearCachesResult, rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    let enrichment = tx.execute("DELETE FROM enrichment_cache", [])?;
    let audio_analysis = tx.execute("DELETE FROM audio_analysis_cache", [])?;
    // audit_issues before audit_files: child rows first to get accurate counts
    // (ON DELETE CASCADE would handle it, but then audit_issues count would be 0)
    let audit_issues = tx.execute("DELETE FROM audit_issues", [])?;
    let audit_files = tx.execute("DELETE FROM audit_files", [])?;
    tx.commit()?;
    Ok(ClearCachesResult {
        enrichment,
        audio_analysis,
        audit_issues,
        audit_files,
    })
}

pub struct ClearCachesResult {
    pub enrichment: usize,
    pub audio_analysis: usize,
    pub audit_issues: usize,
    pub audit_files: usize,
}

pub fn clear_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM broker_discogs_session WHERE broker_url = ?1",
        params![broker_url],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Audit state
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct AuditFile {
    pub path: String,
    pub last_audited: String,
    pub file_mtime: String,
    pub file_size: i64,
}

pub struct AuditIssue {
    pub id: i64,
    pub path: String,
    pub issue_type: String,
    pub detail: Option<String>,
    pub status: String,
    pub resolution: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

pub struct AuditSummary {
    pub by_type_status: Vec<(String, String, i64)>,
}

pub fn upsert_audit_file(
    conn: &Connection,
    path: &str,
    last_audited: &str,
    file_mtime: &str,
    file_size: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO audit_files (path, last_audited, file_mtime, file_size)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path)
         DO UPDATE SET last_audited = ?2, file_mtime = ?3, file_size = ?4",
        params![path, last_audited, file_mtime, file_size],
    )?;
    Ok(())
}

pub fn get_audit_files_in_scope(
    conn: &Connection,
    scope: &str,
) -> Result<Vec<AuditFile>, rusqlite::Error> {
    let pattern = format!("{}%", escape_like(scope));
    let mut stmt = conn.prepare(
        "SELECT path, last_audited, file_mtime, file_size
         FROM audit_files WHERE path LIKE ?1 ESCAPE '\\'",
    )?;
    let rows = stmt.query_map(params![pattern], |row| {
        Ok(AuditFile {
            path: row.get(0)?,
            last_audited: row.get(1)?,
            file_mtime: row.get(2)?,
            file_size: row.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
pub fn get_audit_file(conn: &Connection, path: &str) -> Result<Option<AuditFile>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT path, last_audited, file_mtime, file_size
         FROM audit_files WHERE path = ?1",
    )?;
    let mut rows = stmt.query_map(params![path], |row| {
        Ok(AuditFile {
            path: row.get(0)?,
            last_audited: row.get(1)?,
            file_mtime: row.get(2)?,
            file_size: row.get(3)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

#[cfg(test)]
pub fn delete_audit_file(conn: &Connection, path: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM audit_files WHERE path = ?1", params![path])?;
    Ok(())
}

pub fn upsert_audit_issue(
    conn: &Connection,
    path: &str,
    issue_type: &str,
    detail: Option<&str>,
    status: &str,
    created_at: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO audit_issues (path, issue_type, detail, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path, issue_type)
         DO UPDATE SET
             detail = ?3,
             status = CASE
                 WHEN audit_issues.status IN ('accepted', 'deferred') THEN audit_issues.status
                 ELSE ?4
             END,
             resolution = CASE
                 WHEN audit_issues.status IN ('accepted', 'deferred') THEN audit_issues.resolution
                 ELSE NULL
             END,
             resolved_at = CASE
                 WHEN audit_issues.status IN ('accepted', 'deferred') THEN audit_issues.resolved_at
                 ELSE NULL
             END,
             note = CASE
                 WHEN audit_issues.status IN ('accepted', 'deferred') THEN audit_issues.note
                 ELSE NULL
             END",
        params![path, issue_type, detail, status, created_at],
    )?;
    Ok(())
}

pub fn get_audit_issues(
    conn: &Connection,
    scope: &str,
    status: Option<&str>,
    issue_type: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<AuditIssue>, rusqlite::Error> {
    let pattern = format!("{}%", escape_like(scope));
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(pattern)); // ?1
    param_values.push(Box::new(limit)); // ?2
    param_values.push(Box::new(offset)); // ?3

    let mut conditions = String::new();
    if let Some(s) = status {
        param_values.push(Box::new(s.to_string()));
        conditions.push_str(&format!(" AND status = ?{}", param_values.len()));
    }
    if let Some(issue_type_filter) = issue_type {
        param_values.push(Box::new(issue_type_filter.to_string()));
        conditions.push_str(&format!(" AND issue_type = ?{}", param_values.len()));
    }

    let sql = format!(
        "SELECT id, path, issue_type, detail, status, resolution, note, created_at, resolved_at
         FROM audit_issues
         WHERE path LIKE ?1 ESCAPE '\\'{conditions}
         ORDER BY path, issue_type
         LIMIT ?2 OFFSET ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_values), map_audit_issue)?
        .collect::<Result<_, _>>()?;
    Ok(rows)
}

fn map_audit_issue(row: &rusqlite::Row) -> Result<AuditIssue, rusqlite::Error> {
    Ok(AuditIssue {
        id: row.get(0)?,
        path: row.get(1)?,
        issue_type: row.get(2)?,
        detail: row.get(3)?,
        status: row.get(4)?,
        resolution: row.get(5)?,
        note: row.get(6)?,
        created_at: row.get(7)?,
        resolved_at: row.get(8)?,
    })
}

#[cfg(test)]
pub fn get_audit_issue_by_id(
    conn: &Connection,
    id: i64,
) -> Result<Option<AuditIssue>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, path, issue_type, detail, status, resolution, note, created_at, resolved_at
         FROM audit_issues WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], map_audit_issue)?;
    match rows.next() {
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn resolve_audit_issues(
    conn: &Connection,
    ids: &[i64],
    resolution: crate::audit::Resolution,
    note: Option<&str>,
    resolved_at: &str,
) -> Result<usize, rusqlite::Error> {
    let status = resolution.status().as_str();
    let resolution_str = resolution.as_str();
    let mut count = 0usize;
    for id in ids {
        count += conn.execute(
            "UPDATE audit_issues
             SET status = ?1, resolution = ?2, note = COALESCE(?3, note), resolved_at = ?4
             WHERE id = ?5 AND status = 'open'",
            params![status, resolution_str, note, resolved_at, id],
        )?;
    }
    Ok(count)
}

pub fn mark_issues_resolved_for_path(
    conn: &Connection,
    path: &str,
    issue_types_still_open: &[&str],
    resolved_at: &str,
) -> Result<usize, rusqlite::Error> {
    if issue_types_still_open.is_empty() {
        let count = conn.execute(
            "UPDATE audit_issues
             SET status = 'resolved', resolution = 'fixed', resolved_at = ?1
             WHERE path = ?2 AND status = 'open'",
            params![resolved_at, path],
        )?;
        return Ok(count);
    }
    let placeholders: Vec<String> = (0..issue_types_still_open.len())
        .map(|i| format!("?{}", i + 3))
        .collect();
    let sql = format!(
        "UPDATE audit_issues
         SET status = 'resolved', resolution = 'fixed', resolved_at = ?1
         WHERE path = ?2 AND status = 'open' AND issue_type NOT IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_idx = 1;
    stmt.raw_bind_parameter(param_idx, resolved_at)?;
    param_idx += 1;
    stmt.raw_bind_parameter(param_idx, path)?;
    param_idx += 1;
    for it in issue_types_still_open {
        stmt.raw_bind_parameter(param_idx, *it)?;
        param_idx += 1;
    }
    let count = stmt.raw_execute()?;
    Ok(count)
}

pub fn get_audit_summary(conn: &Connection, scope: &str) -> Result<AuditSummary, rusqlite::Error> {
    let pattern = format!("{}%", escape_like(scope));
    let mut stmt = conn.prepare(
        "SELECT issue_type, status, COUNT(*) as cnt
         FROM audit_issues
         WHERE path LIKE ?1 ESCAPE '\\'
         GROUP BY issue_type, status
         ORDER BY issue_type, status",
    )?;
    let rows = stmt.query_map(params![pattern], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    let by_type_status: Vec<(String, String, i64)> = rows.collect::<Result<_, _>>()?;
    Ok(AuditSummary { by_type_status })
}

/// Query open audit issues of specific types in scope, returning (id, path, issue_type, detail).
pub fn get_open_issues_by_types(
    conn: &Connection,
    scope: &str,
    issue_types: &[&str],
) -> Result<Vec<AuditIssueRow>, rusqlite::Error> {
    if issue_types.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!("{}%", escape_like(scope));
    let placeholders: Vec<String> = (0..issue_types.len())
        .map(|i| format!("?{}", i + 2))
        .collect();
    let sql = format!(
        "SELECT id, path, issue_type, detail FROM audit_issues \
         WHERE path LIKE ?1 ESCAPE '\\' AND status = 'open' \
         AND issue_type IN ({})",
        placeholders.join(", ")
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut param_idx = 1;
    stmt.raw_bind_parameter(param_idx, &pattern)?;
    for it in issue_types {
        param_idx += 1;
        stmt.raw_bind_parameter(param_idx, *it)?;
    }
    let mut results = Vec::new();
    let mut rows = stmt.raw_query();
    while let Some(row) = rows.next()? {
        results.push((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
        ));
    }
    Ok(results)
}

/// Update the detail JSON of an audit issue by ID.
pub fn update_audit_issue_detail(
    conn: &Connection,
    id: i64,
    detail: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "UPDATE audit_issues SET detail = ?1 WHERE id = ?2",
        params![detail, id],
    )?;
    Ok(())
}

pub fn delete_missing_audit_files(
    conn: &Connection,
    scope: &str,
    existing_paths: &std::collections::HashSet<String>,
) -> Result<usize, rusqlite::Error> {
    const BATCH_SIZE: usize = 500;
    let pattern = format!("{}%", escape_like(scope));
    let mut deleted_count = 0usize;
    let mut last_path = String::new();

    loop {
        let mut stmt = conn.prepare(
            "SELECT path
             FROM audit_files
             WHERE path LIKE ?1 ESCAPE '\\' AND path > ?2
             ORDER BY path
             LIMIT ?3",
        )?;
        let batch_paths: Vec<String> = stmt
            .query_map(params![&pattern, &last_path, BATCH_SIZE as i64], |row| {
                row.get(0)
            })?
            .collect::<Result<_, _>>()?;
        if batch_paths.is_empty() {
            break;
        }

        let to_delete: Vec<&str> = batch_paths
            .iter()
            .filter(|p| !existing_paths.contains(p.as_str()))
            .map(|p| p.as_str())
            .collect();

        if !to_delete.is_empty() {
            let placeholders: String = (1..=to_delete.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!("DELETE FROM audit_files WHERE path IN ({placeholders})");
            let mut del_stmt = conn.prepare(&sql)?;
            for (i, path) in to_delete.iter().enumerate() {
                del_stmt.raw_bind_parameter(i + 1, *path)?;
            }
            deleted_count += del_stmt.raw_execute()?;
        }

        last_path = batch_paths
            .last()
            .expect("batch_paths non-empty when loop continues")
            .clone();
    }

    Ok(deleted_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let conn = open(path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    #[test]
    fn test_open_creates_schema() {
        let (_dir, conn) = open_temp_store();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.contains(&"enrichment_cache".to_string()));
        assert!(tables.contains(&"audio_analysis_cache".to_string()));
        assert!(tables.contains(&"broker_discogs_session".to_string()));
        assert!(tables.contains(&"audit_files".to_string()));
        assert!(tables.contains(&"audit_issues".to_string()));
    }

    #[test]
    fn test_open_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let path_str = path.to_str().unwrap();

        // Open twice — second open should not fail
        let conn1 = open(path_str).unwrap();
        drop(conn1);
        let conn2 = open(path_str).unwrap();
        let version: i32 = conn2
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 3);
    }

    #[test]
    fn test_open_accepts_bare_relative_filename_path() {
        use std::sync::{Mutex, OnceLock};

        struct CwdGuard(std::path::PathBuf);
        impl Drop for CwdGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }

        // set_current_dir is process-global, so serialize this test section.
        static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _lock = CWD_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("cwd lock poisoned");

        let original_cwd = std::env::current_dir().unwrap();
        let _restore_cwd = CwdGuard(original_cwd);
        let dir = tempfile::tempdir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();

        let db_name = "internal.sqlite3";
        let conn = open(db_name).unwrap();
        drop(conn);

        assert!(dir.path().join(db_name).is_file());
    }

    #[test]
    fn test_open_reports_parent_directory_creation_failure() {
        let dir = tempfile::tempdir().unwrap();
        let not_a_directory = dir.path().join("not-a-directory");
        std::fs::write(&not_a_directory, b"blocker").unwrap();
        let db_path = not_a_directory.join("test.sqlite3");

        let err = open(db_path.to_str().unwrap()).unwrap_err();
        match err {
            rusqlite::Error::SqliteFailure(_, Some(message)) => {
                assert!(message.contains("failed to create parent directory"));
                assert!(message.contains("not-a-directory"));
            }
            other => panic!("expected sqlite failure with context, got {other:?}"),
        }
    }

    #[test]
    fn test_open_repairs_missing_tables_when_user_version_is_current() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let path_str = path.to_str().unwrap();

        let conn = Connection::open(path_str).unwrap();
        conn.execute_batch("PRAGMA user_version = 3;").unwrap();
        drop(conn);

        let conn = open(path_str).unwrap();
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.contains(&"enrichment_cache".to_string()));
        assert!(tables.contains(&"audio_analysis_cache".to_string()));
        assert!(tables.contains(&"broker_discogs_session".to_string()));
        assert!(tables.contains(&"audit_files".to_string()));
        assert!(tables.contains(&"audit_issues".to_string()));
    }

    #[test]
    fn test_enrichment_cache_round_trip() {
        let (_dir, conn) = open_temp_store();

        // Write
        set_enrichment(
            &conn,
            "discogs",
            "burial",
            "archangel",
            Some("exact"),
            Some(r#"{"title":"Burial - Untrue","genres":["Electronic"]}"#),
        )
        .unwrap();

        // Read
        let entry = get_enrichment(&conn, "discogs", "burial", "archangel")
            .unwrap()
            .expect("should find cached entry");
        assert_eq!(entry.provider, "discogs");
        assert_eq!(entry.query_artist, "burial");
        assert_eq!(entry.query_title, "archangel");
        assert_eq!(entry.match_quality.as_deref(), Some("exact"));
        assert!(entry.response_json.unwrap().contains("Burial"));
        assert!(!entry.created_at.is_empty());
    }

    #[test]
    fn test_enrichment_cache_miss() {
        let (_dir, conn) = open_temp_store();
        let entry = get_enrichment(&conn, "discogs", "nobody", "nothing").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn test_enrichment_cache_upsert() {
        let (_dir, conn) = open_temp_store();

        set_enrichment(
            &conn,
            "discogs",
            "burial",
            "archangel",
            Some("fuzzy"),
            Some("old"),
        )
        .unwrap();
        set_enrichment(
            &conn,
            "discogs",
            "burial",
            "archangel",
            Some("exact"),
            Some("new"),
        )
        .unwrap();

        let entry = get_enrichment(&conn, "discogs", "burial", "archangel")
            .unwrap()
            .unwrap();
        assert_eq!(entry.match_quality.as_deref(), Some("exact"));
        assert_eq!(entry.response_json.as_deref(), Some("new"));
    }

    #[test]
    fn test_enrichment_cache_no_match() {
        let (_dir, conn) = open_temp_store();

        // Cache a "no match" result
        set_enrichment(&conn, "discogs", "nobody", "nothing", Some("none"), None).unwrap();

        let entry = get_enrichment(&conn, "discogs", "nobody", "nothing")
            .unwrap()
            .unwrap();
        assert_eq!(entry.match_quality.as_deref(), Some("none"));
        assert!(entry.response_json.is_none());
    }

    #[test]
    fn test_audio_analysis_cache_round_trip() {
        let (_dir, conn) = open_temp_store();

        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            12345678,
            1700000000,
            "1.0.0",
            r#"{"bpm":128.0,"key":"Am"}"#,
        )
        .unwrap();

        let entry = get_audio_analysis(&conn, "/music/track.flac", "stratum-dsp")
            .unwrap()
            .expect("should find cached entry");
        assert_eq!(entry.file_path, "/music/track.flac");
        assert_eq!(entry.analyzer, "stratum-dsp");
        assert_eq!(entry.file_size, 12345678);
        assert_eq!(entry.file_mtime, 1700000000);
        assert_eq!(entry.analysis_version, "1.0.0");
        assert!(entry.features_json.contains("128.0"));
    }

    #[test]
    fn test_audio_analysis_cache_miss() {
        let (_dir, conn) = open_temp_store();
        let entry = get_audio_analysis(&conn, "/no/such/file.flac", "stratum-dsp").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn test_audio_analysis_cache_upsert() {
        let (_dir, conn) = open_temp_store();

        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            100,
            200,
            "1.0.0",
            "old",
        )
        .unwrap();
        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            100,
            300,
            "1.1.0",
            "new",
        )
        .unwrap();

        let entry = get_audio_analysis(&conn, "/music/track.flac", "stratum-dsp")
            .unwrap()
            .unwrap();
        assert_eq!(entry.file_mtime, 300);
        assert_eq!(entry.analysis_version, "1.1.0");
        assert_eq!(entry.features_json, "new");
    }

    #[test]
    fn test_broker_discogs_session_round_trip() {
        let (_dir, conn) = open_temp_store();

        set_broker_discogs_session(
            &conn,
            "https://broker.example.com",
            "session-token-1",
            1_800_000_000,
        )
        .unwrap();

        let row = get_broker_discogs_session(&conn, "https://broker.example.com")
            .unwrap()
            .expect("broker session should exist");
        assert_eq!(row.broker_url, "https://broker.example.com");
        assert_eq!(row.session_token, "session-token-1");
        assert_eq!(row.expires_at, 1_800_000_000);
        assert!(!row.created_at.is_empty());
        assert!(!row.updated_at.is_empty());

        set_broker_discogs_session(
            &conn,
            "https://broker.example.com",
            "session-token-2",
            1_900_000_000,
        )
        .unwrap();
        let row = get_broker_discogs_session(&conn, "https://broker.example.com")
            .unwrap()
            .expect("broker session should still exist");
        assert_eq!(row.session_token, "session-token-2");
        assert_eq!(row.expires_at, 1_900_000_000);

        clear_broker_discogs_session(&conn, "https://broker.example.com").unwrap();
        let missing = get_broker_discogs_session(&conn, "https://broker.example.com").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn test_audit_file_round_trip() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(
            &conn,
            "/music/track.flac",
            "2026-02-25T12:00:00Z",
            "2026-02-20T10:00:00Z",
            12345,
        )
        .unwrap();

        let entry = get_audit_file(&conn, "/music/track.flac")
            .unwrap()
            .expect("should find audit file");
        assert_eq!(entry.path, "/music/track.flac");
        assert_eq!(entry.last_audited, "2026-02-25T12:00:00Z");
        assert_eq!(entry.file_mtime, "2026-02-20T10:00:00Z");
        assert_eq!(entry.file_size, 12345);
    }

    #[test]
    fn test_audit_file_upsert() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t1", "m1", 100).unwrap();
        upsert_audit_file(&conn, "/music/track.flac", "t2", "m2", 200).unwrap();

        let entry = get_audit_file(&conn, "/music/track.flac").unwrap().unwrap();
        assert_eq!(entry.last_audited, "t2");
        assert_eq!(entry.file_mtime, "m2");
        assert_eq!(entry.file_size, 200);
    }

    #[test]
    fn test_audit_file_miss() {
        let (_dir, conn) = open_temp_store();
        let entry = get_audit_file(&conn, "/no/such/file").unwrap();
        assert!(entry.is_none());
    }

    #[test]
    fn test_audit_files_in_scope() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/a/1.flac", "t", "m", 100).unwrap();
        upsert_audit_file(&conn, "/music/a/2.flac", "t", "m", 200).unwrap();
        upsert_audit_file(&conn, "/music/b/1.flac", "t", "m", 300).unwrap();

        let files = get_audit_files_in_scope(&conn, "/music/a/").unwrap();
        assert_eq!(files.len(), 2);

        let files = get_audit_files_in_scope(&conn, "/music/").unwrap();
        assert_eq!(files.len(), 3);

        let files = get_audit_files_in_scope(&conn, "/other/").unwrap();
        assert_eq!(files.len(), 0);
    }

    #[test]
    fn test_audit_issue_round_trip() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.wav", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.wav",
            "WAV_TAG3_MISSING",
            Some(r#"{"fields":["artist"]}"#),
            "open",
            "2026-02-25T12:00:00Z",
        )
        .unwrap();

        let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "/music/track.wav");
        assert_eq!(issues[0].issue_type, "WAV_TAG3_MISSING");
        assert_eq!(issues[0].status, "open");
        assert!(issues[0].detail.as_ref().unwrap().contains("artist"));
    }

    #[test]
    fn test_audit_issue_upsert_preserves_accepted() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t1").unwrap();

        // Simulate user accepting
        resolve_audit_issues(
            &conn,
            &[1],
            crate::audit::Resolution::AcceptedAsIs,
            None,
            "t2",
        )
        .unwrap();

        // Re-scan upserts the same issue — should preserve accepted status
        upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t3").unwrap();

        let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
        assert_eq!(issue.status, "accepted");
    }

    #[test]
    fn test_audit_issue_reopen_clears_stale_resolution() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            None,
            "open",
            "t1",
        )
        .unwrap();

        // Resolve the issue
        resolve_audit_issues(
            &conn,
            &[1],
            crate::audit::Resolution::Fixed,
            Some("fixed upstream"),
            "t2",
        )
        .unwrap();

        let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
        assert_eq!(issue.status, "resolved");
        assert_eq!(issue.resolution.as_deref(), Some("fixed"));
        assert_eq!(issue.note.as_deref(), Some("fixed upstream"));
        assert_eq!(issue.resolved_at.as_deref(), Some("t2"));

        // Re-scan detects the issue again — should reopen and clear stale fields
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            Some("d2"),
            "open",
            "t3",
        )
        .unwrap();

        let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
        assert_eq!(issue.status, "open");
        assert!(
            issue.resolution.is_none(),
            "resolution should be cleared on reopen"
        );
        assert!(issue.note.is_none(), "note should be cleared on reopen");
        assert!(
            issue.resolved_at.is_none(),
            "resolved_at should be cleared on reopen"
        );
    }

    #[test]
    fn test_audit_issue_unique_constraint() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            Some("d1"),
            "open",
            "t1",
        )
        .unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            Some("d2"),
            "open",
            "t2",
        )
        .unwrap();

        // Should still be only one issue, with updated detail
        let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].detail.as_deref(), Some("d2"));
    }

    #[test]
    fn test_audit_cascade_delete() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            None,
            "open",
            "t1",
        )
        .unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_TITLE",
            None,
            "open",
            "t1",
        )
        .unwrap();

        let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 2);

        delete_audit_file(&conn, "/music/track.flac").unwrap();

        let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 0);
    }

    #[test]
    fn test_audit_resolve_issues() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            None,
            "open",
            "t1",
        )
        .unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_TITLE",
            None,
            "open",
            "t1",
        )
        .unwrap();

        let count = resolve_audit_issues(
            &conn,
            &[1],
            crate::audit::Resolution::AcceptedAsIs,
            Some("intentional"),
            "t2",
        )
        .unwrap();
        assert_eq!(count, 1);

        let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
        assert_eq!(issue.status, "accepted");
        assert_eq!(issue.resolution.as_deref(), Some("accepted_as_is"));
        assert_eq!(issue.note.as_deref(), Some("intentional"));

        // Issue 2 remains open
        let issue2 = get_audit_issue_by_id(&conn, 2).unwrap().unwrap();
        assert_eq!(issue2.status, "open");
    }

    #[test]
    fn test_audit_query_filters() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
        upsert_audit_file(&conn, "/music/b.wav", "t", "m", 200).unwrap();
        upsert_audit_issue(&conn, "/music/a.flac", "EMPTY_ARTIST", None, "open", "t1").unwrap();
        upsert_audit_issue(
            &conn,
            "/music/b.wav",
            "WAV_TAG3_MISSING",
            None,
            "open",
            "t1",
        )
        .unwrap();
        resolve_audit_issues(
            &conn,
            &[1],
            crate::audit::Resolution::AcceptedAsIs,
            None,
            "t2",
        )
        .unwrap();

        // Filter by status
        let open = get_audit_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].issue_type, "WAV_TAG3_MISSING");

        // Filter by issue_type
        let wav =
            get_audit_issues(&conn, "/music/", None, Some("WAV_TAG3_MISSING"), 100, 0).unwrap();
        assert_eq!(wav.len(), 1);

        // Filter by both
        let both = get_audit_issues(
            &conn,
            "/music/",
            Some("accepted"),
            Some("EMPTY_ARTIST"),
            100,
            0,
        )
        .unwrap();
        assert_eq!(both.len(), 1);
    }

    #[test]
    fn test_audit_summary() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
        upsert_audit_file(&conn, "/music/b.wav", "t", "m", 200).unwrap();
        upsert_audit_issue(&conn, "/music/a.flac", "EMPTY_ARTIST", None, "open", "t1").unwrap();
        upsert_audit_issue(
            &conn,
            "/music/b.wav",
            "WAV_TAG3_MISSING",
            None,
            "open",
            "t1",
        )
        .unwrap();

        let summary = get_audit_summary(&conn, "/music/").unwrap();
        assert_eq!(summary.by_type_status.len(), 2);
    }

    #[test]
    fn test_audit_delete_missing_files() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
        upsert_audit_file(&conn, "/music/b.flac", "t", "m", 200).unwrap();
        upsert_audit_file(&conn, "/music/c.flac", "t", "m", 300).unwrap();

        let existing: std::collections::HashSet<String> =
            ["/music/a.flac".to_string()].into_iter().collect();
        let count = delete_missing_audit_files(&conn, "/music/", &existing).unwrap();
        assert_eq!(count, 2);

        let files = get_audit_files_in_scope(&conn, "/music/").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/music/a.flac");
    }

    #[test]
    fn test_audit_delete_missing_files_keyset_batches() {
        let (_dir, conn) = open_temp_store();

        for i in 0..1205 {
            let path = format!("/music/{i:04}.flac");
            upsert_audit_file(&conn, &path, "t", "m", i).unwrap();
        }

        let existing: std::collections::HashSet<String> = [
            "/music/0000.flac".to_string(),
            "/music/0600.flac".to_string(),
            "/music/1204.flac".to_string(),
        ]
        .into_iter()
        .collect();

        let count = delete_missing_audit_files(&conn, "/music/", &existing).unwrap();
        assert_eq!(count, 1202);

        let mut remaining = get_audit_files_in_scope(&conn, "/music/")
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect::<Vec<_>>();
        remaining.sort();

        assert_eq!(
            remaining,
            vec![
                "/music/0000.flac".to_string(),
                "/music/0600.flac".to_string(),
                "/music/1204.flac".to_string(),
            ]
        );
    }

    #[test]
    fn test_audit_mark_issues_resolved_for_path() {
        let (_dir, conn) = open_temp_store();

        upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_ARTIST",
            None,
            "open",
            "t1",
        )
        .unwrap();
        upsert_audit_issue(
            &conn,
            "/music/track.flac",
            "EMPTY_TITLE",
            None,
            "open",
            "t1",
        )
        .unwrap();
        upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t1").unwrap();

        // Mark resolved except GENRE_SET which is still detected
        let count = mark_issues_resolved_for_path(&conn, "/music/track.flac", &["GENRE_SET"], "t2")
            .unwrap();
        assert_eq!(count, 2);

        let open = get_audit_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].issue_type, "GENRE_SET");
    }

    // Verify SQL LIKE wildcards in scope paths are escaped and matched literally.
    #[test]
    fn test_audit_files_in_scope_escapes_like_wildcards() {
        let (_dir, conn) = open_temp_store();
        // Path containing SQL LIKE wildcards
        upsert_audit_file(&conn, "/music/100%_done/track.flac", "t", "m", 100).unwrap();
        upsert_audit_file(&conn, "/music/100X_done/track.flac", "t", "m", 200).unwrap();

        // Scope with % — should only match the exact prefix, not wildcard-expand
        let files = get_audit_files_in_scope(&conn, "/music/100%_done/").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/music/100%_done/track.flac");
    }

    #[test]
    fn test_batch_enrichment_existence() {
        let (_dir, conn) = open_temp_store();
        set_enrichment(&conn, "discogs", "artist_a", "title_1", Some("exact"), Some("{}")).unwrap();
        set_enrichment(&conn, "discogs", "artist_a", "title_2", Some("exact"), Some("{}")).unwrap();
        set_enrichment(&conn, "beatport", "artist_b", "title_3", None, Some("{}")).unwrap();

        // Discogs: artist_a has two titles
        let discogs = batch_enrichment_existence(&conn, "discogs", &["artist_a", "artist_b"]).unwrap();
        assert!(discogs.contains(&("artist_a".to_string(), "title_1".to_string())));
        assert!(discogs.contains(&("artist_a".to_string(), "title_2".to_string())));
        // artist_b has no discogs entry
        assert!(!discogs.contains(&("artist_b".to_string(), "title_3".to_string())));

        // Beatport: artist_b has one title
        let beatport = batch_enrichment_existence(&conn, "beatport", &["artist_a", "artist_b"]).unwrap();
        assert!(beatport.contains(&("artist_b".to_string(), "title_3".to_string())));
        assert!(!beatport.contains(&("artist_a".to_string(), "title_1".to_string())));

        // Empty input
        let empty = batch_enrichment_existence(&conn, "discogs", &[]).unwrap();
        assert!(empty.is_empty());

        // Unknown artist
        let unknown = batch_enrichment_existence(&conn, "discogs", &["nobody"]).unwrap();
        assert!(unknown.is_empty());
    }

    #[test]
    fn test_batch_audio_analysis_existence() {
        let (_dir, conn) = open_temp_store();
        set_audio_analysis(&conn, "/music/a.flac", "stratum-dsp", 100, 1000, "1.0", "{}").unwrap();
        set_audio_analysis(&conn, "/music/a.flac", "essentia", 100, 1000, "1.0", "{}").unwrap();
        set_audio_analysis(&conn, "/music/b.flac", "stratum-dsp", 200, 2000, "1.0", "{}").unwrap();

        let result = batch_audio_analysis_existence(&conn, &["/music/a.flac", "/music/b.flac", "/music/c.flac"]).unwrap();
        assert_eq!(result.len(), 3);
        assert!(result.contains(&("/music/a.flac".to_string(), "stratum-dsp".to_string())));
        assert!(result.contains(&("/music/a.flac".to_string(), "essentia".to_string())));
        assert!(result.contains(&("/music/b.flac".to_string(), "stratum-dsp".to_string())));
        // c.flac not cached
        assert!(!result.contains(&("/music/c.flac".to_string(), "stratum-dsp".to_string())));

        // Empty input
        let empty = batch_audio_analysis_existence(&conn, &[]).unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_batch_audio_analysis_existence_chunking() {
        let (_dir, conn) = open_temp_store();
        let paths: Vec<String> = (0..1000).map(|i| format!("/music/track_{i}.flac")).collect();
        for p in &paths {
            set_audio_analysis(&conn, p, "stratum-dsp", 100, 1000, "1.0", "{}").unwrap();
        }

        let path_refs: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
        let result = batch_audio_analysis_existence(&conn, &path_refs).unwrap();
        assert_eq!(result.len(), 1000);
        for p in &paths {
            assert!(result.contains(&(p.clone(), "stratum-dsp".to_string())));
        }
    }

    #[test]
    fn test_batch_enrichment_existence_chunking() {
        let (_dir, conn) = open_temp_store();
        // Seed 1000 artists to exercise multi-chunk path (chunk size = 899).
        let artists: Vec<String> = (0..1000).map(|i| format!("artist_{i}")).collect();
        for a in &artists {
            set_enrichment(&conn, "discogs", a, "title", Some("exact"), Some("{}")).unwrap();
        }

        let artist_refs: Vec<&str> = artists.iter().map(|s| s.as_str()).collect();
        let result = batch_enrichment_existence(&conn, "discogs", &artist_refs).unwrap();
        assert_eq!(result.len(), 1000);
        for a in &artists {
            assert!(result.contains(&(a.clone(), "title".to_string())));
        }
    }
}
