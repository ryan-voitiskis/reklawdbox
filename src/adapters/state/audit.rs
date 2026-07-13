use rusqlite::{Connection, params};

use crate::domain::audit::Resolution;

/// (id, path, issue_type, detail)
pub type AuditIssueRow = (i64, String, String, Option<String>);

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

// ---------------------------------------------------------------------------
// Audit state
// ---------------------------------------------------------------------------

pub struct AuditFile {
    pub path: String,
    #[allow(dead_code)]
    pub last_audited: String,
    /// Stored in the legacy SQL `file_mtime` column for backwards compatibility.
    pub freshness_key: String,
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
    freshness_key: &str,
    file_size: i64,
) -> Result<(), rusqlite::Error> {
    // `file_mtime` is the compatibility container for the opaque audit freshness key.
    conn.execute(
        "INSERT INTO audit_files (path, last_audited, file_mtime, file_size)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path)
         DO UPDATE SET last_audited = ?2, file_mtime = ?3, file_size = ?4",
        params![path, last_audited, freshness_key, file_size],
    )?;
    Ok(())
}

pub fn get_audit_files_in_scope(
    conn: &Connection,
    scope: &str,
) -> Result<Vec<AuditFile>, rusqlite::Error> {
    let pattern = format!("{}%", escape_like(scope));
    let mut stmt = conn.prepare_cached(
        "SELECT path, last_audited, file_mtime, file_size
         FROM audit_files WHERE path LIKE ?1 ESCAPE '\\'",
    )?;
    let rows = stmt.query_map(params![pattern], |row| {
        Ok(AuditFile {
            path: row.get(0)?,
            last_audited: row.get(1)?,
            freshness_key: row.get(2)?,
            file_size: row.get(3)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
pub fn get_audit_file(conn: &Connection, path: &str) -> Result<Option<AuditFile>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT path, last_audited, file_mtime, file_size
         FROM audit_files WHERE path = ?1",
    )?;
    let mut rows = stmt.query_map(params![path], |row| {
        Ok(AuditFile {
            path: row.get(0)?,
            last_audited: row.get(1)?,
            freshness_key: row.get(2)?,
            file_size: row.get(3)?,
        })
    })?;
    rows.next().transpose()
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
    param_values.push(Box::new(pattern));
    param_values.push(Box::new(limit));
    param_values.push(Box::new(offset));

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
    let mut stmt = conn.prepare_cached(
        "SELECT id, path, issue_type, detail, status, resolution, note, created_at, resolved_at
         FROM audit_issues WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], map_audit_issue)?;
    rows.next().transpose()
}

pub fn resolve_audit_issues(
    conn: &Connection,
    ids: &[i64],
    resolution: Resolution,
    note: Option<&str>,
    resolved_at: &str,
) -> Result<usize, rusqlite::Error> {
    let status = resolution.status().as_str();
    let resolution_str = resolution.as_str();
    let tx = conn.unchecked_transaction()?;
    let mut count = 0usize;
    for id in ids {
        count += tx.execute(
            "UPDATE audit_issues
             SET status = ?1, resolution = ?2, note = COALESCE(?3, note), resolved_at = ?4
             WHERE id = ?5 AND status IN ('open', 'deferred', 'accepted')",
            params![status, resolution_str, note, resolved_at, id],
        )?;
    }
    tx.commit()?;
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
             WHERE path = ?2 AND status IN ('open', 'deferred')",
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
         WHERE path = ?2 AND status IN ('open', 'deferred') AND issue_type NOT IN ({})",
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
    let mut stmt = conn.prepare_cached(
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
        results.push((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?));
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
        let mut stmt = conn.prepare_cached(
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
            .map(std::string::String::as_str)
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
