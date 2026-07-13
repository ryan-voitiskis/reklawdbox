use std::collections::HashSet;

use rmcp::ErrorData as McpError;
use rmcp::model::CallToolResult;

use super::*;
use crate::db;

pub(super) fn handle_score_transition(
    server: &ReklawdboxServer,
    params: ScoreTransitionParams,
) -> Result<CallToolResult, McpError> {
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_transition_weights(params.priority.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };

    let (from_track, to_track) = {
        let conn = server.rekordbox_conn()?;
        let from = db::get_track(&conn, &params.source_track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Track '{}' not found", params.source_track_id),
                    None,
                )
            })?;
        let to = db::get_track(&conn, &params.target_track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("Track '{}' not found", params.target_track_id),
                    None,
                )
            })?;
        (from, to)
    };

    let evaluation = {
        let store = server.cache_store_conn()?;
        crate::application::planning::evaluate_transition(
            from_track,
            to_track,
            &store,
            params.energy_phase.map(Into::into),
            &weights,
            params.use_master_tempo.unwrap_or(true),
            params
                .harmonic_style
                .unwrap_or(HarmonicMixingStyle::Balanced)
                .into(),
        )
        .map_err(|e| mcp_internal_error(format!("Failed to build track profiles: {e}")))?
    };
    let from_profile = evaluation.from;
    let to_profile = evaluation.to;
    let scores = evaluation.scores;

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

    ok_json(&result)
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

    let weights = {
        let store = server.cache_store_conn()?;
        resolve_transition_weights(params.priority.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };
    let master_tempo = params.use_master_tempo.unwrap_or(true);
    let limit = params.limit.unwrap_or(10).min(50) as usize;

    let from_track = {
        let conn = server.rekordbox_conn()?;
        db::get_track(&conn, &params.source_track_id)
            .map_err(db_error)?
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!("From track '{}' not found", params.source_track_id),
                    None,
                )
            })?
    };

    let pool_tracks = {
        let conn = server.rekordbox_conn()?;
        if let Some(ref ids) = params.candidate_track_ids {
            db::get_tracks_by_ids(&conn, ids).map_err(db_error)?
        } else if let Some(ref playlist_id) = params.playlist_id {
            db::get_playlist_tracks(&conn, playlist_id, None).map_err(db_error)?
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

    let ranked = {
        let store = server.cache_store_conn()?;
        crate::application::planning::rank_transition_candidates(
            from_track,
            pool_tracks,
            &store,
            params.energy_phase.map(Into::into),
            &weights,
            master_tempo,
            params
                .harmonic_style
                .unwrap_or(HarmonicMixingStyle::Balanced)
                .into(),
            params.target_bpm,
            limit,
        )
        .map_err(|e| mcp_internal_error(format!("Failed to build track profiles: {e}")))?
    };
    let from_profile = ranked.from;
    let skipped_profiles = 0u32;

    let candidates_json: Vec<serde_json::Value> = ranked
        .candidates
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
        "reference_bpm": round_to_3_decimals(ranked.reference_bpm),
        "master_tempo": master_tempo,
        "candidates": candidates_json,
        "total_pool_size": ranked.total_pool_size,
    });
    if skipped_profiles > 0 {
        result["skipped_profiles"] = serde_json::json!(skipped_profiles);
    }

    ok_json(&result)
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
    let weights = {
        let store = server.cache_store_conn()?;
        resolve_transition_weights(params.priority.as_ref(), &store)
            .map_err(|e| McpError::invalid_params(e, None))?
    };

    let tracks = {
        let conn = server.rekordbox_conn()?;
        db::get_tracks_by_ids(&conn, &deduped_ids).map_err(db_error)?
    };
    if tracks.is_empty() {
        return Err(McpError::invalid_params(
            "No valid tracks found for provided track_ids".to_string(),
            None,
        ));
    }

    let master_tempo = params.use_master_tempo.unwrap_or(true);
    let built = {
        let store = server.cache_store_conn()?;
        crate::application::planning::build_set_candidates(
            &store,
            crate::application::planning::BuildSetOptions {
                tracks,
                requested_target,
                energy_curve: params.energy_curve.as_ref().map(Into::into),
                opening_track_id: params.opening_track_id,
                beam_width: effective_beam_width,
                weights,
                master_tempo,
                harmonic_style: params
                    .harmonic_style
                    .unwrap_or(HarmonicMixingStyle::Balanced)
                    .into(),
                bpm_drift_pct: params.bpm_drift_pct.unwrap_or(6.0),
                bpm_range: params.bpm_range,
            },
        )
        .map_err(|error| match error {
            crate::application::planning::BuildSetError::Profile(error) => {
                mcp_internal_error(format!("Failed to build track profiles: {error}"))
            }
            crate::application::planning::BuildSetError::OpeningTrack(error) => {
                McpError::invalid_params(error, None)
            }
            crate::application::planning::BuildSetError::EnergyCurve(error) => {
                McpError::invalid_params(format!("Invalid energy_curve: {error}"), None)
            }
        })?
    };
    let effective_beam_width = built.beam_width;
    let profiles_by_id = built.profiles_by_id;
    let plans = built.plans;
    let actual_target = built.actual_target;
    let bpm_trajectory = built.bpm_trajectory;

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

    ok_json(&result)
}
