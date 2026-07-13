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
