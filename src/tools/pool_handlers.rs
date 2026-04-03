use std::collections::HashSet;

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::db;

pub(super) fn handle_score_pool_compatibility(
    server: &ReklawdboxServer,
    params: ScorePoolCompatibilityParams,
) -> Result<CallToolResult, McpError> {
    let master_tempo = params.master_tempo.unwrap_or(false);
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_pool_weights(params.preset.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };

    let mode = if params.track_a.is_some() && params.track_b.is_some() {
        PoolMode::Pairwise
    } else if params.track_id.is_some() && params.pool_track_ids.is_some() {
        PoolMode::OneVsPool
    } else if params.pool_track_ids.is_some() {
        PoolMode::Cohesion
    } else {
        return Err(McpError::invalid_params(
            "Provide track_a + track_b (pairwise), track_id + pool_track_ids (one-vs-pool), or pool_track_ids alone (cohesion)".to_string(),
            None,
        ));
    };

    match mode {
        PoolMode::Pairwise => {
            let id_a = params.track_a.expect("track_a is Some in Pairwise branch");
            let id_b = params.track_b.expect("track_b is Some in Pairwise branch");

            let (track_a, track_b) = {
                let conn = server.rekordbox_conn()?;
                let a = db::get_track(&conn, &id_a)
                    .map_err(db_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("Track '{id_a}' not found"), None)
                    })?;
                let b = db::get_track(&conn, &id_b)
                    .map_err(db_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("Track '{id_b}' not found"), None)
                    })?;
                (a, b)
            };

            let store = server.cache_store_conn()?;
            let profile_a = build_track_profile(track_a, &store)
                .map_err(|e| mcp_internal_error(format!("Profile error: {e}")))?;
            let profile_b = build_track_profile(track_b, &store)
                .map_err(|e| mcp_internal_error(format!("Profile error: {e}")))?;

            let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
                None
            });

            let ref_bpm = params
                .reference_bpm
                .unwrap_or_else(|| median_bpm(&[profile_a.bpm, profile_b.bpm]));

            let scores = score_pool_compatibility_pair(
                &profile_a,
                &profile_b,
                master_tempo,
                ref_bpm,
                &weights,
                norm_stats.as_ref(),
            );

            let result = serde_json::json!({
                "mode": "pairwise",
                "track_a": track_summary(&profile_a),
                "track_b": track_summary(&profile_b),
                "reference_bpm": round_to_3_decimals(ref_bpm),
                "master_tempo": master_tempo,
                "scores": scores.to_json(),
            });

            let json =
                serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }

        PoolMode::OneVsPool => {
            let candidate_id = params
                .track_id
                .expect("track_id is Some in OneVsPool branch");
            let pool_ids = params
                .pool_track_ids
                .expect("pool_track_ids is Some in OneVsPool branch");

            let (candidate_track, pool_tracks) = {
                let conn = server.rekordbox_conn()?;
                let candidate = db::get_track(&conn, &candidate_id)
                    .map_err(db_error)?
                    .ok_or_else(|| {
                        McpError::invalid_params(format!("Track '{candidate_id}' not found"), None)
                    })?;
                let pool = db::get_tracks_by_ids(&conn, &pool_ids).map_err(db_error)?;
                (candidate, pool)
            };

            if pool_tracks.is_empty() {
                return Err(McpError::invalid_params(
                    "No pool tracks found".to_string(),
                    None,
                ));
            }

            let store = server.cache_store_conn()?;
            let candidate_profile = build_track_profile(candidate_track, &store)
                .map_err(|e| mcp_internal_error(format!("Profile error: {e}")))?;

            let built = build_profiles(pool_tracks, &store);
            let pool_profiles = built.profiles;

            if pool_profiles.is_empty() {
                return Err(mcp_internal_error(
                    "Failed to build any pool profiles".to_string(),
                ));
            }

            let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
                None
            });

            let mut all_bpms: Vec<f64> = pool_profiles.iter().map(|p| p.bpm).collect();
            all_bpms.push(candidate_profile.bpm);
            let ref_bpm = params
                .reference_bpm
                .unwrap_or_else(|| median_bpm(&all_bpms));

            let pool_refs: Vec<&TrackProfile> = pool_profiles.iter().collect();
            let result_scores = score_candidate_vs_pool(
                &candidate_profile,
                &pool_refs,
                master_tempo,
                ref_bpm,
                &weights,
                norm_stats.as_ref(),
            );

            let per_member_json: Vec<serde_json::Value> = result_scores
                .per_member
                .iter()
                .map(|(id, scores)| {
                    serde_json::json!({
                        "pool_track_id": id,
                        "scores": scores.to_json(),
                    })
                })
                .collect();

            let mut result = serde_json::json!({
                "mode": "one_vs_pool",
                "candidate": track_summary(&candidate_profile),
                "reference_bpm": round_to_3_decimals(ref_bpm),
                "master_tempo": master_tempo,
                "min_score": round_to_3_decimals(result_scores.min_score),
                "mean_score": round_to_3_decimals(result_scores.mean_score),
                "per_member": per_member_json,
            });
            if !built.skipped.is_empty() {
                result["skipped_tracks"] = serde_json::json!(built.skipped);
            }

            let json =
                serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }

        PoolMode::Cohesion => {
            let pool_ids = params
                .pool_track_ids
                .expect("pool_track_ids is Some in Cohesion branch");

            if pool_ids.len() < 2 {
                return Err(McpError::invalid_params(
                    "Need at least 2 tracks for cohesion analysis".to_string(),
                    None,
                ));
            }

            let pool_tracks = {
                let conn = server.rekordbox_conn()?;
                db::get_tracks_by_ids(&conn, &pool_ids).map_err(db_error)?
            };

            let store = server.cache_store_conn()?;
            let built = build_profiles(pool_tracks, &store);
            let profiles = built.profiles;

            if profiles.len() < 2 {
                return Err(McpError::invalid_params(
                    "Need at least 2 valid profiles for cohesion".to_string(),
                    None,
                ));
            }

            let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
                None
            });

            let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();
            let ref_bpm = params.reference_bpm.unwrap_or_else(|| median_bpm(&bpms));

            let profile_refs: Vec<&TrackProfile> = profiles.iter().collect();
            let cohesion = compute_pool_cohesion(
                &profile_refs,
                master_tempo,
                ref_bpm,
                &weights,
                norm_stats.as_ref(),
            );

            let mut result = cohesion_to_json(&cohesion, ref_bpm, master_tempo);
            if !built.skipped.is_empty() {
                result["skipped_tracks"] = serde_json::json!(built.skipped);
            }

            let json =
                serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
            Ok(CallToolResult::success(vec![Content::text(json)]))
        }
    }
}

pub(super) fn handle_expand_pool(
    server: &ReklawdboxServer,
    params: ExpandPoolParams,
) -> Result<CallToolResult, McpError> {
    let master_tempo = params.master_tempo.unwrap_or(false);
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_pool_weights(params.preset.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };
    let additions = params.additions.unwrap_or(3).min(20) as usize;
    let cross_genre = params.cross_genre.unwrap_or(false);

    if params.seed_track_ids.is_empty() {
        return Err(McpError::invalid_params(
            "seed_track_ids must not be empty".to_string(),
            None,
        ));
    }

    let seed_tracks = {
        let conn = server.rekordbox_conn()?;
        db::get_tracks_by_ids(&conn, &params.seed_track_ids).map_err(db_error)?
    };

    if seed_tracks.is_empty() {
        return Err(McpError::invalid_params(
            "No seed tracks found".to_string(),
            None,
        ));
    }

    let store = server.cache_store_conn()?;
    let seed_built = build_profiles(seed_tracks, &store);
    let seed_profiles = seed_built.profiles;

    if seed_profiles.is_empty() {
        return Err(mcp_internal_error(
            "Failed to build any seed profiles".to_string(),
        ));
    }

    let seed_bpms: Vec<f64> = seed_profiles.iter().map(|p| p.bpm).collect();
    let ref_bpm = params
        .reference_bpm
        .unwrap_or_else(|| median_bpm(&seed_bpms));

    let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
        None
    });

    // seed_bpms guaranteed non-empty by the check above
    let min_seed_bpm = seed_bpms.iter().copied().reduce(f64::min).unwrap();
    let max_seed_bpm = seed_bpms.iter().copied().reduce(f64::max).unwrap();
    let bpm_low = min_seed_bpm * 0.92;
    let bpm_high = max_seed_bpm * 1.08;

    let seed_families: HashSet<GenreFamily> =
        seed_profiles.iter().map(|p| p.genre_family).collect();
    let seed_ids: HashSet<String> = seed_profiles.iter().map(|p| p.track.id.clone()).collect();

    // Intersect user BPM filters with seed-derived range
    let mut candidate_filters = params.filters;
    candidate_filters.bpm_min = Some(match candidate_filters.bpm_min {
        Some(user_min) => user_min.max(bpm_low),
        None => bpm_low,
    });
    candidate_filters.bpm_max = Some(match candidate_filters.bpm_max {
        Some(user_max) => user_max.min(bpm_high),
        None => bpm_high,
    });

    let candidate_tracks = {
        let conn = server.rekordbox_conn()?;
        resolve_tracks(
            &conn,
            None,
            params.playlist_id.as_deref(),
            candidate_filters,
            params.max_tracks,
            None,
            &ResolveTracksOpts {
                default_max_tracks: None,
                max_tracks_cap: None,
                exclude_samplers: true,
            },
        )?
    };

    let mut candidate_profiles: Vec<TrackProfile> = Vec::new();
    for track in candidate_tracks {
        if seed_ids.contains(&track.id) {
            continue;
        }
        match build_track_profile(track, &store) {
            Ok(p) => {
                if !cross_genre && !seed_families.contains(&p.genre_family) {
                    continue;
                }
                candidate_profiles.push(p);
            }
            Err(_) => continue,
        }
    }

    let candidates_scanned = candidate_profiles.len();

    let mut pool: Vec<TrackProfile> = seed_profiles;
    let mut added: Vec<AdditionResult> = Vec::new();
    let quality_threshold = 0.4;

    for _ in 0..additions {
        if candidate_profiles.is_empty() {
            break;
        }

        let pool_refs: Vec<&TrackProfile> = pool.iter().collect();

        let mut best_idx = 0;
        let mut best_min = f64::NEG_INFINITY;
        let mut best_mean = 0.0;
        let mut best_result: Option<CandidatePoolScore> = None;

        for (i, candidate) in candidate_profiles.iter().enumerate() {
            let result = score_candidate_vs_pool(
                candidate,
                &pool_refs,
                master_tempo,
                ref_bpm,
                &weights,
                norm_stats.as_ref(),
            );

            // Rank by min_score, tiebreak by mean_score
            if result.min_score > best_min
                || (result.min_score == best_min && result.mean_score > best_mean)
            {
                best_idx = i;
                best_min = result.min_score;
                best_mean = result.mean_score;
                best_result = Some(result);
            }
        }

        if best_min < quality_threshold {
            break;
        }

        let chosen = candidate_profiles.swap_remove(best_idx);
        let result = best_result.unwrap();

        let rationale = build_addition_rationale(&result);

        added.push(AdditionResult {
            track_id: chosen.track.id.clone(),
            title: chosen.track.title.clone(),
            artist: chosen.track.artist.clone(),
            min_score: result.min_score,
            mean_score: result.mean_score,
            rationale,
        });

        pool.push(chosen);
    }

    let stopped_early = added.len() < additions;

    let pool_refs: Vec<&TrackProfile> = pool.iter().collect();
    let final_cohesion = compute_pool_cohesion(
        &pool_refs,
        master_tempo,
        ref_bpm,
        &weights,
        norm_stats.as_ref(),
    );

    let additions_json: Vec<serde_json::Value> = added
        .iter()
        .map(|a| {
            serde_json::json!({
                "track_id": a.track_id,
                "title": a.title,
                "artist": a.artist,
                "min_score": round_to_3_decimals(a.min_score),
                "mean_score": round_to_3_decimals(a.mean_score),
                "rationale": a.rationale,
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "additions": additions_json,
        "pool_cohesion": {
            "mean_pairwise": round_to_3_decimals(final_cohesion.mean_pairwise),
            "min_pairwise": round_to_3_decimals(final_cohesion.min_pairwise),
        },
        "stopped_early": stopped_early,
        "candidates_scanned": candidates_scanned,
        "reference_bpm": round_to_3_decimals(ref_bpm),
        "master_tempo": master_tempo,
    });
    if !seed_built.skipped.is_empty() {
        result["skipped_seed_tracks"] = serde_json::json!(seed_built.skipped);
    }

    let json = serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_describe_pool(
    server: &ReklawdboxServer,
    params: DescribePoolParams,
) -> Result<CallToolResult, McpError> {
    let master_tempo = params.master_tempo.unwrap_or(false);
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_pool_weights(params.preset.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };

    if params.pool_track_ids.is_none() && params.playlist_id.is_none() {
        return Err(McpError::invalid_params(
            "Provide pool_track_ids or playlist_id".to_string(),
            None,
        ));
    }

    let pool_tracks = {
        let conn = server.rekordbox_conn()?;
        if let Some(ref ids) = params.pool_track_ids {
            db::get_tracks_by_ids(&conn, ids).map_err(db_error)?
        } else {
            let pid = params.playlist_id.as_ref().unwrap();
            db::get_playlist_tracks(&conn, pid, None).map_err(db_error)?
        }
    };

    if pool_tracks.len() < 2 {
        return Err(McpError::invalid_params(
            "Need at least 2 tracks to describe a pool".to_string(),
            None,
        ));
    }

    let store = server.cache_store_conn()?;
    let built = build_profiles(pool_tracks, &store);
    let profiles = built.profiles;
    let essentia_count = profiles.iter().filter(|p| p.timbral.is_some()).count();

    if profiles.len() < 2 {
        return Err(mcp_internal_error(
            "Failed to build enough profiles".to_string(),
        ));
    }

    let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
        None
    });

    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();
    let ref_bpm = params.reference_bpm.unwrap_or_else(|| median_bpm(&bpms));

    let profile_refs: Vec<&TrackProfile> = profiles.iter().collect();
    let cohesion = compute_pool_cohesion(
        &profile_refs,
        master_tempo,
        ref_bpm,
        &weights,
        norm_stats.as_ref(),
    );

    // profiles guaranteed >=2 by the check above
    let energies: Vec<f64> = profiles.iter().map(|p| p.energy).collect();
    let energy_min = energies.iter().copied().reduce(f64::min).unwrap();
    let energy_max = energies.iter().copied().reduce(f64::max).unwrap();

    let bpm_min = bpms.iter().copied().reduce(f64::min).unwrap();
    let bpm_max = bpms.iter().copied().reduce(f64::max).unwrap();
    let bpm_center = median_bpm(&bpms);
    let bpm_spread = bpm_max - bpm_min;

    let key_neighborhood: Vec<String> = profiles
        .iter()
        .filter_map(|p| {
            let k = p.camelot_key?;
            if !master_tempo && ref_bpm > 0.0 {
                let shift = bpm_pitch_shift(p.bpm, ref_bpm).round() as i32;
                Some(format_camelot(transpose_camelot_key(k, shift)))
            } else {
                Some(format_camelot(k))
            }
        })
        .collect();

    let mut genre_counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for p in &profiles {
        if let Some(ref g) = p.canonical_genre {
            *genre_counts.entry(g.as_str()).or_default() += 1;
        }
    }
    let dominant_genre = genre_counts
        .into_iter()
        .max_by_key(|(_, c)| *c)
        .map(|(g, _)| g.to_string());

    // Tracks with min-compatibility < 0.5 to any pool member
    let weak_candidates: HashSet<&str> = cohesion
        .per_pair
        .iter()
        .filter(|(_, _, s)| s.composite < 0.5)
        .flat_map(|(a, b, _)| [a.as_str(), b.as_str()])
        .collect();

    let weak_members: Vec<serde_json::Value> = weak_candidates
        .into_iter()
        .filter_map(|id| {
            let min_score = cohesion
                .per_pair
                .iter()
                .filter(|(a, b, _)| a == id || b == id)
                .map(|(_, _, s)| s.composite)
                .reduce(f64::min)?;
            (min_score < 0.5).then(|| {
                serde_json::json!({
                    "track_id": id,
                    "min_score_to_pool": round_to_3_decimals(min_score),
                })
            })
        })
        .collect();

    let analysis_coverage = essentia_count as f64 / profiles.len() as f64;

    let mut result = serde_json::json!({
        "cohesion": {
            "mean_pairwise": round_to_3_decimals(cohesion.mean_pairwise),
            "min_pairwise": round_to_3_decimals(cohesion.min_pairwise),
        },
        "medoid_track_id": cohesion.medoid_id,
        "weak_members": weak_members,
        "energy_band": [round_to_3_decimals(energy_min), round_to_3_decimals(energy_max)],
        "bpm_center": round_to_3_decimals(bpm_center),
        "bpm_spread": round_to_3_decimals(bpm_spread),
        "key_neighborhood": key_neighborhood,
        "dominant_genre": dominant_genre,
        "analysis_coverage": round_to_3_decimals(analysis_coverage),
        "track_count": profiles.len(),
        "master_tempo": master_tempo,
    });
    if !built.skipped.is_empty() {
        result["skipped_tracks"] = serde_json::json!(built.skipped);
    }

    if !master_tempo {
        let median_ref = median_bpm(&bpms);

        let sweep_result = sweep_optimal_reference_bpm(&profiles, &bpms);

        result["reference_bpm_used"] = serde_json::json!(round_to_3_decimals(ref_bpm));
        if let Some((optimal_bpm, optimal_stability)) = sweep_result {
            let median_stability = compute_key_stability_at_bpm(&profiles, median_ref);
            result["optimal_reference_bpm"] = serde_json::json!(round_to_3_decimals(optimal_bpm));
            result["key_stability_at_optimal"] =
                serde_json::json!(round_to_3_decimals(optimal_stability));
            result["key_stability_at_median"] =
                serde_json::json!(round_to_3_decimals(median_stability));
        } else {
            result["optimal_reference_bpm"] = serde_json::Value::Null;
            result["bpm_range_warning"] = serde_json::json!(
                "Pool spans too wide a BPM range for reliable harmonic evaluation at a single reference BPM"
            );
        }
    }

    let json = serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

struct BuildProfilesResult {
    profiles: Vec<TrackProfile>,
    skipped: Vec<String>,
}

fn build_profiles(tracks: Vec<crate::types::Track>, store: &Connection) -> BuildProfilesResult {
    let mut profiles = Vec::new();
    let mut skipped = Vec::new();
    for track in tracks {
        let id = track.id.clone();
        match build_track_profile(track, store) {
            Ok(p) => profiles.push(p),
            Err(e) => {
                tracing::warn!(track_id = %id, error = %e, "Skipping track: profile build failed");
                skipped.push(id);
            }
        }
    }
    BuildProfilesResult { profiles, skipped }
}

enum PoolMode {
    Pairwise,
    OneVsPool,
    Cohesion,
}

struct AdditionResult {
    track_id: String,
    title: String,
    artist: String,
    min_score: f64,
    mean_score: f64,
    rationale: serde_json::Value,
}

fn track_summary(p: &TrackProfile) -> serde_json::Value {
    serde_json::json!({
        "track_id": p.track.id,
        "title": p.track.title,
        "artist": p.track.artist,
        "key": p.key_display,
        "bpm": round_to_3_decimals(p.bpm),
        "energy": round_to_3_decimals(p.energy),
        "genre": p.track.genre,
    })
}

fn median_bpm(bpms: &[f64]) -> f64 {
    if bpms.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f64> = bpms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn cohesion_to_json(
    cohesion: &PoolCohesionResult,
    ref_bpm: f64,
    master_tempo: bool,
) -> serde_json::Value {
    let per_pair_json: Vec<serde_json::Value> = cohesion
        .per_pair
        .iter()
        .map(|(id_a, id_b, scores)| {
            serde_json::json!({
                "track_a": id_a,
                "track_b": id_b,
                "scores": scores.to_json(),
            })
        })
        .collect();

    serde_json::json!({
        "mode": "cohesion",
        "reference_bpm": round_to_3_decimals(ref_bpm),
        "master_tempo": master_tempo,
        "mean_pairwise": round_to_3_decimals(cohesion.mean_pairwise),
        "min_pairwise": round_to_3_decimals(cohesion.min_pairwise),
        "weakest_member_id": cohesion.weakest_member_id,
        "medoid_id": cohesion.medoid_id,
        "per_pair": per_pair_json,
    })
}

fn build_addition_rationale(result: &CandidatePoolScore) -> serde_json::Value {
    if result.per_member.is_empty() {
        return serde_json::json!({});
    }

    let most_compatible_id = result
        .per_member
        .iter()
        .max_by(|(_, a), (_, b)| {
            a.composite
                .partial_cmp(&b.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map_or("", |(id, _)| id.as_str());

    let mut axis_sums: std::collections::HashMap<&str, (f64, u32)> =
        std::collections::HashMap::new();
    for (_, scores) in &result.per_member {
        for (name, value) in [
            ("key", scores.key.value),
            ("bpm", scores.bpm.value),
            ("energy", scores.energy.value),
            ("genre", scores.genre.value),
            ("brightness", scores.brightness.value),
            ("rhythm", scores.rhythm.value),
        ] {
            let entry = axis_sums.entry(name).or_insert((0.0, 0));
            entry.0 += value;
            entry.1 += 1;
        }
        if let Some(ref t) = scores.timbral {
            let entry = axis_sums.entry("timbral").or_insert((0.0, 0));
            entry.0 += t.value;
            entry.1 += 1;
        }
    }

    let mut axis_means: Vec<(&str, f64)> = axis_sums
        .iter()
        .map(|(name, (sum, count))| (*name, sum / *count as f64))
        .collect();
    axis_means.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));

    let strongest: Vec<&str> = axis_means
        .iter()
        .take(2)
        .filter(|(_, v)| *v >= 0.7)
        .map(|(name, _)| *name)
        .collect();
    let weakest = axis_means
        .last()
        .map_or("unknown", |(name, _)| *name);

    serde_json::json!({
        "strongest_axes": strongest,
        "weakest_axis": weakest,
        "most_compatible_member": most_compatible_id,
    })
}

/// Sweep reference BPMs to find the one maximizing key compatibility.
/// Returns None if the BPM range is too wide for any single reference.
pub(super) fn sweep_optimal_reference_bpm(
    profiles: &[TrackProfile],
    bpms: &[f64],
) -> Option<(f64, f64)> {
    // Compute the valid reference BPM interval analytically.
    // For each track at BPM X, the 1-semitone constraint gives:
    //   X / 2^(1/12) <= ref <= X * 2^(1/12)
    // The valid interval is the intersection of all per-track intervals.
    let semitone_ratio = 2.0_f64.powf(1.0 / 12.0); // ~1.0595
    let mut interval_lo = f64::NEG_INFINITY;
    let mut interval_hi = f64::INFINITY;

    for &bpm in bpms {
        if bpm <= 0.0 {
            continue;
        }
        let lo = bpm / semitone_ratio;
        let hi = bpm * semitone_ratio;
        if lo > interval_lo {
            interval_lo = lo;
        }
        if hi < interval_hi {
            interval_hi = hi;
        }
    }

    if interval_lo > interval_hi || interval_lo <= 0.0 {
        return None;
    }

    let step = 0.1;
    let mut best_bpm = interval_lo;
    let mut best_stability = f64::NEG_INFINITY;
    let mut ref_bpm = interval_lo;

    while ref_bpm <= interval_hi {
        let stability = compute_key_stability_at_bpm(profiles, ref_bpm);
        if stability > best_stability {
            best_stability = stability;
            best_bpm = ref_bpm;
        }
        ref_bpm += step;
    }

    // Also check the interval boundaries (may not land on a grid point)
    let stability_hi = compute_key_stability_at_bpm(profiles, interval_hi);
    if stability_hi > best_stability {
        best_stability = stability_hi;
        best_bpm = interval_hi;
    }

    if best_stability > f64::NEG_INFINITY {
        Some((best_bpm, best_stability))
    } else {
        None
    }
}

/// Compute mean key axis score across all pairs at a given reference BPM.
fn compute_key_stability_at_bpm(profiles: &[TrackProfile], ref_bpm: f64) -> f64 {
    let n = profiles.len();
    if n < 2 {
        return 1.0;
    }

    let mut sum = 0.0;
    let mut count = 0u32;

    for i in 0..n {
        for j in (i + 1)..n {
            let key_score = score_key_with_pitch_shifts(
                profiles[i].camelot_key,
                profiles[j].camelot_key,
                bpm_pitch_shift(profiles[i].bpm, ref_bpm),
                bpm_pitch_shift(profiles[j].bpm, ref_bpm),
            );
            sum += key_score.value;
            count += 1;
        }
    }

    if count > 0 { sum / count as f64 } else { 1.0 }
}

pub(super) fn handle_discover_pools(
    server: &ReklawdboxServer,
    params: DiscoverPoolsParams,
) -> Result<CallToolResult, McpError> {
    let master_tempo = params.master_tempo.unwrap_or(false);
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_pool_weights(params.preset.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };
    let threshold = params.threshold.unwrap_or(0.7).clamp(0.3, 0.95);
    let min_size = params.min_pool_size.unwrap_or(3).max(2) as usize;
    let max_size = params
        .max_pool_size
        .unwrap_or(12)
        .clamp(min_size as u32, 20) as usize;
    let max_pools = params.max_pools.unwrap_or(10).min(50) as usize;

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
                default_max_tracks: Some(200),
                max_tracks_cap: Some(500),
                exclude_samplers: true,
            },
        )?
    };

    if tracks.len() < min_size {
        return Err(McpError::invalid_params(
            format!("Need at least {min_size} tracks, got {}", tracks.len()),
            None,
        ));
    }

    let store = server.cache_store_conn()?;
    let built = build_profiles(tracks, &store);
    let profiles = built.profiles;

    if profiles.len() < min_size {
        return Err(mcp_internal_error(format!(
            "Only {} profiles built (need {min_size})",
            profiles.len()
        )));
    }

    let norm_stats = ensure_timbral_norm_stats(&store).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "Timbral norm stats unavailable, scoring without timbral axis");
        None
    });

    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();
    let ref_bpm = params.reference_bpm.unwrap_or_else(|| median_bpm(&bpms));

    let profile_refs: Vec<&TrackProfile> = profiles.iter().collect();
    let pools = discover_pools(
        &profile_refs,
        master_tempo,
        ref_bpm,
        &weights,
        norm_stats.as_ref(),
        threshold,
        min_size,
        max_size,
        max_pools,
    );

    let bridges = find_bridge_tracks(&pools);

    let pools_json: Vec<serde_json::Value> = pools
        .iter()
        .enumerate()
        .map(|(idx, pool)| {
            let pool_profiles: Vec<&TrackProfile> = pool
                .track_ids
                .iter()
                .filter_map(|id| profiles.iter().find(|p| p.track.id == *id))
                .collect();

            let bpm_min = pool_profiles
                .iter()
                .map(|p| p.bpm)
                .reduce(f64::min)
                .unwrap_or(0.0);
            let bpm_max = pool_profiles
                .iter()
                .map(|p| p.bpm)
                .reduce(f64::max)
                .unwrap_or(0.0);
            let energy_min = pool_profiles
                .iter()
                .map(|p| p.energy)
                .reduce(f64::min)
                .unwrap_or(0.0);
            let energy_max = pool_profiles
                .iter()
                .map(|p| p.energy)
                .reduce(f64::max)
                .unwrap_or(0.0);

            let mut genre_counts: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for p in &pool_profiles {
                if let Some(ref g) = p.canonical_genre {
                    *genre_counts.entry(g.as_str()).or_default() += 1;
                }
            }
            let dominant_genre = genre_counts
                .into_iter()
                .max_by_key(|(_, c)| *c)
                .map(|(g, _)| g.to_string());

            let tracks_json: Vec<serde_json::Value> =
                pool_profiles.iter().map(|p| track_summary(p)).collect();

            serde_json::json!({
                "pool_index": idx,
                "size": pool.track_ids.len(),
                "mean_compatibility": round_to_3_decimals(pool.mean_compatibility),
                "min_compatibility": round_to_3_decimals(pool.min_compatibility),
                "score": round_to_3_decimals(pool.score),
                "core_members": pool.core_members,
                "edge_members": pool.edge_members,
                "bpm_range": [round_to_3_decimals(bpm_min), round_to_3_decimals(bpm_max)],
                "energy_range": [round_to_3_decimals(energy_min), round_to_3_decimals(energy_max)],
                "dominant_genre": dominant_genre,
                "tracks": tracks_json,
            })
        })
        .collect();

    let bridges_json: Vec<serde_json::Value> = bridges
        .iter()
        .filter_map(|(id, pool_indices)| {
            profiles.iter().find(|p| p.track.id == *id).map(|p| {
                serde_json::json!({
                    "track": track_summary(p),
                    "appears_in_pools": pool_indices,
                })
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "pools": pools_json,
        "bridge_tracks": bridges_json,
        "tracks_analyzed": profiles.len(),
        "threshold": threshold,
        "master_tempo": master_tempo,
        "reference_bpm": round_to_3_decimals(ref_bpm),
    });
    if !built.skipped.is_empty() {
        result["skipped_tracks"] = serde_json::json!(built.skipped);
    }

    let json = serde_json::to_string(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
