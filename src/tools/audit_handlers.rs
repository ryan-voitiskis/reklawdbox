use std::collections::HashSet;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::audit;
use crate::store;

pub(super) async fn handle_audit_state(
    store_path: String,
    params: AuditOperation,
) -> Result<CallToolResult, McpError> {
    match params {
        AuditOperation::Scan {
            path_prefix,
            revalidate,
            skip_issue_types,
        } => {
            let revalidate = revalidate.unwrap_or(false);
            let skip: HashSet<audit::IssueType> = skip_issue_types
                .unwrap_or_default()
                .iter()
                .filter_map(|s| s.parse::<audit::IssueType>().ok())
                .collect();

            let summary = tokio::task::spawn_blocking(move || {
                let conn = store::open(&store_path)
                    .map_err(|e| format!("Failed to open internal store: {e}"))?;
                audit::scan(&conn, &path_prefix, revalidate, &skip)
            })
            .await
            .map_err(|e| mcp_internal_error(format!("join error: {e}")))?
            .map_err(mcp_internal_error)?;

            let json = serde_json::to_string_pretty(&summary)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
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

            let json = serde_json::to_string_pretty(&issues)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
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
            let text = serde_json::to_string_pretty(&json)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(text)]))
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

            let json = serde_json::to_string_pretty(&summary)
                .map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
    }
}
