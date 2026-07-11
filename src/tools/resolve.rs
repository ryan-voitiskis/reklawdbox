use rusqlite::Connection;

use rmcp::ErrorData as McpError;

use super::*;
use crate::db;
use crate::genre;
use crate::types::Track;

pub(super) struct ResolveTracksOpts {
    /// Default max_tracks when track_ids are absent and max_tracks param is None.
    /// When track_ids IS present and this is Some, defaults to ids.len().
    /// None = no auto-default (used by cache_coverage).
    pub default_max_tracks: Option<u32>,
    /// Hard cap on effective max. Some(200) for bounded tools, None for unbounded.
    pub max_tracks_cap: Option<u32>,
    /// Post-filter to exclude sampler tracks (used by cache_coverage).
    pub exclude_samplers: bool,
}

pub(super) fn track_has_unknown_genre(track: &Track) -> bool {
    !track.genre.is_empty()
        && !genre::is_known_genre(&track.genre)
        && genre::canonical_genre_from_alias(&track.genre).is_none()
}

pub(super) fn apply_offset_limit(
    tracks: Vec<Track>,
    offset: Option<u32>,
    limit: Option<u32>,
) -> Vec<Track> {
    // Reklawdbox supports 32- and 64-bit targets, where every u32 fits usize.
    let offset = offset.unwrap_or(0) as usize;
    let limit = limit.map_or(usize::MAX, |value| value as usize);
    tracks.into_iter().skip(offset).take(limit).collect()
}

/// Resolve tracks using priority: track_ids > playlist_id > search filters.
///
/// Shared by `enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`, and `cache_coverage`.
pub(super) fn resolve_tracks(
    conn: &Connection,
    track_ids: Option<&[String]>,
    playlist_id: Option<&str>,
    mut filters: SearchFilterParams,
    max_tracks_param: Option<u32>,
    offset: Option<u32>,
    opts: &ResolveTracksOpts,
) -> Result<Vec<crate::types::Track>, McpError> {
    let effective_max: Option<u32> = match opts.default_max_tracks {
        Some(default_when_no_ids) => {
            let default = track_ids.map_or(default_when_no_ids, |ids| {
                u32::try_from(ids.len()).unwrap_or(u32::MAX)
            });
            let mut max = max_tracks_param.unwrap_or(default);
            if let Some(max_tracks_cap) = opts.max_tracks_cap {
                max = max.min(max_tracks_cap);
            }
            Some(max)
        }
        None => max_tracks_param.map(|m| {
            if let Some(max_tracks_cap) = opts.max_tracks_cap {
                m.min(max_tracks_cap)
            } else {
                m
            }
        }),
    };

    if filters.has_unknown_genre == Some(true) && filters.has_genre.is_none() {
        filters.has_genre = Some(true);
    }

    let has_unknown_genre = filters.has_unknown_genre;
    let bounded = opts.max_tracks_cap.is_some();

    // Selector priority is IDs > playlist > search. Pagination is applied in
    // SQL only when every logical filter is also in SQL; otherwise the full
    // ordered candidate set is post-filtered and paginated locally.
    let (tracks, pagination_applied_in_db) = if let Some(ids) = track_ids {
        (db::get_tracks_by_ids(conn, ids).map_err(db_error)?, false)
    } else if let Some(pid) = playlist_id {
        let playlist_requires_post_filter =
            has_unknown_genre == Some(true) || opts.exclude_samplers;
        if playlist_requires_post_filter {
            (
                db::get_playlist_tracks_unbounded(conn, pid, None).map_err(db_error)?,
                false,
            )
        } else if bounded {
            (
                db::get_playlist_tracks_page(conn, pid, effective_max, offset).map_err(db_error)?,
                true,
            )
        } else {
            (
                db::get_playlist_tracks_unbounded_page(conn, pid, effective_max, offset)
                    .map_err(db_error)?,
                true,
            )
        }
    } else {
        if has_unknown_genre == Some(true) {
            let search = filters
                .into_search_params(true, None, None)
                .map_err(|e| McpError::invalid_params(e, None))?;
            (
                db::search_tracks_unbounded(conn, &search).map_err(db_error)?,
                false,
            )
        } else {
            let search = filters
                .into_search_params(true, effective_max, offset)
                .map_err(|e| McpError::invalid_params(e, None))?;
            if bounded {
                (db::search_tracks(conn, &search).map_err(db_error)?, true)
            } else {
                (
                    db::search_tracks_unbounded(conn, &search).map_err(db_error)?,
                    true,
                )
            }
        }
    };

    let mut tracks: Vec<_> = if opts.exclude_samplers {
        tracks
            .into_iter()
            .filter(|t| !t.file_path.contains(db::SAMPLER_PATH_FRAGMENT))
            .collect()
    } else {
        tracks
    };

    if has_unknown_genre == Some(true) {
        tracks.retain(track_has_unknown_genre);
    }

    if pagination_applied_in_db {
        Ok(tracks)
    } else {
        Ok(apply_offset_limit(tracks, offset, effective_max))
    }
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
    if let Some(has_label) = params.filters.has_label {
        filters.push(format!("has_label = {has_label}"));
    }
    if let Some(has_unknown_genre) = params.filters.has_unknown_genre {
        filters.push(format!("has_unknown_genre = {has_unknown_genre}"));
    }
    if let Some(year_zero) = params.filters.year_zero {
        filters.push(format!("year_zero = {year_zero}"));
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
    if let Some(prefix) = &params.filters.path_prefix {
        filters.push(format!("path_prefix = \"{prefix}\""));
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
