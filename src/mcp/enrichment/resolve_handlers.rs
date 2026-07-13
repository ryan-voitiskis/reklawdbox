use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::audio;
use crate::adapters::rekordbox as db;
use crate::adapters::state as store;
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input,
    get_fresh_analysis_entry, resolved_audio_cache_key,
};
use crate::application::enrichment::resolve::{
    canonical_current_genre, resolve_cached_provider_data,
};
use crate::domain::classification::taxonomy as genre;
use crate::mcp::{
    ReklawdboxServer, ResolveFormat, ResolveTrackDataParams, ResolveTracksDataParams,
    ResolveTracksOpts, cache_error, db_error, mcp_internal_error, ok_json, resolve_tracks,
};

pub(in crate::mcp) fn handle_resolve_track_data(
    server: &ReklawdboxServer,
    params: ResolveTrackDataParams,
) -> Result<CallToolResult, McpError> {
    let track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{}' not found", params.track_id), None)
            })?
    };

    let norm_artist = crate::domain::metadata::normalize_for_matching(&track.artist);
    let norm_title = crate::domain::metadata::normalize_for_matching(&track.title);
    let norm_album = crate::domain::metadata::normalize_for_matching(&track.album);
    let norm_album = (!norm_album.is_empty()).then_some(norm_album);

    let essentia_installed = server.essentia_python_path().is_some();

    let (discogs_cache, beatport_cache, stratum_cache, essentia_cache) = {
        let store = server.cache_store_conn()?;
        let discogs_cache = store::get_enrichment(
            &store,
            "discogs",
            &norm_artist,
            &norm_title,
            norm_album.as_deref(),
            false,
        )
        .map_err(cache_error)?;
        let beatport_cache =
            store::get_enrichment(&store, "beatport", &norm_artist, &norm_title, None, false)
                .map_err(cache_error)?;
        let stratum_cache = get_fresh_analysis_entry(
            &store,
            &track.file_path,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(mcp_internal_error)?;
        let essentia_cache = get_fresh_analysis_entry(
            &store,
            &track.file_path,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(mcp_internal_error)?;
        (discogs_cache, beatport_cache, stratum_cache, essentia_cache)
    };

    let staged = server.context.mutation.changes.get(&track.id);

    let result = resolve_single_track(
        &track,
        discogs_cache.as_ref(),
        beatport_cache.as_ref(),
        stratum_cache.as_ref(),
        essentia_cache.as_ref(),
        essentia_installed,
        staged.as_ref(),
    );

    ok_json(&result)
}

pub(in crate::mcp) fn handle_resolve_tracks_data(
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
            None,
            &ResolveTracksOpts {
                default_max_tracks: Some(50),
                max_tracks_cap: Some(200),
                exclude_samplers: false,
            },
        )?
    };

    let params_format = params.format.unwrap_or_default();
    let essentia_installed = server.essentia_python_path().is_some();

    // Pre-compute normalized keys and resolved audio paths.
    let current_audio_identities = audio_cache_identities_with_current_stratum_input(
        tracks.iter().map(|track| track.file_path.as_str()),
    );
    let norm_keys: Vec<_> = tracks
        .iter()
        .zip(current_audio_identities)
        .map(|(t, audio_identity)| {
            let a = crate::domain::metadata::normalize_for_matching(&t.artist);
            let ti = crate::domain::metadata::normalize_for_matching(&t.title);
            let al = crate::domain::metadata::normalize_for_matching(&t.album);
            let audio_key = audio_identity
                .as_ref()
                .map(|identity| identity.cache_key.clone())
                .unwrap_or_else(|| resolved_audio_cache_key(&t.file_path));
            (
                a,
                ti,
                (!al.is_empty()).then_some(al),
                audio_key,
                audio_identity,
            )
        })
        .collect();

    // Build batch keys.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len() * 2);
    let stratum_identities: Vec<_> = norm_keys
        .iter()
        .filter_map(|(_, _, _, _, identity)| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = norm_keys
        .iter()
        .filter_map(|(_, _, _, _, identity)| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    for (a, t, al, _, _) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
        enrich_keys.push(("beatport", a, t, ""));
    }

    // Batch load — 3 queries total instead of 4N.
    let (enrich_map, stratum_map, essentia_map) = {
        let store = server.cache_store_conn()?;
        let enrich_map = store::batch_get_enrichment(&store, &enrich_keys).map_err(cache_error)?;
        let stratum_map = store::batch_get_fresh_audio_analysis(
            &store,
            &stratum_identities,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;
        let essentia_map = store::batch_get_fresh_audio_analysis(
            &store,
            &essentia_identities,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;
        (enrich_map, stratum_map, essentia_map)
    };

    let mut results = Vec::with_capacity(tracks.len());
    for (track, (norm_artist, norm_title, norm_album, audio_key, _)) in
        tracks.iter().zip(&norm_keys)
    {
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

        let result = match params_format {
            ResolveFormat::Full => {
                let staged = server.context.mutation.changes.get(&track.id);
                resolve_single_track(
                    track,
                    discogs_cache,
                    beatport_cache,
                    stratum_cache,
                    essentia_cache,
                    essentia_installed,
                    staged.as_ref(),
                )
            }
            ResolveFormat::Classification => resolve_single_track_compact(
                track,
                discogs_cache,
                beatport_cache,
                stratum_cache,
                essentia_cache,
            ),
        };
        results.push(result);
    }

    ok_json(&results)
}

/// Build the resolved JSON payload for a single track.
pub(crate) fn resolve_single_track(
    track: &crate::domain::library::Track,
    discogs_cache: Option<&store::EnrichmentCacheEntry>,
    beatport_cache: Option<&store::EnrichmentCacheEntry>,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
    essentia_installed: bool,
    staged: Option<&crate::domain::metadata::TrackChange>,
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
        let stratum_bpm = sj.get("bpm").and_then(serde_json::Value::as_f64);
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

    let provider_resolution =
        resolve_cached_provider_data(&track.label, discogs_cache, beatport_cache);

    let staged_val = staged.map(|s| {
        serde_json::json!({
            "genre": s.genre,
            "comments": s.comments,
            "rating": s.rating,
            "color": s.color,
            "label": s.label,
            "year": s.year,
            "album": s.album,
        })
    });

    let data_completeness = serde_json::json!({
        "rekordbox": true,
        "stratum_dsp": stratum_cache.is_some(),
        "essentia": essentia_cache.is_some(),
        "essentia_installed": essentia_installed,
        "discogs": provider_resolution.completeness.discogs_cached,
        "beatport": provider_resolution.completeness.beatport_cached,
    });

    let current_genre_canonical = canonical_current_genre(&track.genre)
        .map_or(serde_json::Value::Null, |canonical| {
            serde_json::json!(canonical)
        });

    let discogs_style_mappings: Vec<serde_json::Value> = provider_resolution
        .discogs_style_mappings
        .iter()
        .map(|mapping| {
            serde_json::json!({
                "style": mapping.raw,
                "maps_to": mapping.maps_to,
                "mapping_type": mapping.mapping_type,
            })
        })
        .collect();

    let beatport_genre_mapping =
        provider_resolution
            .beatport_genre_mapping
            .as_ref()
            .map(|mapping| {
                serde_json::json!({
                    "genre": mapping.raw,
                    "maps_to": mapping.maps_to,
                    "mapping_type": mapping.mapping_type,
                })
            });

    let effective_label = provider_resolution.effective_label.as_deref();
    let label_inferred_genre = provider_resolution.label_genre;

    let genre_taxonomy = serde_json::json!({
        "current_genre_canonical": current_genre_canonical,
        "discogs_style_mappings": discogs_style_mappings,
        "beatport_genre_mapping": beatport_genre_mapping,
        "label": effective_label,
        "label_genre": label_inferred_genre,
    });

    serde_json::json!({
        "track_id": track.id,
        "rekordbox": rekordbox,
        "audio_analysis": audio_analysis,
        "discogs": provider_resolution.discogs,
        "beatport": provider_resolution.beatport,
        "staged_changes": staged_val,
        "data_completeness": data_completeness,
        "genre_taxonomy": genre_taxonomy,
    })
}

/// Build a compact resolved JSON for classification workflows.
/// Returns only fields needed for the decision tree, ~400-500 bytes per track.
fn resolve_single_track_compact(
    track: &crate::domain::library::Track,
    discogs_cache: Option<&store::EnrichmentCacheEntry>,
    beatport_cache: Option<&store::EnrichmentCacheEntry>,
    stratum_cache: Option<&store::CachedAudioAnalysis>,
    essentia_cache: Option<&store::CachedAudioAnalysis>,
) -> serde_json::Value {
    let bpm_range_json = |genre: &str| -> serde_json::Value {
        genre::genre_bpm_range(genre).map_or(serde_json::Value::Null, |r| {
            serde_json::json!([r.typical_min, r.typical_max])
        })
    };

    let (current_genre_canonical, current_genre_bpm_range) = canonical_current_genre(&track.genre)
        .map_or(
            (serde_json::Value::Null, serde_json::Value::Null),
            |canonical| (serde_json::json!(canonical), bpm_range_json(canonical)),
        );

    let provider_resolution =
        resolve_cached_provider_data(&track.label, discogs_cache, beatport_cache);
    let effective_label = provider_resolution.effective_label.as_deref();
    let label_inferred_genre = provider_resolution.label_genre;

    // Group Discogs styles by canonical genre, keeping only exact/alias matches
    let discogs_mapped_genres: serde_json::Value = {
        if provider_resolution.discogs_mapped_genres.is_empty() {
            serde_json::Value::Null
        } else {
            let entries: Vec<serde_json::Value> = provider_resolution
                .discogs_mapped_genres
                .iter()
                .map(|(genre, count)| {
                    let bpm = bpm_range_json(genre);
                    serde_json::json!({"genre": genre, "style_count": count, "bpm_range": bpm})
                })
                .collect();
            serde_json::json!(entries)
        }
    };

    let beatport_genre_raw = provider_resolution
        .beatport_genre_raw
        .as_deref()
        .map_or(serde_json::Value::Null, |genre| serde_json::json!(genre));

    let beatport_mapped_genre = provider_resolution
        .beatport_mapped_genre
        .as_deref()
        .map_or(serde_json::Value::Null, |genre| serde_json::json!(genre));
    let beatport_mapped_genre_bpm_range = provider_resolution
        .beatport_mapped_genre
        .as_deref()
        .map_or(serde_json::Value::Null, bpm_range_json);

    let stratum_json = stratum_cache
        .and_then(|sc| serde_json::from_str::<serde_json::Value>(&sc.features_json).ok());

    let stratum_bpm = stratum_json
        .as_ref()
        .and_then(|sj| sj.get("bpm"))
        .and_then(serde_json::Value::as_f64);
    let stratum_key = stratum_json
        .as_ref()
        .and_then(|sj| sj.get("key"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let key_agreement = stratum_key
        .as_deref()
        .map(|sk| sk.eq_ignore_ascii_case(&track.key));
    let bpm_agreement = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);

    let essentia_data = essentia_cache
        .and_then(|ec| serde_json::from_str::<audio::EssentiaOutput>(&ec.features_json).ok());

    let audio_obj = if stratum_json.is_some() || essentia_data.is_some() {
        let mut obj = serde_json::json!({
            "stratum_bpm": stratum_bpm,
            "rekordbox_bpm": track.bpm,
            "bpm_agreement": bpm_agreement,
            "stratum_key": stratum_key,
            "key_agreement": key_agreement,
        });
        if let Some(ref ed) = essentia_data {
            obj["danceability"] = serde_json::json!(ed.danceability);
            obj["rhythm_regularity"] = serde_json::json!(ed.rhythm_regularity);
            obj["spectral_centroid_mean"] = serde_json::json!(ed.spectral_centroid_mean);
            obj["dynamic_complexity"] = serde_json::json!(ed.dynamic_complexity);
        } else {
            obj["danceability"] = serde_json::Value::Null;
            obj["rhythm_regularity"] = serde_json::Value::Null;
            obj["spectral_centroid_mean"] = serde_json::Value::Null;
            obj["dynamic_complexity"] = serde_json::Value::Null;
        }
        obj
    } else {
        serde_json::Value::Null
    };

    serde_json::json!({
        "track_id": track.id,
        "artist": track.artist,
        "title": track.title,
        "current_genre": track.genre,
        "current_genre_canonical": current_genre_canonical,
        "current_genre_bpm_range": current_genre_bpm_range,
        "bpm": track.bpm,
        "key": track.key,
        "rating": track.rating,
        "discogs_mapped_genres": discogs_mapped_genres,
        "beatport_mapped_genre": beatport_mapped_genre,
        "beatport_mapped_genre_bpm_range": beatport_mapped_genre_bpm_range,
        "beatport_genre_raw": beatport_genre_raw,
        "label": effective_label,
        "label_genre": label_inferred_genre,
        "audio": audio_obj,
        "data": {
            "stratum": stratum_cache.is_some(),
            "essentia": essentia_cache.is_some(),
            "discogs": provider_resolution.completeness.discogs_cached,
            "beatport": provider_resolution.completeness.beatport_cached,
        },
    })
}
