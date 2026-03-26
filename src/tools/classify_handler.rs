use std::collections::HashMap;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};
use tracing::warn;

use super::analysis::get_fresh_analysis_entry;
use super::resolve::*;
use super::scoring::map_genre_through_taxonomy;
use super::{ReklawdboxServer, mcp_internal_error};
use crate::audio;
use crate::classify::{
    AudioFeatures, ClassificationAction, ClassificationConfidence, ClassificationResult,
    MappedGenre, TrackEvidence, classify_track,
};
use crate::genre;
use crate::normalize;
use crate::store;
use crate::tools::params::{AuditGenresParams, ClassifyFormat, ClassifyTracksParams};

pub(super) fn handle_classify_tracks(
    server: &ReklawdboxServer,
    params: ClassifyTracksParams,
) -> Result<CallToolResult, McpError> {
    if let Some(ref ids) = params.track_ids {
        if ids.is_empty() {
            return Err(mcp_internal_error(
                "track_ids was provided but empty — nothing to classify.".into(),
            ));
        }
    }

    // Force has_genre=false when using filter-based selection. When track_ids
    // are provided, the caller has explicitly selected tracks — respect that
    // selection (e.g. unknown-genre tracks from suggest_normalizations).
    let mut filters = params.filters;
    if params.track_ids.is_none() {
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

    let format = params.format.unwrap_or_default();
    let output = match format {
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
                .map(|r| r.to_compact())
                .collect();
            serde_json::json!({
                "summary": summary,
                "results": compact,
            })
        }
    };

    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_audit_genres(
    server: &ReklawdboxServer,
    params: AuditGenresParams,
) -> Result<CallToolResult, McpError> {
    if let Some(ref ids) = params.track_ids {
        if ids.is_empty() {
            return Err(mcp_internal_error(
                "track_ids was provided but empty — nothing to audit.".into(),
            ));
        }
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

    let json =
        serde_json::to_string_pretty(&output).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
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

fn classify_batch(
    server: &ReklawdboxServer,
    tracks: &[crate::types::Track],
    overrides: &[(String, String)],
) -> Result<(Vec<ClassificationResult>, u32), McpError> {
    let mut results = Vec::with_capacity(tracks.len());
    let mut cache_errors = 0u32;

    for track in tracks {
        let evidence = build_track_evidence(server, track, overrides, &mut cache_errors)?;
        results.push(classify_track(&evidence));
    }

    Ok((results, cache_errors))
}

fn build_track_evidence(
    server: &ReklawdboxServer,
    track: &crate::types::Track,
    overrides: &[(String, String)],
    cache_errors: &mut u32,
) -> Result<TrackEvidence, McpError> {
    let norm_artist = normalize::normalize_for_matching(&track.artist);
    let norm_title = normalize::normalize_for_matching(&track.title);
    let norm_album = normalize::normalize_for_matching(&track.album);
    let norm_album = (!norm_album.is_empty()).then_some(norm_album);

    let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
        let store_conn = server.cache_store_conn()?;
        let discogs = store::get_enrichment(
            &store_conn,
            "discogs",
            &norm_artist,
            &norm_title,
            norm_album.as_deref(),
        )
        .unwrap_or_else(|e| {
            warn!(
                track_id = track.id.as_str(),
                "Cache read error (discogs): {e}"
            );
            *cache_errors += 1;
            None
        });
        let beatport =
            store::get_enrichment(&store_conn, "beatport", &norm_artist, &norm_title, None)
                .unwrap_or_else(|e| {
                    warn!(
                        track_id = track.id.as_str(),
                        "Cache read error (beatport): {e}"
                    );
                    *cache_errors += 1;
                    None
                });
        let stratum = get_fresh_analysis_entry(
            &store_conn,
            &track.file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .unwrap_or_else(|e| {
            warn!(
                track_id = track.id.as_str(),
                "Analysis cache error (stratum): {e}"
            );
            *cache_errors += 1;
            None
        });
        let essentia = get_fresh_analysis_entry(
            &store_conn,
            &track.file_path,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .unwrap_or_else(|e| {
            warn!(
                track_id = track.id.as_str(),
                "Analysis cache error (essentia): {e}"
            );
            *cache_errors += 1;
            None
        });
        (discogs, beatport, stratum, essentia)
    };

    let discogs_val = parse_response_json(discogs_cache.as_ref());
    let discogs_mapped = extract_discogs_genres(discogs_val.as_ref(), overrides);

    let beatport_val = parse_response_json(beatport_cache.as_ref());
    let (beatport_genre, beatport_raw) = extract_beatport_genre(beatport_val.as_ref(), overrides);

    let effective_label = if !track.label.is_empty() {
        Some(track.label.clone())
    } else {
        discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
            .map(|s| s.to_string())
    };
    let label_genre_val = effective_label.as_deref().and_then(genre::label_genre);

    // has_audio reflects whether features actually parsed, not just cache presence
    let audio = extract_audio_features(track, stratum_cache.as_ref(), essentia_cache.as_ref());
    let has_audio = audio.is_some();

    Ok(TrackEvidence {
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
    })
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
        if (mapping_type == "exact" || mapping_type == "alias")
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
    let canonical = if (mapping_type == "exact" || mapping_type == "alias") && maps_to.is_some() {
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
        .and_then(|v| v.as_f64());
    let bpm_agreement = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);

    Some(AudioFeatures {
        rekordbox_bpm: track.bpm,
        stratum_bpm,
        bpm_agreement,
        danceability: essentia_data.as_ref().and_then(|e| e.danceability),
        dynamic_complexity: essentia_data.as_ref().and_then(|e| e.dynamic_complexity),
        rhythm_regularity: essentia_data.as_ref().and_then(|e| e.rhythm_regularity),
        spectral_centroid_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_centroid_mean),
    })
}
