use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use crate::adapters::rekordbox as db;
use crate::domain::planning::{PoolCohesionResult, TrackProfile, round_to_3_decimals};
use crate::mcp::planning::PoolAxisScoresPresentation;
use crate::mcp::{
    DescribePoolParams, DiscoverPoolsParams, ExpandPoolParams, ReklawdboxServer, ResolveTracksOpts,
    ScorePoolCompatibilityParams, db_error, mcp_internal_error, ok_json, resolve_pool_weights,
    resolve_tracks,
};

pub(in crate::mcp) fn handle_score_pool_compatibility(
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
            let evaluation = crate::application::planning::evaluate_pool_pair(
                [track_a, track_b],
                &store,
                master_tempo,
                params.reference_bpm,
                &weights,
            )
            .map_err(|e| mcp_internal_error(format!("Profile error: {e}")))?;

            let result = serde_json::json!({
                "mode": "pairwise",
                "track_a": track_summary(&evaluation.track_a),
                "track_b": track_summary(&evaluation.track_b),
                "reference_bpm": round_to_3_decimals(evaluation.reference_bpm),
                "master_tempo": master_tempo,
                "scores": evaluation.scores.to_json(),
            });

            ok_json(&result)
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
            let evaluation = crate::application::planning::evaluate_candidate_pool(
                candidate_track,
                pool_tracks,
                &store,
                master_tempo,
                params.reference_bpm,
                &weights,
            )
            .map_err(|error| {
                if error == "Failed to build any pool profiles" {
                    mcp_internal_error(error)
                } else {
                    mcp_internal_error(format!("Profile error: {error}"))
                }
            })?;

            let per_member_json: Vec<serde_json::Value> = evaluation
                .scores
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
                "candidate": track_summary(&evaluation.candidate),
                "reference_bpm": round_to_3_decimals(evaluation.reference_bpm),
                "master_tempo": master_tempo,
                "min_score": round_to_3_decimals(evaluation.scores.min_score),
                "mean_score": round_to_3_decimals(evaluation.scores.mean_score),
                "per_member": per_member_json,
            });
            if !evaluation.skipped.is_empty() {
                result["skipped_tracks"] = serde_json::json!(evaluation.skipped);
            }

            ok_json(&result)
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
            let evaluation = crate::application::planning::evaluate_pool_cohesion(
                pool_tracks,
                &store,
                master_tempo,
                params.reference_bpm,
                &weights,
            )
            .map_err(|error| McpError::invalid_params(error, None))?;

            let mut result =
                cohesion_to_json(&evaluation.cohesion, evaluation.reference_bpm, master_tempo);
            if !evaluation.skipped.is_empty() {
                result["skipped_tracks"] = serde_json::json!(evaluation.skipped);
            }

            ok_json(&result)
        }
    }
}

pub(in crate::mcp) fn handle_expand_pool(
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
    let seed = crate::application::planning::prepare_pool_expansion(
        seed_tracks,
        &store,
        params.reference_bpm,
    )
    .map_err(mcp_internal_error)?;

    // Intersect user BPM filters with seed-derived range
    let mut candidate_filters = params.filters;
    candidate_filters.bpm_min = Some(match candidate_filters.bpm_min {
        Some(user_min) => user_min.max(seed.bpm_low),
        None => seed.bpm_low,
    });
    candidate_filters.bpm_max = Some(match candidate_filters.bpm_max {
        Some(user_max) => user_max.min(seed.bpm_high),
        None => seed.bpm_high,
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

    let candidate_tracks: Vec<_> = candidate_tracks
        .into_iter()
        .filter(|track| !seed.track_ids.contains(&track.id))
        .collect();
    let expanded = crate::application::planning::expand_pool(
        seed,
        candidate_tracks,
        &store,
        additions,
        cross_genre,
        master_tempo,
        &weights,
    );

    let additions_json: Vec<serde_json::Value> = expanded
        .additions
        .iter()
        .map(|a| {
            serde_json::json!({
                "track_id": a.track_id,
                "title": a.title,
                "artist": a.artist,
                "min_score": round_to_3_decimals(a.min_score),
                "mean_score": round_to_3_decimals(a.mean_score),
                "rationale": {
                    "strongest_axes": a.rationale.strongest_axes,
                    "weakest_axis": a.rationale.weakest_axis,
                    "most_compatible_member": a.rationale.most_compatible_member,
                },
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "additions": additions_json,
        "pool_cohesion": {
            "mean_pairwise": round_to_3_decimals(expanded.final_cohesion.mean_pairwise),
            "min_pairwise": round_to_3_decimals(expanded.final_cohesion.min_pairwise),
        },
        "stopped_early": expanded.stopped_early,
        "candidates_scanned": expanded.candidates_scanned,
        "reference_bpm": round_to_3_decimals(expanded.reference_bpm),
        "master_tempo": master_tempo,
    });
    if !expanded.skipped_seed_tracks.is_empty() {
        result["skipped_seed_tracks"] = serde_json::json!(expanded.skipped_seed_tracks);
    }

    ok_json(&result)
}

pub(in crate::mcp) fn handle_describe_pool(
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
    let description = crate::application::planning::describe_pool(
        pool_tracks,
        &store,
        master_tempo,
        params.reference_bpm,
        &weights,
    )
    .map_err(mcp_internal_error)?;
    let weak_members: Vec<_> = description
        .weak_members
        .iter()
        .map(|member| {
            serde_json::json!({
                "track_id": member.track_id,
                "min_score_to_pool": round_to_3_decimals(member.min_score_to_pool),
            })
        })
        .collect();

    let mut result = serde_json::json!({
        "cohesion": {
            "mean_pairwise": round_to_3_decimals(description.cohesion.mean_pairwise),
            "min_pairwise": round_to_3_decimals(description.cohesion.min_pairwise),
        },
        "medoid_track_id": description.cohesion.medoid_id,
        "weak_members": weak_members,
        "energy_band": [round_to_3_decimals(description.energy_band.0), round_to_3_decimals(description.energy_band.1)],
        "bpm_center": round_to_3_decimals(description.bpm_center),
        "bpm_spread": round_to_3_decimals(description.bpm_spread),
        "key_neighborhood": description.key_neighborhood,
        "dominant_genre": description.dominant_genre,
        "analysis_coverage": round_to_3_decimals(description.analysis_coverage),
        "track_count": description.track_count,
        "master_tempo": master_tempo,
    });
    if !description.skipped.is_empty() {
        result["skipped_tracks"] = serde_json::json!(description.skipped);
    }

    if !master_tempo {
        result["reference_bpm_used"] =
            serde_json::json!(round_to_3_decimals(description.reference_bpm));
        if let Some((optimal_bpm, optimal_stability)) = description.optimal_reference {
            result["optimal_reference_bpm"] = serde_json::json!(round_to_3_decimals(optimal_bpm));
            result["key_stability_at_optimal"] =
                serde_json::json!(round_to_3_decimals(optimal_stability));
            result["key_stability_at_median"] = serde_json::json!(round_to_3_decimals(
                description
                    .median_key_stability
                    .expect("optimal reference has median stability"),
            ));
        } else {
            result["optimal_reference_bpm"] = serde_json::Value::Null;
            result["bpm_range_warning"] = serde_json::json!(
                "Pool spans too wide a BPM range for reliable harmonic evaluation at a single reference BPM"
            );
        }
    }

    ok_json(&result)
}

enum PoolMode {
    Pairwise,
    OneVsPool,
    Cohesion,
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

pub(in crate::mcp) fn handle_discover_pools(
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
    let discovered = crate::application::planning::discover_track_pools(
        tracks,
        &store,
        master_tempo,
        params.reference_bpm,
        &weights,
        threshold,
        min_size,
        max_size,
        max_pools,
    )
    .map_err(mcp_internal_error)?;
    let profiles = discovered.profiles;

    let pools_json: Vec<serde_json::Value> = discovered
        .pools
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

    let bridges_json: Vec<serde_json::Value> = discovered
        .bridges
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
        "reference_bpm": round_to_3_decimals(discovered.reference_bpm),
    });
    if !discovered.skipped.is_empty() {
        result["skipped_tracks"] = serde_json::json!(discovered.skipped);
    }

    ok_json(&result)
}
