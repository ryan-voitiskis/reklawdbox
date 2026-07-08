use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

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
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(format!("Track '{}' not found", params.track_id), None)
            })?
    };

    let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
    let norm_title = crate::normalize::normalize_for_matching(&track.title);
    let norm_album = crate::normalize::normalize_for_matching(&track.album);
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

    ok_json(&result)
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
    let norm_keys: Vec<_> = tracks
        .iter()
        .map(|t| {
            let a = crate::normalize::normalize_for_matching(&t.artist);
            let ti = crate::normalize::normalize_for_matching(&t.title);
            let al = crate::normalize::normalize_for_matching(&t.album);
            let audio_identity = super::analysis::audio_cache_identity(&t.file_path);
            let audio_key = audio_identity
                .as_ref()
                .map(|identity| identity.cache_key.clone())
                .unwrap_or_else(|| super::analysis::resolved_audio_cache_key(&t.file_path));
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
    let audio_identities: Vec<_> = norm_keys
        .iter()
        .filter_map(|(_, _, _, _, identity)| {
            identity
                .as_ref()
                .map(super::analysis::AudioCacheIdentity::as_store_identity)
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
            &audio_identities,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;
        let essentia_map = store::batch_get_fresh_audio_analysis(
            &store,
            &audio_identities,
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
                let staged = server.state.changes.get(&track.id);
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
            .map_err(db_error)?
            .max(0) as usize;

        let tracks = resolve_tracks(
            &conn,
            params.track_ids.as_deref(),
            params.playlist_id.as_deref(),
            params.filters,
            params.max_tracks,
            None,
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
    let mut discogs_has_result = 0usize;
    let mut beatport_cached = 0usize;
    let mut beatport_has_result = 0usize;
    let mut no_audio_analysis = 0usize;
    let mut no_enrichment = 0usize;
    let mut no_data_at_all = 0usize;
    let mut has_label = 0usize;
    let mut no_label = 0usize;
    let mut enrichment_has_label = 0usize;

    {
        let track_keys: Vec<_> = tracks
            .iter()
            .map(|t| {
                let norm_artist = crate::normalize::normalize_for_matching(&t.artist);
                let norm_title = crate::normalize::normalize_for_matching(&t.title);
                let audio_identity = super::analysis::audio_cache_identity(&t.file_path);
                let audio_key = audio_identity
                    .as_ref()
                    .map(|identity| identity.cache_key.clone())
                    .unwrap_or_else(|| super::analysis::resolved_audio_cache_key(&t.file_path));
                (norm_artist, norm_title, audio_key, audio_identity)
            })
            .collect();

        let unique_artists: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            track_keys
                .iter()
                .filter_map(|(a, _, _, _)| {
                    if seen.insert(a.as_str()) {
                        Some(a.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        };

        let audio_identities: Vec<_> = track_keys
            .iter()
            .filter_map(|(_, _, _, identity)| {
                identity
                    .as_ref()
                    .map(super::analysis::AudioCacheIdentity::as_store_identity)
            })
            .collect();

        let store = server.cache_store_conn()?;

        let discogs_set = store::batch_enrichment_existence(&store, "discogs", &unique_artists)
            .map_err(cache_error)?;
        let beatport_set = store::batch_enrichment_existence(&store, "beatport", &unique_artists)
            .map_err(cache_error)?;
        let discogs_result_set =
            store::batch_enrichment_with_results(&store, "discogs", &unique_artists)
                .map_err(cache_error)?;
        let beatport_result_set =
            store::batch_enrichment_with_results(&store, "beatport", &unique_artists)
                .map_err(cache_error)?;
        let discogs_label_set =
            store::batch_enrichment_with_label(&store, "discogs", &unique_artists)
                .map_err(cache_error)?;
        let stratum_map = store::batch_get_fresh_audio_analysis(
            &store,
            &audio_identities,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;
        let essentia_map = store::batch_get_fresh_audio_analysis(
            &store,
            &audio_identities,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )
        .map_err(cache_error)?;

        // Build borrowed-key sets to avoid per-track clones during counting.
        let discogs_ref: std::collections::HashSet<(&str, &str)> = discogs_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let beatport_ref: std::collections::HashSet<(&str, &str)> = beatport_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let discogs_result_ref: std::collections::HashSet<(&str, &str)> = discogs_result_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let beatport_result_ref: std::collections::HashSet<(&str, &str)> = beatport_result_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        let discogs_label_ref: std::collections::HashSet<(&str, &str)> = discogs_label_set
            .iter()
            .map(|(a, t)| (a.as_str(), t.as_str()))
            .collect();
        for (idx, (norm_artist, norm_title, audio_key, _)) in track_keys.iter().enumerate() {
            let key = (norm_artist.as_str(), norm_title.as_str());
            let has_discogs = discogs_ref.contains(&key);
            let has_beatport = beatport_ref.contains(&key);
            let has_discogs_result = discogs_result_ref.contains(&key);
            let has_beatport_result = beatport_result_ref.contains(&key);
            let has_stratum = stratum_map.contains_key(audio_key);
            let has_essentia = essentia_map.contains_key(audio_key);

            if has_stratum {
                stratum_cached += 1;
            }
            if has_essentia {
                essentia_cached += 1;
            }
            if has_discogs {
                discogs_cached += 1;
            }
            if has_discogs_result {
                discogs_has_result += 1;
            }
            if has_beatport {
                beatport_cached += 1;
            }
            if has_beatport_result {
                beatport_has_result += 1;
            }
            if !has_stratum {
                no_audio_analysis += 1;
            }
            if !has_discogs_result && !has_beatport_result {
                no_enrichment += 1;
            }
            if !has_stratum && !has_essentia && !has_discogs_result && !has_beatport_result {
                no_data_at_all += 1;
            }

            let track_has_label = !tracks[idx].label.is_empty();
            if track_has_label {
                has_label += 1;
            } else {
                no_label += 1;
                if discogs_label_ref.contains(&key) {
                    enrichment_has_label += 1;
                }
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
                "searched": discogs_cached,
                "searched_percent": to_percent(discogs_cached, matched_tracks),
                "has_result": discogs_has_result,
                "has_result_percent": to_percent(discogs_has_result, matched_tracks),
            },
            "beatport": {
                "searched": beatport_cached,
                "searched_percent": to_percent(beatport_cached, matched_tracks),
                "has_result": beatport_has_result,
                "has_result_percent": to_percent(beatport_has_result, matched_tracks),
            },
        },
        "label": {
            "has_label": has_label,
            "has_label_percent": to_percent(has_label, matched_tracks),
            "no_label": no_label,
            "enrichment_has_label": enrichment_has_label,
        },
        "gaps": {
            "no_audio_analysis": no_audio_analysis,
            "no_enrichment": no_enrichment,
            "no_data_at_all": no_data_at_all,
        },
    });

    ok_json(&result)
}

/// Build the resolved JSON payload for a single track.
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

    let discogs_val = parse_enrichment_cache(discogs_cache);
    let beatport_val = parse_enrichment_cache(beatport_cache);

    let staged_val = staged.map(|s| {
        serde_json::json!({
            "genre": s.genre,
            "comments": s.comments,
            "rating": s.rating,
            "color": s.color,
            "label": s.label,
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

    let effective_label = if !track.label.is_empty() {
        Some(track.label.as_str())
    } else {
        discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
    };
    let label_inferred_genre = effective_label.and_then(genre::label_genre);

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
        "discogs": discogs_val,
        "beatport": beatport_val,
        "staged_changes": staged_val,
        "data_completeness": data_completeness,
        "genre_taxonomy": genre_taxonomy,
    })
}

/// Build a compact resolved JSON for classification workflows.
/// Returns only fields needed for the decision tree, ~400-500 bytes per track.
fn resolve_single_track_compact(
    track: &crate::types::Track,
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

    let (current_genre_canonical, current_genre_bpm_range) = if track.genre.is_empty() {
        (serde_json::Value::Null, serde_json::Value::Null)
    } else if let Some(canonical) = genre::canonical_genre_name(&track.genre) {
        (serde_json::json!(canonical), bpm_range_json(canonical))
    } else if let Some(canonical) = genre::canonical_genre_from_alias(&track.genre) {
        (serde_json::json!(canonical), bpm_range_json(canonical))
    } else {
        (serde_json::Value::Null, serde_json::Value::Null)
    };

    // Effective label: prefer Rekordbox, fall back to Discogs enrichment
    let discogs_val = parse_enrichment_cache(discogs_cache);
    let effective_label = if !track.label.is_empty() {
        Some(track.label.as_str())
    } else {
        discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
    };
    let label_inferred_genre = effective_label.and_then(genre::label_genre);

    // Group Discogs styles by canonical genre, keeping only exact/alias matches
    let discogs_mapped_genres: serde_json::Value = {
        let mut genre_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        if let Some(styles) = discogs_val
            .as_ref()
            .and_then(|v| v.get("styles"))
            .and_then(|v| v.as_array())
        {
            for style in styles.iter().filter_map(|s| s.as_str()) {
                let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
                if mapping_type != "unknown"
                    && let Some(genre_name) = maps_to
                {
                    *genre_counts.entry(genre_name).or_insert(0) += 1;
                }
            }
        }
        if genre_counts.is_empty() {
            serde_json::Value::Null
        } else {
            let mut entries: Vec<serde_json::Value> = genre_counts
                .into_iter()
                .map(|(g, count)| {
                    let bpm = bpm_range_json(&g);
                    serde_json::json!({"genre": g, "style_count": count, "bpm_range": bpm})
                })
                .collect();
            // Sort for deterministic output
            entries.sort_by(|a, b| {
                a.get("genre")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("genre").and_then(|v| v.as_str()).unwrap_or(""))
            });
            serde_json::json!(entries)
        }
    };

    let beatport_val = parse_enrichment_cache(beatport_cache);
    let bp_raw_str = beatport_val
        .as_ref()
        .and_then(|v| v.get("genre"))
        .and_then(|v| v.as_str())
        .filter(|g| !g.is_empty());

    let beatport_genre_raw = bp_raw_str.map_or(serde_json::Value::Null, |s| serde_json::json!(s));

    let bp_canonical = bp_raw_str.and_then(|bp_genre| {
        let (maps_to, mapping_type) = map_genre_through_taxonomy(bp_genre);
        if mapping_type != "unknown" {
            maps_to
        } else {
            None
        }
    });

    let beatport_mapped_genre = bp_canonical
        .as_deref()
        .map_or(serde_json::Value::Null, |g| serde_json::json!(g));
    let beatport_mapped_genre_bpm_range = bp_canonical
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
            "discogs": discogs_cache.is_some(),
            "beatport": beatport_cache.is_some(),
        },
    })
}

/// Parse a cached enrichment entry's response_json, injecting match_quality
/// and cached_at metadata into the returned object.
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
