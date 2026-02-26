use rusqlite::Connection;

use rmcp::ErrorData as McpError;

use super::params::{ResolveTracksDataParams, SearchFilterParams};
use crate::db;

pub(super) fn err(msg: String) -> McpError {
    McpError::internal_error(msg, None)
}

pub(super) struct ResolveTracksOpts {
    /// Default max_tracks when track_ids are absent and max_tracks param is None.
    /// When track_ids IS present and this is Some, defaults to ids.len().
    /// None = no auto-default (used by cache_coverage).
    pub default_max: Option<u32>,
    /// Hard cap on effective max. Some(200) for bounded tools, None for unbounded.
    pub cap: Option<u32>,
    /// Post-filter to exclude sampler tracks (used by cache_coverage).
    pub exclude_samplers: bool,
}

/// Resolve tracks using priority: track_ids > playlist_id > search filters.
///
/// Shared by `enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`, and `cache_coverage`.
pub(super) fn resolve_tracks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    filters: SearchFilterParams,
    max_tracks_param: Option<u32>,
    opts: &ResolveTracksOpts,
) -> Result<Vec<crate::types::Track>, McpError> {
    let effective_max: Option<usize> = match opts.default_max {
        Some(default_when_no_ids) => {
            let default = track_ids.map_or(default_when_no_ids, |ids| ids.len() as u32);
            let mut max = max_tracks_param.unwrap_or(default);
            if let Some(cap) = opts.cap {
                max = max.min(cap);
            }
            Some(max as usize)
        }
        None => max_tracks_param.map(|m| {
            if let Some(cap) = opts.cap {
                m.min(cap) as usize
            } else {
                m as usize
            }
        }),
    };

    let bounded = opts.cap.is_some();

    let tracks = if let Some(ids) = track_ids {
        db::get_tracks_by_ids(conn, ids).map_err(|e| err(format!("DB error: {e}")))?
    } else if let Some(pid) = playlist_id {
        let db_limit = if bounded { effective_max.map(|m| m as u32) } else { None };
        if bounded {
            db::get_playlist_tracks(conn, pid, db_limit)
                .map_err(|e| err(format!("DB error: {e}")))?
        } else {
            db::get_playlist_tracks_unbounded(conn, pid, db_limit)
                .map_err(|e| err(format!("DB error: {e}")))?
        }
    } else {
        let limit = effective_max.map(|m| m as u32);
        let search = filters.into_search_params(true, limit, None);
        if bounded {
            db::search_tracks(conn, &search).map_err(|e| err(format!("DB error: {e}")))?
        } else {
            db::search_tracks_unbounded(conn, &search)
                .map_err(|e| err(format!("DB error: {e}")))?
        }
    };

    let mut tracks: Vec<_> = if opts.exclude_samplers {
        tracks
            .into_iter()
            .filter(|t| !t.file_path.starts_with(db::SAMPLER_PATH_PREFIX))
            .collect()
    } else {
        tracks
    };

    if let Some(max) = effective_max {
        tracks.truncate(max);
    }

    Ok(tracks)
}

pub(super) fn describe_resolve_scope(params: &ResolveTracksDataParams) -> String {
    if let Some(track_ids) = &params.track_ids {
        if let Some(max_tracks) = params.max_tracks {
            return format!(
                "track_ids ({}) [max_tracks = {max_tracks}]",
                track_ids.len()
            );
        }
        return format!("track_ids ({})", track_ids.len());
    }

    if let Some(playlist_id) = &params.playlist_id {
        if let Some(max_tracks) = params.max_tracks {
            return format!("playlist_id = \"{playlist_id}\", max_tracks = {max_tracks}");
        }
        return format!("playlist_id = \"{playlist_id}\"");
    }

    let mut filters: Vec<String> = Vec::new();
    if let Some(query) = &params.filters.query {
        filters.push(format!("query ~= \"{query}\""));
    }
    if let Some(artist) = &params.filters.artist {
        filters.push(format!("artist ~= \"{artist}\""));
    }
    if let Some(genre) = &params.filters.genre {
        filters.push(format!("genre ~= \"{genre}\""));
    }
    if let Some(has_genre) = params.filters.has_genre {
        filters.push(format!("has_genre = {has_genre}"));
    }
    if let Some(bpm_min) = params.filters.bpm_min {
        filters.push(format!("bpm_min = {bpm_min}"));
    }
    if let Some(bpm_max) = params.filters.bpm_max {
        filters.push(format!("bpm_max = {bpm_max}"));
    }
    if let Some(key) = &params.filters.key {
        filters.push(format!("key = \"{key}\""));
    }
    if let Some(rating_min) = params.filters.rating_min {
        filters.push(format!("rating_min = {rating_min}"));
    }
    if let Some(label) = &params.filters.label {
        filters.push(format!("label ~= \"{label}\""));
    }
    if let Some(path) = &params.filters.path {
        filters.push(format!("path ~= \"{path}\""));
    }
    if let Some(added_after) = &params.filters.added_after {
        filters.push(format!("added_after = \"{added_after}\""));
    }
    if let Some(added_before) = &params.filters.added_before {
        filters.push(format!("added_before = \"{added_before}\""));
    }
    if let Some(max_tracks) = params.max_tracks {
        filters.push(format!("max_tracks = {max_tracks}"));
    }

    if filters.is_empty() {
        "all tracks".to_string()
    } else {
        filters.join(", ")
    }
}

pub(super) fn to_percent(count: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        ((count as f64 / total as f64) * 1000.0).round() / 10.0
    }
}
