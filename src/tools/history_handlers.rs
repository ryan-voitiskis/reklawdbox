use std::sync::MutexGuard;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use rusqlite::Connection;

use super::mcp_internal_error;
use super::params::{GetPlayStatsParams, GetSessionTracksParams, GetSessionsParams};
use crate::db;

pub(super) fn handle_get_sessions(
    conn: MutexGuard<'_, Connection>,
    params: GetSessionsParams,
) -> Result<CallToolResult, McpError> {
    let after = params
        .after
        .map(|s| db::validate_iso_date(&s, "after"))
        .transpose()
        .map_err(|e| McpError::invalid_params(e, None))?;
    let sessions = db::get_sessions(&conn, params.limit, after.as_deref())
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&sessions).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_session_tracks(
    conn: MutexGuard<'_, Connection>,
    params: GetSessionTracksParams,
) -> Result<CallToolResult, McpError> {
    let tracks = db::get_session_tracks(&conn, &params.session_id)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    if tracks.is_empty() {
        return Ok(CallToolResult::success(vec![Content::text(format!(
            "Session '{}' not found or has no tracks",
            params.session_id
        ))]));
    }
    let json =
        serde_json::to_string_pretty(&tracks).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_play_stats(
    conn: MutexGuard<'_, Connection>,
    params: GetPlayStatsParams,
) -> Result<CallToolResult, McpError> {
    let include_unplayed = params.include_unplayed.unwrap_or(false);
    let limit = params.limit;
    let search = params
        .filters
        .into_search_params(true, None, None)
        .map_err(|e| McpError::invalid_params(e, None))?;
    let stats = db::get_play_stats(&conn, &search, include_unplayed, limit)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&stats).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
