use std::collections::{HashMap, HashSet};

use rmcp::ErrorData as McpError;
use rmcp::model::{CallToolResult, Content};

use super::*;
use crate::db;

pub(super) fn handle_score_transition(
    server: &ReklawdboxServer,
    params: ScoreTransitionParams,
) -> Result<CallToolResult, McpError> {
    let priority = params.priority.unwrap_or(SequencingPriority::Balanced);

    let (from_track, to_track) = {
        let conn = server.rekordbox_conn()?;
        let from = db::get_track(&conn, &params.source_track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Track '{}' not found", params.source_track_id),
                    None,
                )
            })?;
        let to = db::get_track(&conn, &params.target_track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Track '{}' not found", params.target_track_id),
                    None,
                )
            })?;
        (from, to)
    };

    let (from_profile, to_profile) = {
        let store = server.cache_store_conn()?;
        let from = build_track_profile(from_track, &store).map_err(|e| {
            mcp_internal_error(format!("Failed to build source track profile: {e}"))
        })?;
        let to = build_track_profile(to_track, &store).map_err(|e| {
            mcp_internal_error(format!("Failed to build destination track profile: {e}"))
        })?;
        (from, to)
    };

    let master_tempo = params.use_master_tempo.unwrap_or(true);
    let harmonic_style = Some(
        params
            .harmonic_style
            .unwrap_or(HarmonicMixingStyle::Balanced),
    );
    let scores = score_transition_profiles(
        &from_profile,
        &to_profile,
        params.energy_phase,
        params.energy_phase,
        priority,
        master_tempo,
        harmonic_style,
        &ScoringContext::default(),
        None,
    );

    let mut result = serde_json::json!({
        "from": {
            "track_id": from_profile.track.id,
            "title": from_profile.track.title,
            "artist": from_profile.track.artist,
            "key": from_profile.key_display,
            "bpm": round_to_3_decimals(from_profile.bpm),
            "energy": round_to_3_decimals(from_profile.energy),
            "genre": from_profile.track.genre,
        },
        "to": {
            "track_id": to_profile.track.id,
            "title": to_profile.track.title,
            "artist": to_profile.track.artist,
            "key": to_profile.key_display,
            "bpm": round_to_3_decimals(to_profile.bpm),
            "energy": round_to_3_decimals(to_profile.energy),
            "genre": to_profile.track.genre,
        },
        "scores": scores.to_json(),
    });
    result["key_relation"] = serde_json::json!(scores.key_relation);
    result["bpm_adjustment_pct"] =
        serde_json::json!(round_to_3_decimals(scores.bpm_adjustment_pct));
    if let Some(ref ek) = scores.effective_to_key {
        result["effective_to_key"] = serde_json::json!(ek);
    }
    if scores.pitch_shift_semitones != 0 {
        result["pitch_shift_semitones"] = serde_json::json!(scores.pitch_shift_semitones);
    }

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_query_transition_candidates(
    server: &ReklawdboxServer,
    params: QueryTransitionCandidatesParams,
) -> Result<CallToolResult, McpError> {
    if params.candidate_track_ids.is_none() && params.playlist_id.is_none() {
        return Err(McpError::invalid_params(
            "At least one of pool_track_ids or playlist_id must be provided".to_string(),
            None,
        ));
    }

    let priority = params.priority.unwrap_or(SequencingPriority::Balanced);
    let master_tempo = params.use_master_tempo.unwrap_or(true);
    let harmonic_style = Some(
        params
            .harmonic_style
            .unwrap_or(HarmonicMixingStyle::Balanced),
    );
    let limit = params.limit.unwrap_or(10).min(50) as usize;

    // Load from-track
    let from_track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.source_track_id)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("From track '{}' not found", params.source_track_id),
                    None,
                )
            })?
    };

    // Load pool tracks
    let pool_tracks = {
        let conn = server.rekordbox_conn()?;
        if let Some(ref ids) = params.candidate_track_ids {
            db::get_tracks_by_ids(&conn, ids)
                .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
        } else if let Some(ref playlist_id) = params.playlist_id {
            db::get_playlist_tracks(&conn, playlist_id, None)
                .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
        } else {
            vec![]
        }
    };

    if pool_tracks.is_empty() {
        return Err(McpError::invalid_params(
            "No tracks found in the specified pool".to_string(),
            None,
        ));
    }

    // Build profiles
    let from_profile = {
        let store = server.cache_store_conn()?;
        build_track_profile(from_track, &store)
            .map_err(|e| mcp_internal_error(format!("Failed to build source track profile: {e}")))?
    };

    let mut pool_profiles: Vec<TrackProfile> = Vec::new();
    let mut skipped_profiles = 0u32;
    {
        let store = server.cache_store_conn()?;
        for track in pool_tracks {
            if track.id == params.source_track_id {
                continue; // exclude from-track from pool
            }
            match build_track_profile(track, &store) {
                Ok(profile) => pool_profiles.push(profile),
                Err(_) => {
                    skipped_profiles += 1;
                    continue;
                }
            }
        }
    }

    // Score each candidate
    let ctx = ScoringContext::default();
    let reference_bpm = params.target_bpm.unwrap_or(from_profile.bpm);
    let play_bpms = params.target_bpm.map(|target| (from_profile.bpm, target));

    let mut scored: Vec<(TrackProfile, TransitionScores)> = pool_profiles
        .into_iter()
        .map(|to_profile| {
            let scores = score_transition_profiles(
                &from_profile,
                &to_profile,
                params.energy_phase,
                params.energy_phase,
                priority,
                master_tempo,
                harmonic_style,
                &ctx,
                play_bpms,
            );
            (to_profile, scores)
        })
        .collect();

    // Sort by composite descending
    scored.sort_by(|a, b| {
        b.1.composite
            .partial_cmp(&a.1.composite)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.track.id.cmp(&b.0.track.id))
    });
    let total_pool_size = scored.len();
    scored.truncate(limit);

    // Build output
    let candidates_json: Vec<serde_json::Value> = scored
        .iter()
        .map(|(profile, scores)| {
            let mut candidate = serde_json::json!({
                "track_id": profile.track.id,
                "title": profile.track.title,
                "artist": profile.track.artist,
                "native_bpm": round_to_3_decimals(profile.bpm),
                "native_key": profile.key_display,
                "bpm_difference_pct": round_to_3_decimals(scores.bpm_adjustment_pct),
                "key_relation": scores.key_relation,
                "scores": scores.to_json(),
            });

            // play_at_bpm and pitch fields only meaningful when targeting a specific BPM
            if let Some(target) = params.target_bpm {
                candidate["play_at_bpm"] = serde_json::json!(round_to_3_decimals(target));
                candidate["pitch_adjustment_pct"] =
                    serde_json::json!(round_to_3_decimals(scores.bpm_adjustment_pct));

                let pitch_shift = if !master_tempo && profile.bpm > 0.0 && target > 0.0 {
                    (12.0 * (target / profile.bpm).log2()).round() as i32
                } else {
                    0
                };
                if pitch_shift != 0 {
                    candidate["pitch_shift_semitones"] = serde_json::json!(pitch_shift);
                }
                if !master_tempo
                    && pitch_shift != 0
                    && let Some(ek) = profile
                        .camelot_key
                        .map(|k| format_camelot(transpose_camelot_key(k, pitch_shift)))
                {
                    candidate["effective_key"] = serde_json::json!(ek);
                }
            }

            candidate
        })
        .collect();

    let mut result = serde_json::json!({
        "from": {
            "track_id": from_profile.track.id,
            "title": from_profile.track.title,
            "artist": from_profile.track.artist,
            "native_bpm": round_to_3_decimals(from_profile.bpm),
            "key": from_profile.key_display,
            "energy": round_to_3_decimals(from_profile.energy),
            "genre": from_profile.track.genre,
        },
        "reference_bpm": round_to_3_decimals(reference_bpm),
        "master_tempo": master_tempo,
        "candidates": candidates_json,
        "total_pool_size": total_pool_size,
    });
    if skipped_profiles > 0 {
        result["skipped_profiles"] = serde_json::json!(skipped_profiles);
    }

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}

pub(super) fn handle_build_set(
    server: &ReklawdboxServer,
    params: BuildSetParams,
) -> Result<CallToolResult, McpError> {
    if params.track_ids.is_empty() {
        return Err(McpError::invalid_params(
            "track_ids must include at least one track".to_string(),
            None,
        ));
    }
    if params.target_tracks == 0 {
        return Err(McpError::invalid_params(
            "target_tracks must be at least 1".to_string(),
            None,
        ));
    }

    let mut seen = HashSet::new();
    let deduped_ids: Vec<String> = params
        .track_ids
        .into_iter()
        .filter(|track_id| seen.insert(track_id.clone()))
        .collect();
    if deduped_ids.is_empty() {
        return Err(McpError::invalid_params(
            "track_ids must include at least one unique track ID".to_string(),
            None,
        ));
    }

    // Resolve effective beam width: beam_width supersedes candidates
    let effective_beam_width = params
        .beam_width
        .unwrap_or_else(|| params.candidates.unwrap_or(3))
        .clamp(1, 8) as usize;
    let requested_target = params.target_tracks as usize;
    let priority = params.priority.unwrap_or(SequencingPriority::Balanced);

    let tracks = {
        let conn = server.rekordbox_conn()?;
        db::get_tracks_by_ids(&conn, &deduped_ids)
            .map_err(|e| mcp_internal_error(format!("DB error: {e}")))?
    };
    if tracks.is_empty() {
        return Err(McpError::invalid_params(
            "No valid tracks found for provided track_ids".to_string(),
            None,
        ));
    }

    let mut profiles_by_id: HashMap<String, TrackProfile> = HashMap::new();
    {
        let store = server.cache_store_conn()?;
        for track in tracks {
            let profile = build_track_profile(track, &store)
                .map_err(|e| mcp_internal_error(format!("Failed to build track profile: {e}")))?;
            profiles_by_id.insert(profile.track.id.clone(), profile);
        }
    }

    if let Some(opening_track_id) = params.opening_track_id.as_deref()
        && !profiles_by_id.contains_key(opening_track_id)
    {
        return Err(McpError::invalid_params(
            format!("opening_track_id '{opening_track_id}' is not in track_ids"),
            None,
        ));
    }

    let actual_target = requested_target.min(profiles_by_id.len());
    let phases = match params.energy_curve.as_ref() {
        Some(EnergyCurveInput::Custom(_)) => {
            let requested_phases =
                resolve_energy_curve(params.energy_curve.as_ref(), requested_target).map_err(
                    |e| McpError::invalid_params(format!("Invalid energy_curve: {e}"), None),
                )?;
            requested_phases.into_iter().take(actual_target).collect()
        }
        _ => resolve_energy_curve(params.energy_curve.as_ref(), actual_target)
            .map_err(|e| McpError::invalid_params(format!("Invalid energy_curve: {e}"), None))?,
    };

    // Compute BPM trajectory if bpm_range is set
    let bpm_trajectory = params
        .bpm_range
        .map(|(start, end)| compute_bpm_trajectory(&phases, start, end));

    let start_tracks = select_start_track_ids(
        &profiles_by_id,
        if profiles_by_id.len() <= actual_target {
            1
        } else {
            effective_beam_width
        },
        phases[0],
        params.opening_track_id.as_deref(),
    );

    let master_tempo = params.use_master_tempo.unwrap_or(true);
    let harmonic_style = Some(
        params
            .harmonic_style
            .unwrap_or(HarmonicMixingStyle::Balanced),
    );
    let bpm_drift_pct = params.bpm_drift_pct.unwrap_or(6.0);

    // Route: beam_width=1 -> greedy (backward compat), beam_width>=2 -> beam search
    let plans: Vec<CandidatePlan> = if effective_beam_width <= 1 {
        // Greedy path -- use variation via start tracks
        let effective_candidates = if profiles_by_id.len() <= actual_target {
            1
        } else {
            start_tracks.len()
        };
        (0..effective_candidates)
            .map(|i| {
                let start_id = start_tracks[i % start_tracks.len()].clone();
                build_candidate_plan(
                    &profiles_by_id,
                    &start_id,
                    actual_target,
                    &phases,
                    priority,
                    i,
                    master_tempo,
                    harmonic_style,
                    bpm_drift_pct,
                    bpm_trajectory.as_deref(),
                )
            })
            .collect()
    } else {
        // Beam search path
        let mut all_plans = Vec::new();
        for start_id in &start_tracks {
            let mut beam_plans = build_candidate_plan_beam(
                &profiles_by_id,
                start_id,
                actual_target,
                &phases,
                priority,
                effective_beam_width,
                master_tempo,
                harmonic_style,
                bpm_drift_pct,
                bpm_trajectory.as_deref(),
            );
            all_plans.append(&mut beam_plans);
        }
        // Deduplicate across start tracks
        let mut seen_track_sequences: HashSet<Vec<String>> = HashSet::new();
        all_plans.retain(|plan| seen_track_sequences.insert(plan.ordered_ids.clone()));
        // Sort by mean composite descending, keep top beam_width
        all_plans.sort_by(|a, b| {
            let a_mean = if a.transitions.is_empty() {
                0.0
            } else {
                a.transitions
                    .iter()
                    .map(|t| t.scores.composite)
                    .sum::<f64>()
                    / a.transitions.len() as f64
            };
            let b_mean = if b.transitions.is_empty() {
                0.0
            } else {
                b.transitions
                    .iter()
                    .map(|t| t.scores.composite)
                    .sum::<f64>()
                    / b.transitions.len() as f64
            };
            b_mean
                .partial_cmp(&a_mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_plans.truncate(effective_beam_width);
        all_plans
    };

    let mut candidates = Vec::with_capacity(plans.len());
    for (candidate_index, plan) in plans.into_iter().enumerate() {
        let tracks_json: Vec<serde_json::Value> = plan
            .ordered_ids
            .iter()
            .enumerate()
            .filter_map(|(pos, track_id)| {
                profiles_by_id.get(track_id).map(|profile| {
                    let mut track_json = serde_json::json!({
                        "track_id": profile.track.id,
                        "title": profile.track.title,
                        "artist": profile.track.artist,
                        "key": profile.key_display,
                        "bpm": profile.bpm,
                        "energy": profile.energy,
                        "genre": profile.track.genre,
                    });

                    // Enrich with BPM trajectory fields when bpm_range is set
                    if let Some(ref trajectory) = bpm_trajectory
                        && let Some(&target_bpm) = trajectory.get(pos)
                    {
                        track_json["play_at_bpm"] =
                            serde_json::json!(round_to_3_decimals(target_bpm));
                        let pct = if profile.bpm > 0.0 {
                            (target_bpm - profile.bpm).abs() / profile.bpm * 100.0
                        } else {
                            0.0
                        };
                        track_json["pitch_adjustment_pct"] =
                            serde_json::json!(round_to_3_decimals(pct));

                        if !master_tempo && profile.bpm > 0.0 {
                            let shift = (12.0 * (target_bpm / profile.bpm).log2()).round() as i32;
                            if shift != 0
                                && let Some(ek) = profile
                                    .camelot_key
                                    .map(|k| format_camelot(transpose_camelot_key(k, shift)))
                            {
                                track_json["effective_key"] = serde_json::json!(ek);
                            }
                        }
                    }

                    track_json
                })
            })
            .collect();

        let transitions_json: Vec<serde_json::Value> = plan
            .transitions
            .iter()
            .map(|transition| {
                let mut t = serde_json::json!({
                    "from_index": transition.from_index,
                    "to_index": transition.to_index,
                    "scores": transition.scores.to_json(),
                });
                t["key_relation"] = serde_json::json!(transition.scores.key_relation);
                t["bpm_adjustment_pct"] =
                    serde_json::json!(round_to_3_decimals(transition.scores.bpm_adjustment_pct));
                if let Some(ref ek) = transition.scores.effective_to_key {
                    t["effective_to_key"] = serde_json::json!(ek);
                }
                if transition.scores.pitch_shift_semitones != 0 {
                    t["pitch_shift_semitones"] =
                        serde_json::json!(transition.scores.pitch_shift_semitones);
                }
                t
            })
            .collect();

        let total_seconds: i32 = plan
            .ordered_ids
            .iter()
            .filter_map(|track_id| profiles_by_id.get(track_id))
            .map(|profile| {
                if profile.track.length > 0 {
                    profile.track.length
                } else {
                    6 * 60
                }
            })
            .sum();
        let estimated_duration_minutes = (total_seconds as f64 / 60.0).round() as i64;

        let mean_composite = if plan.transitions.is_empty() {
            0.0
        } else {
            plan.transitions
                .iter()
                .map(|transition| transition.scores.composite)
                .sum::<f64>()
                / plan.transitions.len() as f64
        };
        let set_score = round_to_3_decimals(mean_composite * 10.0);

        let candidate_label = ((b'A' + (candidate_index as u8)) as char).to_string();
        let mut candidate_json = serde_json::json!({
            "id": candidate_label,
            "tracks": tracks_json,
            "transitions": transitions_json,
            "set_score": set_score,
            "estimated_duration_minutes": estimated_duration_minutes,
        });

        if let Some(ref trajectory) = bpm_trajectory {
            candidate_json["bpm_trajectory"] = serde_json::json!(
                trajectory
                    .iter()
                    .map(|b| round_to_3_decimals(*b))
                    .collect::<Vec<f64>>()
            );
        }

        candidates.push(candidate_json);
    }

    let mut result = serde_json::json!({
        "candidates": candidates,
        "pool_size": profiles_by_id.len(),
        "tracks_used": actual_target,
        "beam_width": effective_beam_width,
    });

    if let Some(ref trajectory) = bpm_trajectory {
        result["bpm_trajectory"] = serde_json::json!(
            trajectory
                .iter()
                .map(|b| round_to_3_decimals(*b))
                .collect::<Vec<f64>>()
        );
    }

    let json =
        serde_json::to_string_pretty(&result).map_err(|e| mcp_internal_error(format!("{e}")))?;
    Ok(CallToolResult::success(vec![Content::text(json)]))
}
