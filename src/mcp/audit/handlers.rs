use std::collections::HashSet;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::audit;
use crate::domain::audit::IssueType;
use crate::mcp::{AuditOperation, mcp_internal_error, ok_json};

pub(in crate::mcp) async fn handle_audit_state(
    store_path: String,
    rekordbox_db_path: Option<String>,
    params: AuditOperation,
) -> Result<CallToolResult, McpError> {
    match params {
        AuditOperation::Scan {
            path_prefix,
            revalidate,
            skip_issue_types,
        } => {
            let revalidate = revalidate.unwrap_or(false);
            let skip: HashSet<IssueType> = skip_issue_types
                .unwrap_or_default()
                .iter()
                .filter_map(|s| s.parse::<IssueType>().ok())
                .collect();

            let summary = tokio::task::spawn_blocking(move || {
                let conn = store::open(&store_path)
                    .map_err(|e| format!("Failed to open internal store: {e}"))?;

                let imported = rekordbox_db_path.and_then(|db_path| match db::open(&db_path) {
                    Ok(rb_conn) => match db::paths_imported_in_scope(&rb_conn, &path_prefix) {
                        Ok(set) => Some(set),
                        Err(e) => {
                            tracing::warn!("Failed to query imported paths: {e}");
                            None
                        }
                    },
                    Err(e) => {
                        tracing::warn!("Failed to open Rekordbox DB for audit: {e}");
                        None
                    }
                });

                audit::scan(&conn, &path_prefix, revalidate, &skip, imported.as_ref())
            })
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
            .map_err(mcp_internal_error)?;

            ok_json(&summary)
        }

        AuditOperation::QueryIssues {
            path_prefix,
            status,
            issue_type,
            limit,
            offset,
        } => {
            let limit = limit.unwrap_or(100);
            let offset = offset.unwrap_or(0);

            let issues = tokio::task::spawn_blocking(move || {
                let conn = store::open(&store_path)
                    .map_err(|e| format!("Failed to open internal store: {e}"))?;
                audit::query_issues(
                    &conn,
                    &path_prefix,
                    status.as_deref(),
                    issue_type.as_deref(),
                    limit,
                    offset,
                )
            })
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
            .map_err(mcp_internal_error)?;

            ok_json(&issues)
        }

        AuditOperation::ResolveIssues {
            issue_ids,
            resolution,
            note,
        } => {
            let count = tokio::task::spawn_blocking(move || {
                let conn = store::open(&store_path)
                    .map_err(|e| format!("Failed to open internal store: {e}"))?;
                audit::resolve_issues(&conn, &issue_ids, &resolution, note.as_deref())
            })
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
            .map_err(mcp_internal_error)?;

            let json = serde_json::json!({ "resolved": count });
            ok_json(&json)
        }

        AuditOperation::GetSummary { path_prefix } => {
            let summary = tokio::task::spawn_blocking(move || {
                let conn = store::open(&store_path)
                    .map_err(|e| format!("Failed to open internal store: {e}"))?;
                audit::get_summary(&conn, &path_prefix)
            })
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
            .map_err(mcp_internal_error)?;

            ok_json(&summary)
        }
    }
}
