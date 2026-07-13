//! Audit issue query, resolution, and summary orchestration.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::adapters::state;
use crate::domain::audit::{AuditStatus, Resolution};

use super::scan::{enforce_trailing_slash, now_iso};

pub(crate) struct StatusCounts {
    pub(crate) open: i64,
    pub(crate) resolved: i64,
    pub(crate) accepted: i64,
    pub(crate) deferred: i64,
}

pub(crate) fn aggregate_status_counts(summary: &state::AuditSummary) -> StatusCounts {
    let mut counts = StatusCounts {
        open: 0,
        resolved: 0,
        accepted: 0,
        deferred: 0,
    };
    for (_, status, count) in &summary.by_type_status {
        match AuditStatus::from_str(status) {
            Some(AuditStatus::Open) => counts.open += count,
            Some(AuditStatus::Resolved) => counts.resolved += count,
            Some(AuditStatus::Accepted) => counts.accepted += count,
            Some(AuditStatus::Deferred) => counts.deferred += count,
            None => {}
        }
    }
    counts
}

// ---------------------------------------------------------------------------
// Query, resolve, summary — thin wrappers
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct IssueRecord {
    pub id: i64,
    pub path: String,
    pub issue_type: String,
    pub detail: Option<serde_json::Value>,
    pub status: String,
    pub resolution: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

pub(crate) fn store_issue_to_record(issue: state::AuditIssue) -> IssueRecord {
    let detail = issue
        .detail
        .as_deref()
        .and_then(|d| match serde_json::from_str(d) {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::warn!("issue {}: detail JSON parse failed: {e}", issue.id);
                None
            }
        });
    IssueRecord {
        id: issue.id,
        path: issue.path,
        issue_type: issue.issue_type,
        detail,
        status: issue.status,
        resolution: issue.resolution,
        note: issue.note,
        created_at: issue.created_at,
        resolved_at: issue.resolved_at,
    }
}

pub fn query_issues(
    conn: &Connection,
    scope: &str,
    status: Option<&str>,
    issue_type: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<Vec<IssueRecord>, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let issues = state::get_audit_issues(conn, &scope, status, issue_type, limit, offset)
        .map_err(|e| format!("DB error: {e}"))?;
    Ok(issues.into_iter().map(store_issue_to_record).collect())
}

pub fn resolve_issues(
    conn: &Connection,
    ids: &[i64],
    resolution: &str,
    note: Option<&str>,
) -> Result<usize, String> {
    let parsed_resolution = Resolution::from_str(resolution)
        .filter(|r| !matches!(r, Resolution::Fixed))
        .ok_or_else(|| {
            format!(
                "Invalid resolution \"{resolution}\". Must be one of: \
                 accepted_as_is, wont_fix, deferred"
            )
        })?;
    let now = now_iso();
    state::resolve_audit_issues(conn, ids, parsed_resolution, note, &now)
        .map_err(|e| format!("DB error: {e}"))
}

#[derive(Debug, Serialize)]
pub struct SummaryReport {
    pub scope: String,
    pub by_type: HashMap<String, HashMap<String, i64>>,
    pub total_open: i64,
    pub total_resolved: i64,
    pub total_accepted: i64,
    pub total_deferred: i64,
}

pub fn get_summary(conn: &Connection, scope: &str) -> Result<SummaryReport, String> {
    let scope = enforce_trailing_slash(scope);
    if scope == "/" {
        return Err("Scope must not be empty or root (/)".to_string());
    }
    let summary = state::get_audit_summary(conn, &scope).map_err(|e| format!("DB error: {e}"))?;

    let mut by_type: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for (issue_type, status, count) in &summary.by_type_status {
        by_type
            .entry(issue_type.clone())
            .or_default()
            .insert(status.clone(), *count);
    }

    let counts = aggregate_status_counts(&summary);

    Ok(SummaryReport {
        scope,
        by_type,
        total_open: counts.open,
        total_resolved: counts.resolved,
        total_accepted: counts.accepted,
        total_deferred: counts.deferred,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::audit::scan::{audit_freshness_key, is_successful_audit_freshness_key};
    use crate::domain::audit::AuditContext;

    #[test]
    fn audit_workflow_preserves_freshness_and_resolution() {
        let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(123);
        let album_key = audit_freshness_key(Some(modified), AuditContext::AlbumTrack).unwrap();
        let loose_key = audit_freshness_key(Some(modified), AuditContext::LooseTrack).unwrap();
        assert_ne!(album_key, loose_key);
        assert!(is_successful_audit_freshness_key(&album_key));

        let (_directory, store) = seed_query_resolve_db();
        let open = query_issues(&store, "/music/a", Some("open"), None, 100, 0).unwrap();
        let issue_id = open[0].id;
        assert_eq!(
            resolve_issues(&store, &[issue_id], "deferred", Some("later")).unwrap(),
            1
        );
        let deferred = query_issues(&store, "/music/a", Some("deferred"), None, 100, 0).unwrap();
        assert_eq!(deferred.len(), 1);
        assert_eq!(deferred[0].note.as_deref(), Some("later"));
    }

    // -- query_issues & resolve_issues --

    fn seed_query_resolve_db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite3");
        let conn = state::open(db_path.to_str().unwrap()).unwrap();

        state::upsert_audit_file(&conn, "/music/a/track1.flac", "t1", "m1", 100).unwrap();
        state::upsert_audit_file(&conn, "/music/b/track2.wav", "t1", "m1", 200).unwrap();

        // Issues for track1 (two open, one accepted)
        state::upsert_audit_issue(
            &conn,
            "/music/a/track1.flac",
            "EMPTY_ARTIST",
            None,
            "open",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        state::upsert_audit_issue(
            &conn,
            "/music/a/track1.flac",
            "GENRE_SET",
            None,
            "open",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
        // Accept the GENRE_SET issue (id=2)
        state::resolve_audit_issues(
            &conn,
            &[2],
            Resolution::AcceptedAsIs,
            Some("intended"),
            "2026-01-02T00:00:00Z",
        )
        .unwrap();

        // Issues for track2 (one open WAV_TAG3_MISSING)
        state::upsert_audit_issue(
            &conn,
            "/music/b/track2.wav",
            "WAV_TAG3_MISSING",
            Some(r#"{"fields":["artist"]}"#),
            "open",
            "2026-01-01T00:00:00Z",
        )
        .unwrap();

        (dir, conn)
    }

    #[test]
    fn query_issues_all_in_scope() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/music/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 3);
    }

    #[test]
    fn query_issues_narrow_scope() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/music/a/", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.path.starts_with("/music/a/")));
    }

    #[test]
    fn query_issues_filter_by_status_open() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
        assert_eq!(issues.len(), 2);
        assert!(issues.iter().all(|i| i.status == "open"));
    }

    #[test]
    fn query_issues_filter_by_status_accepted() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/music/", Some("accepted"), None, 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].issue_type, "GENRE_SET");
        assert_eq!(issues[0].status, "accepted");
    }

    #[test]
    fn query_issues_filter_by_issue_type() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues =
            query_issues(&conn, "/music/", None, Some("WAV_TAG3_MISSING"), 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "/music/b/track2.wav");
    }

    #[test]
    fn query_issues_filter_by_status_and_type() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues =
            query_issues(&conn, "/music/", Some("open"), Some("EMPTY_ARTIST"), 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].path, "/music/a/track1.flac");
        assert_eq!(issues[0].issue_type, "EMPTY_ARTIST");
    }

    #[test]
    fn query_issues_empty_result_for_non_existent_scope() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/other/", None, None, 100, 0).unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn query_issues_empty_result_for_non_matching_type() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues = query_issues(&conn, "/music/", None, Some("BAD_FILENAME"), 100, 0).unwrap();
        assert!(issues.is_empty());
    }

    #[test]
    fn query_issues_rejects_root_scope() {
        let (_dir, conn) = seed_query_resolve_db();
        let err = query_issues(&conn, "/", None, None, 100, 0).unwrap_err();
        assert!(err.contains("root"));
    }

    #[test]
    fn query_issues_rejects_empty_scope() {
        let (_dir, conn) = seed_query_resolve_db();
        // Empty scope normalizes to "/" via enforce_trailing_slash
        let err = query_issues(&conn, "", None, None, 100, 0).unwrap_err();
        assert!(err.contains("root"));
    }

    #[test]
    fn query_issues_limit_and_offset() {
        let (_dir, conn) = seed_query_resolve_db();
        let first = query_issues(&conn, "/music/", None, None, 1, 0).unwrap();
        assert_eq!(first.len(), 1);

        let second = query_issues(&conn, "/music/", None, None, 1, 1).unwrap();
        assert_eq!(second.len(), 1);
        assert_ne!(first[0].id, second[0].id);

        let beyond = query_issues(&conn, "/music/", None, None, 100, 100).unwrap();
        assert!(beyond.is_empty());
    }

    #[test]
    fn query_issues_detail_parsed_as_json() {
        let (_dir, conn) = seed_query_resolve_db();
        let issues =
            query_issues(&conn, "/music/b/", None, Some("WAV_TAG3_MISSING"), 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        let detail = issues[0].detail.as_ref().expect("detail should be parsed");
        assert_eq!(detail["fields"][0], "artist");
    }

    #[test]
    fn query_issues_adds_trailing_slash() {
        let (_dir, conn) = seed_query_resolve_db();
        // Scope without trailing slash should still work
        let issues = query_issues(&conn, "/music/a", None, None, 100, 0).unwrap();
        assert_eq!(issues.len(), 2);
    }

    // -- resolve_issues tests --

    #[test]
    fn resolve_issues_accepted_as_is() {
        let (_dir, conn) = seed_query_resolve_db();
        // Issue id=1 is open EMPTY_ARTIST
        let count = resolve_issues(&conn, &[1], "accepted_as_is", Some("ok")).unwrap();
        assert_eq!(count, 1);

        let issues = query_issues(&conn, "/music/a/", Some("accepted"), None, 100, 0).unwrap();
        // id=1 (EMPTY_ARTIST) + id=2 (GENRE_SET) are both accepted now
        assert_eq!(issues.len(), 2);

        let resolved = issues.iter().find(|i| i.id == 1).unwrap();
        assert_eq!(resolved.status, "accepted");
        assert_eq!(resolved.resolution.as_deref(), Some("accepted_as_is"));
        assert_eq!(resolved.note.as_deref(), Some("ok"));
        assert!(resolved.resolved_at.is_some());
    }

    #[test]
    fn resolve_issues_wont_fix() {
        let (_dir, conn) = seed_query_resolve_db();
        let count = resolve_issues(&conn, &[1], "wont_fix", None).unwrap();
        assert_eq!(count, 1);

        let issues = query_issues(&conn, "/music/a/", Some("accepted"), None, 100, 0).unwrap();
        let resolved = issues.iter().find(|i| i.id == 1).unwrap();
        assert_eq!(resolved.resolution.as_deref(), Some("wont_fix"));
    }

    #[test]
    fn resolve_issues_deferred() {
        let (_dir, conn) = seed_query_resolve_db();
        let count = resolve_issues(&conn, &[1], "deferred", Some("later")).unwrap();
        assert_eq!(count, 1);

        let issues = query_issues(&conn, "/music/a/", Some("deferred"), None, 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, 1);
        assert_eq!(issues[0].status, "deferred");
        assert_eq!(issues[0].resolution.as_deref(), Some("deferred"));
        assert_eq!(issues[0].note.as_deref(), Some("later"));
    }

    #[test]
    fn resolve_issues_rejects_fixed_resolution() {
        let (_dir, conn) = seed_query_resolve_db();
        let err = resolve_issues(&conn, &[1], "fixed", None).unwrap_err();
        assert!(err.contains("Invalid resolution"));
    }

    #[test]
    fn resolve_issues_rejects_unknown_resolution() {
        let (_dir, conn) = seed_query_resolve_db();
        let err = resolve_issues(&conn, &[1], "banana", None).unwrap_err();
        assert!(err.contains("Invalid resolution"));
    }

    #[test]
    fn resolve_issues_multiple_ids() {
        let (_dir, conn) = seed_query_resolve_db();
        // ids 1 (EMPTY_ARTIST, open) and 3 (WAV_TAG3_MISSING, open)
        let count = resolve_issues(&conn, &[1, 3], "accepted_as_is", None).unwrap();
        assert_eq!(count, 2);

        let open = query_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
        assert!(open.is_empty());
    }

    #[test]
    fn resolve_issues_non_existent_id_returns_zero() {
        let (_dir, conn) = seed_query_resolve_db();
        let count = resolve_issues(&conn, &[999], "accepted_as_is", None).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn resolve_issues_empty_ids_returns_zero() {
        let (_dir, conn) = seed_query_resolve_db();
        let count = resolve_issues(&conn, &[], "accepted_as_is", None).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn resolve_issues_already_resolved_can_be_re_resolved() {
        let (_dir, conn) = seed_query_resolve_db();
        // id=2 is already accepted — re-resolving with deferred should work
        let count = resolve_issues(&conn, &[2], "deferred", Some("revisit")).unwrap();
        assert_eq!(count, 1);

        let issues = query_issues(&conn, "/music/a/", Some("deferred"), None, 100, 0).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].id, 2);
        assert_eq!(issues[0].note.as_deref(), Some("revisit"));
    }
}
