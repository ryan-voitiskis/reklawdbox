use std::collections::HashMap;

use super::resolve::*;
use super::{ReklawdboxServer, mcp_internal_error, ok_json};
use crate::application::classification;
pub(super) use crate::application::classification::evidence::parse_response_json;
use crate::classify::{ClassificationAction, ClassificationConfidence, ClassificationResult};
use crate::genre;
use crate::tools::params::{
    AuditGenresParams, CalibrateAudioProfilesParams, CalibrationCoverageParams, ClassifyFormat,
    ClassifyTracksParams,
};
use crate::types::TrackChange;
use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

pub(super) fn handle_classify_tracks(
    server: &ReklawdboxServer,
    params: ClassifyTracksParams,
) -> Result<CallToolResult, McpError> {
    if let Some(ref ids) = params.track_ids
        && ids.is_empty()
    {
        return Err(mcp_internal_error(
            "track_ids was provided but empty — nothing to classify.",
        ));
    }

    // Default to ungenred tracks when using filter-based selection.
    // has_unknown_genre=true auto-sets has_genre=true inside resolve_tracks.
    // When track_ids are provided, respect the explicit selection.
    let mut filters = params.filters;
    if params.track_ids.is_none() && filters.has_unknown_genre != Some(true) {
        filters.has_genre = Some(false);
    }

    let tracks = {
        let conn = server.rekordbox_conn()?;
        resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            filters,
            params.max_tracks,
            params.offset,
            &ResolveTracksOpts {
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let overrides: Vec<(String, String)> = params
        .genre_overrides
        .unwrap_or_default()
        .into_iter()
        .map(|o| (o.from.trim().to_ascii_lowercase(), o.to))
        .collect();

    let invalid_targets: Vec<&str> = overrides
        .iter()
        .filter(|(_, to)| genre::canonical_genre_name(to).is_none())
        .map(|(_, to)| to.as_str())
        .collect();
    if !invalid_targets.is_empty() {
        return Err(mcp_internal_error(format!(
            "Invalid genre override target(s): {}. Must be canonical genre names (see get_genre_taxonomy).",
            invalid_targets.join(", ")
        )));
    }

    let (results, cache_errors) = {
        let store_conn = server.cache_store_conn()?;
        classification::classify_batch(&store_conn, &tracks, &overrides)
            .map_err(super::cache_error)?
    };

    let (high, medium, low, insufficient) = count_by_confidence(&results);
    let (suggest, conflict, confirm, manual) = count_by_action(&results);

    let mut summary = serde_json::json!({
        "total": results.len(),
        "by_confidence": { "high": high, "medium": medium, "low": low, "insufficient": insufficient },
        "by_action": { "suggest": suggest, "conflict": conflict, "confirm": confirm, "manual": manual },
    });
    if cache_errors > 0 {
        summary["cache_read_errors"] = cache_errors.into();
    }

    // --- Auto-staging ---
    let mut staging_info = serde_json::Value::Null;
    if let Some(ref levels) = params.auto_stage {
        let track_changes: Vec<TrackChange> = results
            .iter()
            .filter(|r| {
                r.genre.is_some() && levels.iter().any(|l| l.matches_confidence(&r.confidence))
            })
            .map(|r| TrackChange {
                track_id: r.track_id.clone(),
                genre: r.genre.map(String::from),
                ..Default::default()
            })
            .collect();

        let (staged, total_pending) = server.state.changes.stage(track_changes);
        staging_info = serde_json::json!({
            "staged": staged,
            "total_pending": total_pending,
        });
    }

    // --- Format output ---
    let format = params.format.unwrap_or_default();
    let mut output = match format {
        ClassifyFormat::Full => serde_json::json!({
            "summary": summary,
            "results": results.iter()
                .filter(|r| !matches!(r.action, ClassificationAction::Confirm))
                .collect::<Vec<_>>(),
            "needs_review": results.iter()
                .filter(|r| !matches!(r.action, ClassificationAction::Confirm)
                    && matches!(r.confidence,
                        ClassificationConfidence::Low | ClassificationConfidence::Insufficient
                    ))
                .collect::<Vec<_>>(),
        }),
        ClassifyFormat::Compact => {
            let compact: Vec<_> = results
                .iter()
                .filter(|r| !matches!(r.action, ClassificationAction::Confirm))
                .map(super::super::classify::ClassificationResult::to_compact)
                .collect();
            serde_json::json!({
                "summary": summary,
                "results": compact,
            })
        }
        ClassifyFormat::Summary => {
            let by_genre = build_genre_distribution(&results);
            serde_json::json!({
                "summary": summary,
                "by_genre": by_genre,
            })
        }
        ClassifyFormat::Dispatch => {
            let (artists, dispatch_stats) = build_dispatch_groups(&results);
            serde_json::json!({
                "summary": summary,
                "artists": artists,
                "dispatch_stats": dispatch_stats,
            })
        }
    };

    if !staging_info.is_null() {
        output["staging"] = staging_info;
    }

    ok_json(&output)
}

pub(super) fn handle_audit_genres(
    server: &ReklawdboxServer,
    params: AuditGenresParams,
) -> Result<CallToolResult, McpError> {
    if let Some(ref ids) = params.track_ids
        && ids.is_empty()
    {
        return Err(mcp_internal_error(
            "track_ids was provided but empty — nothing to audit.",
        ));
    }

    // Force has_genre=true when using filter-based selection. When track_ids
    // are provided, respect the explicit selection.
    let mut filters = params.filters;
    if params.track_ids.is_none() {
        filters.has_genre = Some(true);
    }

    let tracks = {
        let conn = server.rekordbox_conn()?;
        resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            filters,
            params.max_tracks,
            params.offset,
            &ResolveTracksOpts {
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let include_confirmed = params.include_confirmed.unwrap_or(false);
    let (results, cache_errors) = {
        let store_conn = server.cache_store_conn()?;
        classification::classify_batch(&store_conn, &tracks, &[]).map_err(super::cache_error)?
    };

    let visible: Vec<&ClassificationResult> = results
        .iter()
        .filter(|r| include_confirmed || !matches!(r.action, ClassificationAction::Confirm))
        .collect();

    let confirmed_count = results
        .iter()
        .filter(|r| matches!(r.action, ClassificationAction::Confirm))
        .count();
    let conflict_count = results
        .iter()
        .filter(|r| matches!(r.action, ClassificationAction::Conflict))
        .count();

    let (high, medium, low, insufficient) = count_by_confidence(&results);

    let mut summary = serde_json::json!({
        "total_audited": results.len(),
        "confirmed": confirmed_count,
        "conflicts": conflict_count,
        "manual_review": results.iter().filter(|r| matches!(r.action, ClassificationAction::Manual)).count(),
        "by_confidence": { "high": high, "medium": medium, "low": low, "insufficient": insufficient },
    });
    if cache_errors > 0 {
        summary["cache_read_errors"] = cache_errors.into();
    }

    let output = serde_json::json!({
        "summary": summary,
        "results": visible,
    });

    ok_json(&output)
}

fn count_by_confidence(results: &[ClassificationResult]) -> (u32, u32, u32, u32) {
    let (mut high, mut medium, mut low, mut insufficient) = (0u32, 0u32, 0u32, 0u32);
    for r in results {
        match r.confidence {
            ClassificationConfidence::High => high += 1,
            ClassificationConfidence::Medium => medium += 1,
            ClassificationConfidence::Low => low += 1,
            ClassificationConfidence::Insufficient => insufficient += 1,
        }
    }
    (high, medium, low, insufficient)
}

fn count_by_action(results: &[ClassificationResult]) -> (u32, u32, u32, u32) {
    let (mut suggest, mut conflict, mut confirm, mut manual) = (0u32, 0u32, 0u32, 0u32);
    for r in results {
        match r.action {
            ClassificationAction::Suggest => suggest += 1,
            ClassificationAction::Conflict => conflict += 1,
            ClassificationAction::Confirm => confirm += 1,
            ClassificationAction::Manual => manual += 1,
        }
    }
    (suggest, conflict, confirm, manual)
}

/// Build a genre-grouped distribution for the summary format.
/// Groups by recommended genre, then by confidence level, with artist counts.
pub(super) fn build_genre_distribution(results: &[ClassificationResult]) -> serde_json::Value {
    // genre → { confidence → [artists] }
    let mut genre_map: HashMap<&str, HashMap<&str, Vec<&str>>> = HashMap::new();

    for r in results {
        if matches!(r.action, ClassificationAction::Confirm) {
            continue;
        }
        let Some(genre) = r.genre else {
            continue;
        };
        let conf = match r.confidence {
            ClassificationConfidence::High => "high",
            ClassificationConfidence::Medium => "medium",
            ClassificationConfidence::Low => "low",
            ClassificationConfidence::Insufficient => "insufficient",
        };
        genre_map
            .entry(genre)
            .or_default()
            .entry(conf)
            .or_default()
            .push(&r.artist);
    }

    // Build sorted output: genre → { count, by_confidence: { high: N, ... }, top_artists: [...] }
    let mut genres: Vec<_> = genre_map
        .into_iter()
        .map(|(genre, conf_map)| {
            let total: usize = conf_map.values().map(std::vec::Vec::len).sum();
            (genre, conf_map, total)
        })
        .collect();
    genres.sort_by_key(|b| std::cmp::Reverse(b.2));

    genres
        .into_iter()
        .map(|(genre, conf_map, total)| {
            // Count artists across all confidence levels
            let mut artist_counts: HashMap<&str, usize> = HashMap::new();
            for artists in conf_map.values() {
                for &a in artists {
                    *artist_counts.entry(a).or_default() += 1;
                }
            }
            let mut top: Vec<_> = artist_counts.into_iter().collect();
            top.sort_by_key(|b| std::cmp::Reverse(b.1));
            let top_artists: Vec<String> = top
                .iter()
                .take(5)
                .map(|(a, c)| {
                    if *c > 1 {
                        format!("{a} ({c})")
                    } else {
                        a.to_string()
                    }
                })
                .collect();

            let mut by_conf = serde_json::Map::new();
            for level in &["high", "medium", "low", "insufficient"] {
                if let Some(artists) = conf_map.get(level) {
                    by_conf.insert(level.to_string(), serde_json::json!(artists.len()));
                }
            }

            serde_json::json!({
                "genre": genre,
                "count": total,
                "by_confidence": by_conf,
                "top_artists": top_artists,
            })
        })
        .collect()
}

/// Build artist-grouped roster of low/insufficient confidence tracks for subagent dispatch.
fn build_dispatch_groups(
    results: &[ClassificationResult],
) -> (serde_json::Value, serde_json::Value) {
    let mut artist_map: HashMap<&str, Vec<serde_json::Value>> = HashMap::new();

    let mut tracks_without_suggestion: usize = 0;

    for r in results {
        if matches!(r.action, ClassificationAction::Confirm) {
            continue;
        }
        let conf = match r.confidence {
            ClassificationConfidence::Low => "low",
            ClassificationConfidence::Insufficient => "insufficient",
            ClassificationConfidence::High | ClassificationConfidence::Medium => continue,
        };
        if r.genre.is_none() {
            tracks_without_suggestion += 1;
        }
        artist_map
            .entry(&r.artist)
            .or_default()
            .push(serde_json::json!({
                "track_id": r.track_id,
                "title": r.title,
                "genre": r.genre,
                "confidence": conf,
                "evidence": r.evidence,
                "candidates": r.candidates,
                "flags": r.flags,
            }));
    }

    let total_tracks: usize = artist_map.values().map(std::vec::Vec::len).sum();
    let total_artists = artist_map.len();

    let mut artists: Vec<_> = artist_map.into_iter().collect();
    artists.sort_by_key(|b| std::cmp::Reverse(b.1.len()));

    let artists: Vec<serde_json::Value> = artists
        .into_iter()
        .map(|(artist, tracks)| {
            serde_json::json!({
                "artist": artist,
                "track_count": tracks.len(),
                "tracks": tracks,
            })
        })
        .collect();

    let mut stats = serde_json::json!({
        "total_tracks": total_tracks,
        "total_artists": total_artists,
    });
    if tracks_without_suggestion > 0 {
        stats["tracks_without_suggestion"] = serde_json::json!(tracks_without_suggestion);
    }

    (serde_json::Value::Array(artists), stats)
}

pub(super) fn handle_calibrate_audio_profiles(
    server: &ReklawdboxServer,
    params: CalibrateAudioProfilesParams,
) -> Result<CallToolResult, McpError> {
    let playlist_name = params.playlist.as_deref().unwrap_or("genre_verified");

    // 1. Get playlist tracks
    let (tracks, _playlist_name) = {
        let conn = server.rekordbox_conn()?;
        let playlists = crate::db::get_playlists(&conn).map_err(|e| {
            McpError::internal_error(format!("Failed to list playlists: {e}"), None)
        })?;
        let playlist = playlists
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(playlist_name))
            .ok_or_else(|| {
                McpError::internal_error(
                    format!("Playlist '{playlist_name}' not found. Create it in Rekordbox and add verified tracks."),
                    None,
                )
            })?;
        let tracks =
            crate::db::get_playlist_tracks_unbounded(&conn, &playlist.id, None).map_err(|e| {
                McpError::internal_error(format!("Failed to get playlist tracks: {e}"), None)
            })?;
        (tracks, playlist.name.clone())
    };

    if tracks.is_empty() {
        return Err(McpError::internal_error(
            format!("Playlist '{playlist_name}' is empty — add verified tracks first."),
            None,
        ));
    }

    let result = {
        let store_conn = server.cache_store_conn()?;
        match classification::calibrate_audio_profiles(&store_conn, &tracks, playlist_name) {
            Ok(result) => result,
            Err(classification::CalibrationError::NoSamples) => {
                return Err(McpError::internal_error(
                    "No tracks with both genre tags and audio features found.",
                    None,
                ));
            }
            Err(classification::CalibrationError::Store(error)) => {
                return Err(super::cache_error(error));
            }
        }
    };

    ok_json(&result)
}

pub(super) fn handle_calibration_coverage(
    server: &ReklawdboxServer,
    params: CalibrationCoverageParams,
) -> Result<CallToolResult, McpError> {
    let playlist_name = params.playlist.as_deref().unwrap_or("genre_verified");

    let (tracks, resolved_playlist_name) = {
        let conn = server.rekordbox_conn()?;
        let playlists = crate::db::get_playlists(&conn).map_err(|e| {
            McpError::internal_error(format!("Failed to list playlists: {e}"), None)
        })?;
        let playlist = playlists
            .iter()
            .find(|p| p.name.eq_ignore_ascii_case(playlist_name))
            .ok_or_else(|| {
                McpError::internal_error(
                    format!(
                        "Playlist '{playlist_name}' not found. Create it in Rekordbox and add verified tracks."
                    ),
                    None,
                )
            })?;
        let tracks =
            crate::db::get_playlist_tracks_unbounded(&conn, &playlist.id, None).map_err(|e| {
                McpError::internal_error(format!("Failed to get playlist tracks: {e}"), None)
            })?;
        (tracks, playlist.name.clone())
    };

    let result = {
        let store_conn = server.cache_store_conn()?;
        classification::calibration_coverage(&store_conn, &tracks, &resolved_playlist_name)
            .map_err(super::cache_error)?
    };

    ok_json(&result)
}
