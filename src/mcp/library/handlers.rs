use std::sync::MutexGuard;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use rusqlite::Connection;

use crate::db;
use crate::genre;
use crate::mcp::{
    GetPlaylistTracksParams, GetTrackParams, SearchTracksParams, apply_offset_limit, db_error,
    ok_json, track_has_unknown_genre,
};

pub(in crate::mcp) fn handle_search_tracks(
    conn: MutexGuard<'_, Connection>,
    params: SearchTracksParams,
) -> Result<CallToolResult, McpError> {
    let has_unknown_genre = params.filters.has_unknown_genre == Some(true);
    let limit = params.limit;
    let mut filters = params.filters;
    if has_unknown_genre && filters.has_genre.is_none() {
        filters.has_genre = Some(true);
    }
    // Unknown-genre classification is a Rust post-filter, so its pagination
    // must be applied locally after the full ordered candidate set is filtered.
    let mut search = filters
        .into_search_params(
            !params.include_samples.unwrap_or(false),
            if has_unknown_genre { None } else { limit },
            if has_unknown_genre {
                None
            } else {
                params.offset
            },
        )
        .map_err(|e| McpError::invalid_params(e, None))?;
    search.playlist = params.playlist;
    let tracks = if has_unknown_genre {
        db::search_tracks_unbounded(&conn, &search).map_err(db_error)?
    } else {
        db::search_tracks(&conn, &search).map_err(db_error)?
    };
    let tracks = if has_unknown_genre {
        let tracks = tracks.into_iter().filter(track_has_unknown_genre).collect();
        apply_offset_limit(tracks, params.offset, Some(limit.unwrap_or(50).min(200)))
    } else {
        tracks
    };
    ok_json(&tracks)
}

pub(in crate::mcp) fn handle_get_track(
    conn: MutexGuard<'_, Connection>,
    params: GetTrackParams,
) -> Result<CallToolResult, McpError> {
    let track = db::get_track(&conn, &params.track_id).map_err(db_error)?;
    match track {
        Some(t) => ok_json(&t),
        None => Ok(CallToolResult::success(vec![Content::text(format!(
            "Track '{}' not found",
            params.track_id
        ))])),
    }
}

pub(in crate::mcp) fn handle_get_playlists(
    conn: MutexGuard<'_, Connection>,
) -> Result<CallToolResult, McpError> {
    let playlists = db::get_playlists(&conn).map_err(db_error)?;
    ok_json(&playlists)
}

pub(in crate::mcp) fn handle_get_playlist_tracks(
    conn: MutexGuard<'_, Connection>,
    params: GetPlaylistTracksParams,
) -> Result<CallToolResult, McpError> {
    let tracks =
        db::get_playlist_tracks(&conn, &params.playlist_id, params.limit).map_err(db_error)?;
    ok_json(&tracks)
}

pub(in crate::mcp) fn handle_get_library_summary(
    conn: MutexGuard<'_, Connection>,
) -> Result<CallToolResult, McpError> {
    let stats = db::get_library_stats(&conn).map_err(db_error)?;
    ok_json(&stats)
}

pub(in crate::mcp) fn handle_get_genre_taxonomy() -> Result<CallToolResult, McpError> {
    let genres = genre::GENRES;
    let aliases = genre::genre_alias_map();
    let bpm_ranges: serde_json::Map<String, serde_json::Value> = genres
        .iter()
        .filter_map(|&g| {
            genre::genre_bpm_range(g).map(|r| {
                (
                    g.to_string(),
                    serde_json::json!([r.typical_min, r.typical_max]),
                )
            })
        })
        .collect();
    let result = serde_json::json!({
        "genres": genres,
        "aliases": aliases,
        "bpm_ranges": bpm_ranges,
        "description": "Flat genre taxonomy. Not a closed list — arbitrary genres are accepted. This list provides consistency suggestions. Aliases map non-canonical genre names to their canonical forms. bpm_ranges shows typical BPM ranges for genres where BPM is diagnostic; genres not listed have ranges too wide to be useful."
    });
    ok_json(&result)
}
