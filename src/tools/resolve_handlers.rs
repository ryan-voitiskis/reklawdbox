use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::audio;
use crate::db;
use crate::genre;
use crate::store;

pub(super) fn handle_resolve_track_data(
    server: &ReklawdboxServer,
    params: ResolveTrackDataParams,
) -> Result<CallToolResult, McpError> {
    let track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{}' not found", params.track_id), None)
            })?
    };

    let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
    let norm_title = crate::normalize::normalize_for_matching(&track.title);

    let essentia_installed = server.essentia_python_path().is_some();

    let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
        let store = server.cache_store_conn()?;
        let discogs_cache = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
            .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
        let beatport_cache = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
            .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
        let audio_cache_key =
            resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
        let stratum_cache =
            store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
        let essentia_cache =
            store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
        (discogs_cache, beatport_cache, stratum_cache, essentia_cache)
    };

    let staged = server.state.changes.get(&track.id);

    let result = resolve_single_track(
        &track,
        discogs_cache.as_ref(),
        beatport_cache.as_ref(),
        stratum_cache.as_ref(),
        essentia_cache.as_ref(),
        essentia_installed,
        staged.as_ref(),
    );

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_resolve_tracks_data(
    server: &ReklawdboxServer,
    params: ResolveTracksDataParams,
) -> Result<CallToolResult, McpError> {
    let tracks = {
        let conn = server.rekordbox_conn()?;
        resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            params.filters,
            params.max_tracks,
            &ResolveTracksOpts {
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let essentia_installed = server.essentia_python_path().is_some();
    let mut results = Vec::with_capacity(tracks.len());
    for track in &tracks {
        let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
        let norm_title = crate::normalize::normalize_for_matching(&track.title);

        let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
            let store = server.cache_store_conn()?;
            let discogs_cache = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
            let beatport_cache =
                store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                    .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
            let audio_cache_key =
                resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
            let stratum_cache =
                store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                    .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
            let essentia_cache =
                store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                    .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?;
            (discogs_cache, beatport_cache, stratum_cache, essentia_cache)
        };

        let staged = server.state.changes.get(&track.id);

        results.push(resolve_single_track(
            track,
            discogs_cache.as_ref(),
            beatport_cache.as_ref(),
            stratum_cache.as_ref(),
            essentia_cache.as_ref(),
            essentia_installed,
            staged.as_ref(),
        ));
    }

    let json =
        serde_json::to_string_pretty(&results).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_cache_coverage(
    server: &ReklawdboxServer,
    params: ResolveTracksDataParams,
) -> Result<CallToolResult, McpError> {
    let filter_description = describe_resolve_scope(&params);

    let (total_tracks, tracks) = {
        let conn = server.rekordbox_conn()?;
        let sample_prefix = format!("%{}%", db::escape_like(db::SAMPLER_PATH_FRAGMENT));
        let total_tracks: usize = conn
            .query_row(
                "SELECT COUNT(*) FROM djmdContent
                 WHERE rb_local_deleted = 0
                   AND FolderPath NOT LIKE ?1 ESCAPE '\\'",
                rusqlite::params![sample_prefix],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .max(0) as usize;

        let tracks = resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            params.filters,
            params.max_tracks,
            &ResolveTracksOpts {
                default_max_tracks: None,
                max_tracks_cap: None,
                exclude_samplers: true,
            },
        )?;

        (total_tracks, tracks)
    };

    let matched_tracks = tracks.len();
    let essentia_installed = server.essentia_python_path().is_some();

    let mut stratum_cached = 0usize;
    let mut essentia_cached = 0usize;
    let mut discogs_cached = 0usize;
    let mut beatport_cached = 0usize;
    let mut no_audio_analysis = 0usize;
    let mut no_enrichment = 0usize;
    let mut no_data_at_all = 0usize;

    {
        let store = server.cache_store_conn()?;
        for track in &tracks {
            let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
            let norm_title = crate::normalize::normalize_for_matching(&track.title);
            let audio_cache_key =
                resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());

            let has_discogs = store::get_enrichment(&store, "discogs", &norm_artist, &norm_title)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
                .is_some();
            let has_beatport = store::get_enrichment(&store, "beatport", &norm_artist, &norm_title)
                .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
                .is_some();
            let has_stratum =
                store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_STRATUM)
                    .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
                    .is_some();
            let has_essentia =
                store::get_audio_analysis(&store, &audio_cache_key, audio::ANALYZER_ESSENTIA)
                    .map_err(|e| mcp_internal_error(format!("Cache read error: {e}")))?
                    .is_some();

            if has_stratum {
                stratum_cached += 1;
            }
            if has_essentia {
                essentia_cached += 1;
            }
            if has_discogs {
                discogs_cached += 1;
            }
            if has_beatport {
                beatport_cached += 1;
            }
            if !has_stratum {
                no_audio_analysis += 1;
            }
            if !has_discogs && !has_beatport {
                no_enrichment += 1;
            }
            if !has_stratum && !has_essentia && !has_discogs && !has_beatport {
                no_data_at_all += 1;
            }
        }
    }

    let result = serde_json::json!({
        "scope": {
            "total_tracks": total_tracks,
            "filter_description": filter_description,
            "matched_tracks": matched_tracks,
        },
        "coverage": {
            "stratum_dsp": {
                "cached": stratum_cached,
                "percent": to_percent(stratum_cached, matched_tracks),
            },
            "essentia": {
                "cached": essentia_cached,
                "percent": to_percent(essentia_cached, matched_tracks),
                "installed": essentia_installed,
            },
            "discogs": {
                "cached": discogs_cached,
                "percent": to_percent(discogs_cached, matched_tracks),
            },
            "beatport": {
                "cached": beatport_cached,
                "percent": to_percent(beatport_cached, matched_tracks),
            },
        },
        "gaps": {
            "no_audio_analysis": no_audio_analysis,
            "no_enrichment": no_enrichment,
            "no_data_at_all": no_data_at_all,
        },
    });

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

/// Build the resolved JSON payload for a single track.
/// This is a pure function that takes pre-fetched data and produces the output.
pub(crate) fn resolve_single_track(
    track: &crate::types::Track,
    discogs_cache: Option<&store::EnrichmentCacheEntry>,
    beatport_cache: Option<&store::EnrichmentCacheEntry>,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
    essentia_installed: bool,
    staged: Option<&crate::types::TrackChange>,
) -> serde_json::Value {
    let rekordbox = serde_json::json!({
        "title": track.title,
        "artist": track.artist,
        "remixer": track.remixer,
        "album": track.album,
        "genre": track.genre,
        "bpm": track.bpm,
        "key": track.key,
        "duration_s": track.length,
        "year": track.year,
        "rating": track.rating,
        "comments": track.comments,
        "label": track.label,
        "color": track.color,
        "play_count": track.play_count,
        "date_added": track.date_added,
    });

    let (stratum_json, stratum_parse_error) = match stratum_cache {
        Some(sc) => match serde_json::from_str::<serde_json::Value>(&sc.features_json) {
            Ok(val) => (Some(val), None),
            Err(e) => (None, Some(format!("stratum-dsp cache JSON corrupt: {e}"))),
        },
        None => (None, None),
    };
    let (essentia_data, essentia_parse_error) = match essentia_cache {
        Some(ec) => match serde_json::from_str::<audio::EssentiaOutput>(&ec.features_json) {
            Ok(val) => (Some(val), None),
            Err(e) => (None, Some(format!("essentia cache JSON corrupt: {e}"))),
        },
        None => (None, None),
    };
    let essentia_json = essentia_data
        .as_ref()
        .and_then(|e| serde_json::to_value(e).ok());

    let (bpm_agreement, key_agreement) = if let Some(ref sj) = stratum_json {
        let stratum_bpm = sj.get("bpm").and_then(|v| v.as_f64());
        let stratum_key = sj.get("key").and_then(|v| v.as_str());

        let bpm_agree = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);
        let key_agree = stratum_key.map(|sk| sk.eq_ignore_ascii_case(&track.key));

        (bpm_agree, key_agree)
    } else {
        (None, None)
    };

    let has_analysis = stratum_json.is_some()
        || essentia_json.is_some()
        || stratum_parse_error.is_some()
        || essentia_parse_error.is_some();
    let audio_analysis = if has_analysis {
        let mut obj = serde_json::json!({
            "stratum_dsp": stratum_json,
            "essentia": essentia_json,
            "bpm_agreement": bpm_agreement,
            "key_agreement": key_agreement,
        });
        if let Some(err) = &stratum_parse_error {
            obj["stratum_dsp_parse_error"] = serde_json::json!(err);
        }
        if let Some(err) = &essentia_parse_error {
            obj["essentia_parse_error"] = serde_json::json!(err);
        }
        obj
    } else {
        serde_json::Value::Null
    };

    let discogs_val = parse_enrichment_cache(discogs_cache);
    let beatport_val = parse_enrichment_cache(beatport_cache);

    let staged_val = staged.map(|s| {
        serde_json::json!({
            "genre": s.genre,
            "comments": s.comments,
            "rating": s.rating,
            "color": s.color,
        })
    });

    let data_completeness = serde_json::json!({
        "rekordbox": true,
        "stratum_dsp": stratum_cache.is_some(),
        "essentia": essentia_cache.is_some(),
        "essentia_installed": essentia_installed,
        "discogs": discogs_cache.is_some(),
        "beatport": beatport_cache.is_some(),
    });

    let current_genre_canonical = if track.genre.is_empty() {
        serde_json::Value::Null
    } else if let Some(canonical) = genre::canonical_genre_name(&track.genre) {
        serde_json::json!(canonical)
    } else if let Some(canonical) = genre::canonical_genre_from_alias(&track.genre) {
        serde_json::json!(canonical)
    } else {
        serde_json::Value::Null
    };

    let discogs_style_mappings: Vec<serde_json::Value> = discogs_val
        .as_ref()
        .and_then(|v| v.get("styles"))
        .and_then(|v| v.as_array())
        .map(|styles| {
            styles
                .iter()
                .filter_map(|s| s.as_str())
                .map(|style| {
                    let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
                    serde_json::json!({
                        "style": style,
                        "maps_to": maps_to,
                        "mapping_type": mapping_type,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let beatport_genre_mapping = beatport_val
        .as_ref()
        .and_then(|v| v.get("genre"))
        .and_then(|v| v.as_str())
        .filter(|g| !g.is_empty())
        .map(|bp_genre| {
            let (maps_to, mapping_type) = map_genre_through_taxonomy(bp_genre);
            serde_json::json!({
                "genre": bp_genre,
                "maps_to": maps_to,
                "mapping_type": mapping_type,
            })
        });

    let genre_taxonomy = serde_json::json!({
        "current_genre_canonical": current_genre_canonical,
        "discogs_style_mappings": discogs_style_mappings,
        "beatport_genre_mapping": beatport_genre_mapping,
    });

    serde_json::json!({
        "track_id": track.id,
        "rekordbox": rekordbox,
        "audio_analysis": audio_analysis,
        "discogs": discogs_val,
        "beatport": beatport_val,
        "staged_changes": staged_val,
        "data_completeness": data_completeness,
        "genre_taxonomy": genre_taxonomy,
    })
}

/// Parse a cached enrichment entry's response_json into a serde_json::Value.
/// Returns None if cache entry is None or has no response_json.
/// Injects match_quality and cached_at metadata into the returned object.
fn parse_enrichment_cache(
    cache: Option<&store::EnrichmentCacheEntry>,
) -> Option<serde_json::Value> {
    cache.and_then(|c| {
        let mut val = c
            .response_json
            .as_ref()
            .and_then(|json_str| serde_json::from_str::<serde_json::Value>(json_str).ok())?;
        if let serde_json::Value::Object(ref mut map) = val {
            map.insert("match_quality".into(), serde_json::json!(c.match_quality));
            map.insert("cached_at".into(), serde_json::json!(c.created_at));
        }
        Some(val)
    })
}
