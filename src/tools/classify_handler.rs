use std::collections::{BTreeMap, HashMap};

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;
use tracing::warn;

use super::resolve::*;
use super::scoring::map_genre_through_taxonomy;
use super::{ReklawdboxServer, mcp_internal_error, ok_json};
use crate::audio;
use crate::audio_profile;
use crate::classify::{
    AudioFeatures, ClassificationAction, ClassificationConfidence, ClassificationResult,
    MappedGenre, TrackEvidence, classify_track_with_profiles,
};
use crate::genre;
use crate::normalize;
use crate::store;
use crate::tools::params::{
    AuditGenresParams, CalibrateAudioProfilesParams, CalibrationCoverageParams, ClassifyFormat,
    ClassifyTracksParams,
};
use crate::types::TrackChange;

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

    let (results, cache_errors) = classify_batch(server, &tracks, &overrides)?;

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
    let (results, cache_errors) = classify_batch(server, &tracks, &[])?;

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
    genres.sort_by(|a, b| b.2.cmp(&a.2));

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
            top.sort_by(|a, b| b.1.cmp(&a.1));
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
    artists.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

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

fn classify_batch(
    server: &ReklawdboxServer,
    tracks: &[crate::types::Track],
    overrides: &[(String, String)],
) -> Result<(Vec<ClassificationResult>, u32), McpError> {
    // Pre-compute normalized keys and resolved audio paths.
    let norm_keys: Vec<(String, String, Option<String>, String)> = tracks
        .iter()
        .map(|t| {
            let a = normalize::normalize_for_matching(&t.artist);
            let ti = normalize::normalize_for_matching(&t.title);
            let al = normalize::normalize_for_matching(&t.album);
            let audio_key = super::analysis::resolved_audio_cache_key(&t.file_path);
            (a, ti, (!al.is_empty()).then_some(al), audio_key)
        })
        .collect();

    // Build batch keys.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 2);
    let mut audio_paths: Vec<&str> = Vec::with_capacity(tracks.len());
    for (a, t, al, audio_key) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("beatport", a, t, ""));
        audio_paths.push(audio_key);
    }

    // Batch load — 3 queries total instead of 4N.
    let (enrich_map, stratum_map, essentia_map, profile_registry) = {
        let store_conn = server.cache_store_conn()?;
        let enrich_map =
            store::batch_get_enrichment(&store_conn, &enrich_keys).map_err(super::cache_error)?;
        let stratum_map = store::batch_get_audio_analysis(
            &store_conn,
            &audio_paths,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(super::cache_error)?;
        let essentia_map = store::batch_get_audio_analysis(
            &store_conn,
            &audio_paths,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(super::cache_error)?;
        let registry = audio_profile::load_from_db(&store_conn).map_err(super::cache_error)?;
        (enrich_map, stratum_map, essentia_map, registry)
    };

    let mut results = Vec::with_capacity(tracks.len());

    for (track, (norm_artist, norm_title, norm_album, audio_key)) in tracks.iter().zip(&norm_keys) {
        let album = norm_album.as_deref().unwrap_or("");
        let discogs_key = (
            "discogs".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            album.to_string(),
        );
        let beatport_key = (
            "beatport".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            String::new(),
        );

        let discogs_cache = enrich_map.get(&discogs_key);
        let beatport_cache = enrich_map.get(&beatport_key);
        let stratum_cache = stratum_map.get(audio_key);
        let essentia_cache = essentia_map.get(audio_key);

        let evidence = build_track_evidence(
            track,
            discogs_cache,
            beatport_cache,
            stratum_cache,
            essentia_cache,
            overrides,
        );
        results.push(classify_track_with_profiles(
            &evidence,
            profile_registry.as_ref(),
        ));
    }

    Ok((results, 0))
}

fn build_track_evidence(
    track: &crate::types::Track,
    discogs_cache: Option<&store::EnrichmentCacheEntry>,
    beatport_cache: Option<&store::EnrichmentCacheEntry>,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
    overrides: &[(String, String)],
) -> TrackEvidence {
    let discogs_val = parse_response_json(discogs_cache);
    let discogs_mapped = extract_discogs_genres(discogs_val.as_ref(), overrides);

    let beatport_val = parse_response_json(beatport_cache);
    let (beatport_genre, beatport_raw) = extract_beatport_genre(beatport_val.as_ref(), overrides);

    let effective_label = if !track.label.is_empty() {
        Some(track.label.clone())
    } else {
        discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
    };
    let label_genre_val = effective_label.as_deref().and_then(genre::label_genre);

    let audio = extract_audio_features(track, stratum_cache, essentia_cache);
    let has_audio = audio.is_some();

    TrackEvidence {
        track_id: track.id.clone(),
        artist: track.artist.clone(),
        title: track.title.clone(),
        current_genre: track.genre.clone(),
        bpm: track.bpm,
        discogs_mapped,
        beatport_genre,
        beatport_raw,
        label: effective_label,
        label_genre: label_genre_val,
        audio,
        has_discogs: discogs_cache.is_some(),
        has_beatport: beatport_cache.is_some(),
        has_audio,
    }
}

pub(super) fn parse_response_json(
    cache: Option<&store::EnrichmentCacheEntry>,
) -> Option<serde_json::Value> {
    cache.and_then(|c| {
        c.response_json.as_ref().and_then(|json_str| {
            match serde_json::from_str::<serde_json::Value>(json_str) {
                Ok(val) => Some(val),
                Err(e) => {
                    warn!(
                        provider = c.provider.as_str(),
                        artist = c.query_artist.as_str(),
                        title = c.query_title.as_str(),
                        "Cached response_json failed to parse: {e}"
                    );
                    None
                }
            }
        })
    })
}

fn apply_override(raw: &str, overrides: &[(String, String)]) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    overrides
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| to.clone())
}

fn extract_discogs_genres(
    discogs_val: Option<&serde_json::Value>,
    overrides: &[(String, String)],
) -> Vec<MappedGenre> {
    let Some(styles) = discogs_val
        .and_then(|v| v.get("styles"))
        .and_then(|v| v.as_array())
    else {
        return vec![];
    };

    let mut genre_counts: HashMap<&'static str, usize> = HashMap::new();

    for style in styles.iter().filter_map(|s| s.as_str()) {
        if let Some(override_genre) = apply_override(style, overrides) {
            if let Some(canonical) = genre::canonical_genre_name(&override_genre) {
                *genre_counts.entry(canonical).or_insert(0) += 1;
                continue;
            } else {
                warn!(
                    from = style,
                    to = override_genre.as_str(),
                    "Genre override target is not a canonical genre — override ignored"
                );
            }
        }
        let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
        if mapping_type != "unknown"
            && let Some(genre_name) = maps_to
            && let Some(canonical) = genre::canonical_genre_name(&genre_name)
        {
            *genre_counts.entry(canonical).or_insert(0) += 1;
        }
    }

    genre_counts
        .into_iter()
        .map(|(genre, style_count)| MappedGenre { genre, style_count })
        .collect()
}

fn extract_beatport_genre(
    beatport_val: Option<&serde_json::Value>,
    overrides: &[(String, String)],
) -> (Option<&'static str>, Option<String>) {
    let raw_str = beatport_val
        .and_then(|v| v.get("genre"))
        .and_then(|v| v.as_str())
        .filter(|g| !g.is_empty());

    let Some(raw) = raw_str else {
        return (None, None);
    };

    if let Some(override_genre) = apply_override(raw, overrides) {
        if let Some(canonical) = genre::canonical_genre_name(&override_genre) {
            return (Some(canonical), Some(raw.to_string()));
        } else {
            warn!(
                from = raw,
                to = override_genre.as_str(),
                "Genre override target is not a canonical genre — override ignored"
            );
        }
    }

    let (maps_to, mapping_type) = map_genre_through_taxonomy(raw);
    let canonical = if mapping_type != "unknown" && maps_to.is_some() {
        maps_to.and_then(|g| genre::canonical_genre_name(&g))
    } else {
        None
    };

    (canonical, Some(raw.to_string()))
}

fn extract_audio_features(
    track: &crate::types::Track,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
) -> Option<AudioFeatures> {
    let stratum_json = stratum_cache.and_then(|sc| {
        match serde_json::from_str::<serde_json::Value>(&sc.features_json) {
            Ok(val) => Some(val),
            Err(e) => {
                warn!(
                    file = track.file_path.as_str(),
                    "Stratum features_json failed to parse: {e}"
                );
                None
            }
        }
    });
    let essentia_data = essentia_cache.and_then(|ec| {
        match serde_json::from_str::<audio::EssentiaOutput>(&ec.features_json) {
            Ok(val) => Some(val),
            Err(e) => {
                warn!(
                    file = track.file_path.as_str(),
                    "Essentia features_json failed to parse: {e}"
                );
                None
            }
        }
    });

    if stratum_json.is_none() && essentia_data.is_none() {
        return None;
    }

    let stratum_bpm = stratum_json
        .as_ref()
        .and_then(|sj| sj.get("bpm"))
        .and_then(serde_json::Value::as_f64);
    let bpm_agreement = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);

    Some(AudioFeatures {
        rekordbox_bpm: track.bpm,
        stratum_bpm,
        bpm_agreement,
        essentia_bpm: essentia_data.as_ref().and_then(|e| e.bpm_essentia),
        duration_seconds: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("duration_seconds"))
            .and_then(serde_json::Value::as_f64),
        danceability: essentia_data.as_ref().and_then(|e| e.danceability),
        dynamic_complexity: essentia_data.as_ref().and_then(|e| e.dynamic_complexity),
        rhythm_regularity: essentia_data.as_ref().and_then(|e| e.rhythm_regularity),
        spectral_centroid_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_centroid_mean),
        // Scalar features from Stratum
        decay_mid_tau: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("decay_mid_tau"))
            .and_then(serde_json::Value::as_f64),
        decay_high_tau: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("decay_high_tau"))
            .and_then(serde_json::Value::as_f64),
        key_clarity: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("key_clarity"))
            .and_then(serde_json::Value::as_f64),
        key_confidence: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("key_confidence"))
            .and_then(serde_json::Value::as_f64),
        // Scalar features from Essentia
        onset_rate: essentia_data.as_ref().and_then(|e| e.onset_rate),
        loudness_integrated: essentia_data.as_ref().and_then(|e| e.loudness_integrated),
        loudness_range: essentia_data.as_ref().and_then(|e| e.loudness_range),
        spectral_centroid_cv: essentia_data.as_ref().and_then(|e| e.spectral_centroid_cv),
        spectral_flux_mean: essentia_data.as_ref().and_then(|e| e.spectral_flux_mean),
        dissonance_mean: essentia_data.as_ref().and_then(|e| e.dissonance_mean),
        // Vector features from Essentia (for timbral distances)
        mfcc_mean: essentia_data.as_ref().and_then(|e| e.mfcc_mean.clone()),
        mfcc_std: essentia_data.as_ref().and_then(|e| e.mfcc_std.clone()),
        spectral_contrast_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_contrast_mean.clone()),
    })
}

fn current_audio_analysis(
    store_conn: &rusqlite::Connection,
    audio_key: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<store::CachedAudioAnalysis>, rusqlite::Error> {
    store::get_audio_analysis(store_conn, audio_key, analyzer)
        .map(|entry| entry.filter(|entry| entry.analysis_version == schema_version))
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

    // 2. Load audio features for each track
    let store_conn = server.cache_store_conn()?;
    let mut samples: Vec<(&'static str, AudioFeatures)> = Vec::new();
    let mut skipped_no_genre = 0u32;
    let mut skipped_no_audio = 0u32;
    let mut skipped_unknown_genre = 0u32;

    for track in &tracks {
        // Must have a genre tag
        if track.genre.is_empty() {
            skipped_no_genre += 1;
            continue;
        }

        // Resolve to canonical genre
        let canonical = match genre::resolve_genre(&track.genre) {
            Some(g) => g,
            None => {
                skipped_unknown_genre += 1;
                continue;
            }
        };

        // Load audio features
        let audio_key = super::analysis::resolved_audio_cache_key(&track.file_path);
        let stratum = current_audio_analysis(
            &store_conn,
            &audio_key,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .ok()
        .flatten();
        let essentia = current_audio_analysis(
            &store_conn,
            &audio_key,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .ok()
        .flatten();

        let stratum_cache = stratum.as_ref();
        let essentia_cache = essentia.as_ref();

        match extract_audio_features(track, stratum_cache, essentia_cache) {
            Some(features) => samples.push((canonical, features)),
            None => {
                skipped_no_audio += 1;
            }
        }
    }

    if samples.is_empty() {
        return Err(McpError::internal_error(
            "No tracks with both genre tags and audio features found.",
            None,
        ));
    }

    // 3. Calibrate
    let sample_refs: Vec<(&str, &AudioFeatures)> = samples.iter().map(|(g, f)| (*g, f)).collect();
    let registry = audio_profile::calibrate(&sample_refs);

    // 4. Save to SQLite
    audio_profile::save_to_db(&store_conn, &registry).map_err(super::cache_error)?;

    // 5. Build summary
    let mut genre_summaries: Vec<serde_json::Value> = registry
        .prototypes
        .values()
        .map(|proto| {
            let mut top_features: Vec<(&str, f64)> = proto
                .features
                .iter()
                .map(|(&name, stat)| (name, stat.fisher_weight))
                .collect();
            top_features.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            top_features.truncate(5);

            let feature_strs: Vec<String> = top_features
                .iter()
                .map(|(name, weight)| format!("{name} ({:.0}%)", weight * 100.0))
                .collect();

            serde_json::json!({
                "genre": proto.genre,
                "n_verified": proto.total_n,
                "n_features": proto.features.len(),
                "has_timbral": proto.mfcc_centroid.is_some(),
                "top_discriminators": feature_strs,
            })
        })
        .collect();
    genre_summaries.sort_by(|a, b| {
        let na = a["n_verified"].as_u64().unwrap_or(0);
        let nb = b["n_verified"].as_u64().unwrap_or(0);
        nb.cmp(&na)
    });

    let result = serde_json::json!({
        "status": "calibrated",
        "playlist": playlist_name,
        "total_tracks": tracks.len(),
        "tracks_with_features": samples.len(),
        "skipped_no_genre": skipped_no_genre,
        "skipped_unknown_genre": skipped_unknown_genre,
        "skipped_no_audio": skipped_no_audio,
        "prototypes_built": registry.prototypes.len(),
        "genres": genre_summaries,
    });

    ok_json(&result)
}

#[derive(Debug, Default)]
struct CalibrationGenreStats {
    playlist_tracks: u32,
    tracks_with_audio_features: u32,
    missing_audio_features: u32,
    tracks_with_stratum_features: u32,
    missing_stratum_features: u32,
    tracks_with_essentia_features: u32,
    missing_essentia_features: u32,
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

    let store_conn = server.cache_store_conn()?;
    let existing_registry = audio_profile::load_from_db(&store_conn).map_err(super::cache_error)?;
    let existing_profiles: HashMap<&'static str, u32> = existing_registry
        .as_ref()
        .map(|registry| {
            registry
                .prototypes
                .values()
                .map(|proto| (proto.genre, proto.total_n))
                .collect()
        })
        .unwrap_or_default();

    let mut by_genre: BTreeMap<&'static str, CalibrationGenreStats> = BTreeMap::new();
    let mut skipped_no_genre = 0u32;
    let mut skipped_unknown_genre = 0u32;

    for track in &tracks {
        if track.genre.trim().is_empty() {
            skipped_no_genre += 1;
            continue;
        }

        let Some(canonical) = genre::resolve_genre(&track.genre) else {
            skipped_unknown_genre += 1;
            continue;
        };

        let stats = by_genre.entry(canonical).or_default();
        stats.playlist_tracks += 1;

        let audio_key = super::analysis::resolved_audio_cache_key(&track.file_path);
        let stratum = current_audio_analysis(
            &store_conn,
            &audio_key,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(super::cache_error)?;
        let essentia = current_audio_analysis(
            &store_conn,
            &audio_key,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(super::cache_error)?;

        if stratum.is_some() {
            stats.tracks_with_stratum_features += 1;
        } else {
            stats.missing_stratum_features += 1;
        }
        if essentia.is_some() {
            stats.tracks_with_essentia_features += 1;
        } else {
            stats.missing_essentia_features += 1;
        }

        if extract_audio_features(track, stratum.as_ref(), essentia.as_ref()).is_some() {
            stats.tracks_with_audio_features += 1;
        } else {
            stats.missing_audio_features += 1;
        }
    }

    let mut ready_to_calibrate = 0u32;
    let mut below_min_tracks = 0u32;
    let mut stored_profiles_present = 0u32;
    let mut total_with_audio_features = 0u32;
    let mut total_missing_audio_features = 0u32;
    let mut total_with_stratum_features = 0u32;
    let mut total_missing_stratum_features = 0u32;
    let mut total_with_essentia_features = 0u32;
    let mut total_missing_essentia_features = 0u32;

    let genres: Vec<serde_json::Value> = by_genre
        .iter()
        .map(|(&genre, stats)| {
            let stored_n = existing_profiles.get(genre).copied();
            let prototype_ready = stats.tracks_with_audio_features >= audio_profile::MIN_TRACKS;
            if prototype_ready && stored_n.is_none() {
                ready_to_calibrate += 1;
            }
            if !prototype_ready {
                below_min_tracks += 1;
            }
            if stored_n.is_some() {
                stored_profiles_present += 1;
            }
            total_with_audio_features += stats.tracks_with_audio_features;
            total_missing_audio_features += stats.missing_audio_features;
            total_with_stratum_features += stats.tracks_with_stratum_features;
            total_missing_stratum_features += stats.missing_stratum_features;
            total_with_essentia_features += stats.tracks_with_essentia_features;
            total_missing_essentia_features += stats.missing_essentia_features;

            let status = if prototype_ready && stored_n.is_some() {
                "profile_present"
            } else if prototype_ready {
                "ready_to_calibrate"
            } else {
                "needs_more_verified_audio"
            };

            serde_json::json!({
                "genre": genre,
                "playlist_tracks": stats.playlist_tracks,
                "tracks_with_audio_features": stats.tracks_with_audio_features,
                "missing_audio_features": stats.missing_audio_features,
                "tracks_with_stratum_features": stats.tracks_with_stratum_features,
                "missing_stratum_features": stats.missing_stratum_features,
                "tracks_with_essentia_features": stats.tracks_with_essentia_features,
                "missing_essentia_features": stats.missing_essentia_features,
                "prototype_ready": prototype_ready,
                "profile": {
                    "stored": stored_n.is_some(),
                    "n_verified": stored_n,
                },
                "status": status,
            })
        })
        .collect();

    let playlist_genres: std::collections::HashSet<&str> = by_genre.keys().copied().collect();
    let mut stored_profiles_not_in_playlist: Vec<&str> = existing_profiles
        .keys()
        .filter(|genre| !playlist_genres.contains(**genre))
        .copied()
        .collect();
    stored_profiles_not_in_playlist.sort_unstable();

    let result = serde_json::json!({
        "status": "ok",
        "playlist": resolved_playlist_name,
        "total_tracks": tracks.len(),
        "tracks_with_canonical_genre": by_genre.values().map(|stats| stats.playlist_tracks).sum::<u32>(),
        "tracks_with_audio_features": total_with_audio_features,
        "missing_audio_features": total_missing_audio_features,
        "tracks_with_stratum_features": total_with_stratum_features,
        "missing_stratum_features": total_missing_stratum_features,
        "tracks_with_essentia_features": total_with_essentia_features,
        "missing_essentia_features": total_missing_essentia_features,
        "skipped_no_genre": skipped_no_genre,
        "skipped_unknown_genre": skipped_unknown_genre,
        "min_tracks_per_genre": audio_profile::MIN_TRACKS,
        "prototypes_existing": existing_profiles.len(),
        "genres_ready_to_calibrate": ready_to_calibrate,
        "genres_below_min_tracks": below_min_tracks,
        "genres_with_stored_profiles": stored_profiles_present,
        "stored_profiles_not_in_playlist": stored_profiles_not_in_playlist,
        "genres": genres,
    });

    ok_json(&result)
}
