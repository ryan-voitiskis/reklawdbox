//! Deterministic greedy and beam-search sequencing policy.

use std::collections::{HashMap, HashSet};

use crate::domain::classification::taxonomy::GenreFamily;

use super::{
    CandidatePlan, CandidateTransition, EnergyPhase, HarmonicMixingStyle, PriorityWeights,
    ScoreAdjustment, ScoringContext, TrackProfile, TransitionScores, score_transition_profiles,
};

const BPM_DRIFT_PENALTY_FACTOR: f64 = 0.7;

pub(crate) fn select_start_track_ids(
    profiles_by_id: &HashMap<String, TrackProfile>,
    requested_candidates: usize,
    first_phase: EnergyPhase,
    forced_start: Option<&str>,
) -> Vec<String> {
    if let Some(track_id) = forced_start {
        return vec![track_id.to_string()];
    }

    let prefer_low_energy = matches!(first_phase, EnergyPhase::Warmup | EnergyPhase::Build);
    let mut profiles: Vec<&TrackProfile> = profiles_by_id.values().collect();
    profiles.sort_by(|left, right| {
        let energy_cmp = if prefer_low_energy {
            left.energy
                .partial_cmp(&right.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            right
                .energy
                .partial_cmp(&left.energy)
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        energy_cmp.then_with(|| left.track.id.cmp(&right.track.id))
    });

    let wanted = requested_candidates.max(1);
    let mut start_track_ids: Vec<String> = profiles
        .into_iter()
        .take(wanted)
        .map(|profile| profile.track.id.clone())
        .collect();
    if start_track_ids.is_empty() {
        start_track_ids.extend(profiles_by_id.keys().take(1).cloned());
    }
    start_track_ids
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_candidate_plan(
    profiles_by_id: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    weights: &PriorityWeights,
    variation_index: usize,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    bpm_drift_pct: f64,
    target_bpms: Option<&[f64]>,
) -> CandidatePlan {
    let mut ordered_ids = vec![start_track_id.to_string()];
    let mut transitions = Vec::new();
    let mut remaining: HashSet<String> = profiles_by_id.keys().cloned().collect();
    remaining.remove(start_track_id);

    let mut genre_run_length: u32 = 0;
    let start_bpm = profiles_by_id.get(start_track_id).map_or(0.0, |p| p.bpm);

    while ordered_ids.len() < target_tracks && !remaining.is_empty() {
        let Some(from_track_id) = ordered_ids.last() else {
            break;
        };
        let Some(from_profile) = profiles_by_id.get(from_track_id) else {
            break;
        };

        let to_phase = phases.get(ordered_ids.len()).copied();
        let from_phase = ordered_ids
            .len()
            .checked_sub(1)
            .and_then(|idx| phases.get(idx).copied());
        let scoring_context = ScoringContext { genre_run_length };
        let step = ordered_ids.len();
        let play_bpms = target_bpms.and_then(|bpms| {
            let from_bpm = bpms.get(step - 1).copied()?;
            let to_bpm = bpms.get(step).copied()?;
            Some((from_bpm, to_bpm))
        });
        let mut scored_next: Vec<(String, TransitionScores)> = remaining
            .iter()
            .filter_map(|candidate_id| {
                profiles_by_id.get(candidate_id).map(|to_profile| {
                    (
                        candidate_id.clone(),
                        score_transition_profiles(
                            from_profile,
                            to_profile,
                            from_phase,
                            to_phase,
                            weights,
                            master_tempo,
                            harmonic_style,
                            &scoring_context,
                            play_bpms,
                        ),
                    )
                })
            })
            .collect();

        if start_bpm > 0.0 && target_tracks > 1 {
            let position = ordered_ids.len();
            let max_position = (target_tracks - 1) as f64;
            let budget_pct = bpm_drift_pct * (position as f64 / max_position);
            let budget_bpm = start_bpm * budget_pct / 100.0;
            for (candidate_id, scores) in &mut scored_next {
                if let Some(candidate_profile) = profiles_by_id.get(candidate_id.as_str()) {
                    let drift = (candidate_profile.bpm - start_bpm).abs();
                    if drift > budget_bpm {
                        let composite_without = scores.composite;
                        scores.composite *= BPM_DRIFT_PENALTY_FACTOR;
                        scores.adjustments.push(ScoreAdjustment {
                            kind: "bpm_drift",
                            delta: scores.composite - composite_without,
                            composite_without,
                            reason: format!(
                                "BPM drift {drift:.1} exceeds budget {budget_bpm:.1} at position {position} — 0.7x penalty",
                            ),
                        });
                    }
                }
            }
        }

        scored_next.sort_by(|left, right| {
            right
                .1
                .composite
                .partial_cmp(&left.1.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });

        if scored_next.is_empty() {
            break;
        }

        let pick_rank = transition_pick_rank(variation_index, ordered_ids.len(), scored_next.len());
        let (next_track_id, transition_scores) = scored_next[pick_rank].clone();

        if let Some(next_profile) = profiles_by_id.get(&next_track_id) {
            if next_profile.genre_family == from_profile.genre_family
                && from_profile.genre_family != GenreFamily::Other
            {
                genre_run_length += 1;
            } else {
                genre_run_length = 0;
            }
        }

        transitions.push(CandidateTransition {
            from_index: ordered_ids.len() - 1,
            to_index: ordered_ids.len(),
            scores: transition_scores,
        });
        ordered_ids.push(next_track_id.clone());
        remaining.remove(&next_track_id);
    }

    CandidatePlan {
        ordered_ids,
        transitions,
    }
}

fn transition_pick_rank(
    variation_index: usize,
    current_length: usize,
    available_options: usize,
) -> usize {
    if available_options <= 1 {
        return 0;
    }
    let preferred_rank = if current_length == 1 {
        variation_index
    } else if variation_index > 0 && current_length.is_multiple_of(4) {
        variation_index.min(1)
    } else {
        0
    };
    preferred_rank.min(available_options - 1)
}

// ---------------------------------------------------------------------------
// Beam search
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct BeamState {
    ordered_ids: Vec<String>,
    remaining: HashSet<String>,
    genre_run_length: u32,
    cumulative_score: f64,
    transitions: Vec<CandidateTransition>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_candidate_plan_beam(
    profiles_by_id: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    weights: &PriorityWeights,
    beam_width: usize,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    bpm_drift_pct: f64,
    target_bpms: Option<&[f64]>,
) -> Vec<CandidatePlan> {
    let mut remaining_init: HashSet<String> = profiles_by_id.keys().cloned().collect();
    remaining_init.remove(start_track_id);

    let start_bpm = profiles_by_id.get(start_track_id).map_or(0.0, |p| p.bpm);

    let initial = BeamState {
        ordered_ids: vec![start_track_id.to_string()],
        remaining: remaining_init,
        genre_run_length: 0,
        cumulative_score: 0.0,
        transitions: Vec::new(),
    };

    let mut beams = vec![initial];

    for step in 1..target_tracks {
        let mut expansions: Vec<BeamState> = Vec::new();

        for beam in &beams {
            if beam.remaining.is_empty() {
                expansions.push(beam.clone());
                continue;
            }

            let from_id = beam
                .ordered_ids
                .last()
                .expect("ordered_ids always has at least the start track");
            let Some(from_profile) = profiles_by_id.get(from_id) else {
                expansions.push(beam.clone());
                continue;
            };

            let to_phase = phases.get(step).copied();
            let from_phase = step.checked_sub(1).and_then(|idx| phases.get(idx).copied());
            let scoring_context = ScoringContext {
                genre_run_length: beam.genre_run_length,
            };

            let play_bpms = target_bpms.and_then(|bpms| {
                let from_bpm = bpms.get(step - 1).copied()?;
                let to_bpm = bpms.get(step).copied()?;
                Some((from_bpm, to_bpm))
            });

            for candidate_id in &beam.remaining {
                let Some(to_profile) = profiles_by_id.get(candidate_id) else {
                    continue;
                };

                let mut scores = score_transition_profiles(
                    from_profile,
                    to_profile,
                    from_phase,
                    to_phase,
                    weights,
                    master_tempo,
                    harmonic_style,
                    &scoring_context,
                    play_bpms,
                );

                if start_bpm > 0.0 && target_tracks > 1 {
                    let max_position = (target_tracks - 1) as f64;
                    let budget_pct = bpm_drift_pct * (step as f64 / max_position);
                    let budget_bpm = start_bpm * budget_pct / 100.0;
                    let drift = (to_profile.bpm - start_bpm).abs();
                    if drift > budget_bpm {
                        let composite_without = scores.composite;
                        scores.composite *= BPM_DRIFT_PENALTY_FACTOR;
                        scores.adjustments.push(ScoreAdjustment {
                            kind: "bpm_drift",
                            delta: scores.composite - composite_without,
                            composite_without,
                            reason: format!(
                                "BPM drift {drift:.1} exceeds budget {budget_bpm:.1} at step {step} — 0.7x penalty",
                            ),
                        });
                    }
                }

                let new_cumulative = beam.cumulative_score + scores.composite;

                let new_genre_run = if to_profile.genre_family == from_profile.genre_family
                    && from_profile.genre_family != GenreFamily::Other
                {
                    beam.genre_run_length + 1
                } else {
                    0
                };

                let mut new_ordered = beam.ordered_ids.clone();
                new_ordered.push(candidate_id.clone());
                let mut new_remaining = beam.remaining.clone();
                new_remaining.remove(candidate_id);
                let mut new_transitions = beam.transitions.clone();
                new_transitions.push(CandidateTransition {
                    from_index: step - 1,
                    to_index: step,
                    scores,
                });

                expansions.push(BeamState {
                    ordered_ids: new_ordered,
                    remaining: new_remaining,
                    genre_run_length: new_genre_run,
                    cumulative_score: new_cumulative,
                    transitions: new_transitions,
                });
            }
        }

        expansions.sort_by(|a, b| {
            let a_mean = if a.transitions.is_empty() {
                0.0
            } else {
                a.cumulative_score / a.transitions.len() as f64
            };
            let b_mean = if b.transitions.is_empty() {
                0.0
            } else {
                b.cumulative_score / b.transitions.len() as f64
            };
            b_mean
                .partial_cmp(&a_mean)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.ordered_ids.cmp(&b.ordered_ids))
        });

        expansions.truncate(beam_width);
        beams = expansions;
    }

    let mut seen_plans: HashSet<Vec<String>> = HashSet::new();
    beams
        .into_iter()
        .filter(|beam| seen_plans.insert(beam.ordered_ids.clone()))
        .map(|beam| CandidatePlan {
            ordered_ids: beam.ordered_ids,
            transitions: beam.transitions,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_sequence_empty_pool_preserves_single_start_plan() {
        let profiles = HashMap::new();
        let plans = build_candidate_plan_beam(
            &profiles,
            "missing",
            4,
            &[EnergyPhase::Peak; 4],
            &super::super::priority_weights(super::super::SequencingPriority::Balanced),
            3,
            true,
            None,
            6.0,
            None,
        );
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].ordered_ids, vec!["missing"]);
        assert!(plans[0].transitions.is_empty());
    }
}
