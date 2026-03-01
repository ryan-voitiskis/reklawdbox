use std::sync::MutexGuard;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use rusqlite::Connection;

use super::*;
use crate::db;
use crate::genre;

pub(super) fn handle_search_tracks(
    conn: MutexGuard<'_, Connection>,
    params: SearchTracksParams,
) -> Result<CallToolResult, McpError> {
    let mut search = params.filters.into_search_params(
        !params.include_samples.unwrap_or(false),
        params.limit,
        params.offset,
    );
    search.playlist = params.playlist;
    let tracks = db::search_tracks(&conn, &search)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&tracks).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_track(
    conn: MutexGuard<'_, Connection>,
    params: GetTrackParams,
) -> Result<CallToolResult, McpError> {
    let track = db::get_track(&conn, &params.track_id)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    match track {
        Some(t) => {
            let json =
                serde_json::to_string_pretty(&t).map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
        None => Ok(CallToolResult::success(vec![Content::text(format!(
            "Track '{}' not found",
            params.track_id
        ))])),
    }
}

pub(super) fn handle_get_playlists(
    conn: MutexGuard<'_, Connection>,
) -> Result<CallToolResult, McpError> {
    let playlists =
        db::get_playlists(&conn).map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&playlists).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_playlist_tracks(
    conn: MutexGuard<'_, Connection>,
    params: GetPlaylistTracksParams,
) -> Result<CallToolResult, McpError> {
    let tracks = db::get_playlist_tracks(&conn, &params.playlist_id, params.limit)
        .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&tracks).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_library_summary(
    conn: MutexGuard<'_, Connection>,
) -> Result<CallToolResult, McpError> {
    let stats =
        db::get_library_stats(&conn).map_err(|e| mcp_internal_error(format!("DB error: {e}")))?;
    let json =
        serde_json::to_string_pretty(&stats).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_get_genre_taxonomy() -> Result<CallToolResult, McpError> {
    let genres = genre::GENRES;
    let aliases = genre::genre_alias_map();
    let mut result = serde_json::json!({
        "genres": genres,
        "aliases": aliases,
        "description": "Flat genre taxonomy. Not a closed list — arbitrary genres are accepted. This list provides consistency suggestions. Aliases map non-canonical genre names to their canonical forms."
    });
    attach_corpus_provenance(&mut result, consult_genre_workflow_docs());
    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
