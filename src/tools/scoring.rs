use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use super::*;
use crate::genre;
use crate::store;

// Energy curve preset phase boundaries
const WARMUP_PHASE_END: f64 = 0.15;
const BUILD_PHASE_END: f64 = 0.45;
const PEAK_PHASE_END: f64 = 0.75;
const PEAKONLY_BUILD_END: f64 = 0.10;
const PEAKONLY_RELEASE_END: f64 = 0.85;

// Scoring factors
const BPM_DRIFT_PENALTY_FACTOR: f64 = 0.7;
const HARMONIC_PENALTY_FACTOR: f64 = 0.5;

// Brightness axis thresholds (Hz)
const BRIGHTNESS_SIMILAR_HZ: f64 = 300.0;
const BRIGHTNESS_SHIFT_HZ: f64 = 800.0;
const BRIGHTNESS_JUMP_HZ: f64 = 1500.0;

// Rhythm regularity thresholds
const RHYTHM_MATCHED_DELTA: f64 = 0.1;
const RHYTHM_MANAGEABLE_DELTA: f64 = 0.25;
const RHYTHM_CHALLENGING_DELTA: f64 = 0.5;

#[derive(Debug, Clone)]
pub(super) struct TrackProfile {
    pub(super) track: crate::types::Track,
    pub(super) camelot_key: Option<CamelotKey>,
    pub(super) key_display: String,
    pub(super) bpm: f64,
    pub(super) energy: f64,
    pub(super) brightness: Option<f64>,
    pub(super) rhythm_regularity: Option<f64>,
    pub(super) loudness_range: Option<f64>,
    pub(super) canonical_genre: Option<String>,
    pub(super) genre_family: GenreFamily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CamelotKey {
    number: u8,
    letter: char,
}

pub(super) use crate::genre::GenreFamily;

#[derive(Debug, Clone)]
pub(super) struct AxisScore {
    pub(super) value: f64,
    pub(super) label: String,
}

#[derive(Debug, Clone)]
pub(super) struct TransitionScores {
    pub(super) key: AxisScore,
    pub(super) bpm: AxisScore,
    pub(super) energy: AxisScore,
    pub(super) genre: AxisScore,
    pub(super) brightness: AxisScore,
    pub(super) rhythm: AxisScore,
    pub(super) composite: f64,
    pub(super) effective_to_key: Option<String>,
    pub(super) pitch_shift_semitones: i32,
    pub(super) key_relation: String,
    pub(super) bpm_adjustment_pct: f64,
}

impl TransitionScores {
    pub(super) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "key": { "value": round_to_3_decimals(self.key.value), "label": self.key.label },
            "bpm": { "value": round_to_3_decimals(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_to_3_decimals(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_to_3_decimals(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_to_3_decimals(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_to_3_decimals(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_to_3_decimals(self.composite),
        })
    }
}

#[derive(Debug, Clone, Default)]
pub(super) struct ScoringContext {
    pub(super) genre_run_length: u32,
}

#[derive(Debug, Clone)]
pub(super) struct CandidateTransition {
    pub(super) from_index: usize,
    pub(super) to_index: usize,
    pub(super) scores: TransitionScores,
}

#[derive(Debug, Clone)]
pub(super) struct CandidatePlan {
    pub(super) ordered_ids: Vec<String>,
    pub(super) transitions: Vec<CandidateTransition>,
}

pub(super) fn resolve_energy_curve(
    energy_curve: Option<&EnergyCurveInput>,
    target_tracks: usize,
) -> Result<Vec<EnergyPhase>, String> {
    if target_tracks == 0 {
        return Err("target_tracks must be at least 1".to_string());
    }

    match energy_curve {
        Some(EnergyCurveInput::Custom(phases)) => {
            if phases.len() != target_tracks {
                return Err(format!(
                    "custom phase array length ({}) must match target_tracks ({target_tracks})",
                    phases.len()
                ));
            }
            Ok(phases.clone())
        }
        Some(EnergyCurveInput::Preset(preset)) => Ok((0..target_tracks)
            .map(|position| preset_energy_phase(*preset, position, target_tracks))
            .collect()),
        None => Ok((0..target_tracks)
            .map(|position| {
                preset_energy_phase(
                    EnergyCurvePreset::WarmupBuildPeakRelease,
                    position,
                    target_tracks,
                )
            })
            .collect()),
    }
}

fn preset_energy_phase(preset: EnergyCurvePreset, position: usize, total: usize) -> EnergyPhase {
    let fraction = if total == 0 {
        0.0
    } else {
        position as f64 / total as f64
    };
    match preset {
        EnergyCurvePreset::WarmupBuildPeakRelease => {
            if fraction < WARMUP_PHASE_END {
                EnergyPhase::Warmup
            } else if fraction < BUILD_PHASE_END {
                EnergyPhase::Build
            } else if fraction < PEAK_PHASE_END {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
        EnergyCurvePreset::FlatEnergy => EnergyPhase::Peak,
        EnergyCurvePreset::PeakOnly => {
            if fraction < PEAKONLY_BUILD_END {
                EnergyPhase::Build
            } else if fraction < PEAKONLY_RELEASE_END {
                EnergyPhase::Peak
            } else {
                EnergyPhase::Release
            }
        }
    }
}

pub(super) fn select_start_track_ids(
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
pub(super) fn build_candidate_plan(
    profiles_by_id: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    priority: SequencingPriority,
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

    // Track genre run length for stickiness scoring
    let mut genre_run_length: u32 = 0;
    // Track start BPM for trajectory drift penalty
    let start_bpm = profiles_by_id
        .get(start_track_id)
        .map(|p| p.bpm)
        .unwrap_or(0.0);

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
                            priority,
                            master_tempo,
                            harmonic_style,
                            &scoring_context,
                            play_bpms,
                        ),
                    )
                })
            })
            .collect();

        // BPM trajectory coherence penalty (percentage-based)
        if start_bpm > 0.0 && target_tracks > 1 {
            let position = ordered_ids.len(); // tracks already placed (progress through set)
            let max_position = (target_tracks - 1) as f64;
            let budget_pct = bpm_drift_pct * (position as f64 / max_position);
            let budget_bpm = start_bpm * budget_pct / 100.0;
            for (candidate_id, scores) in &mut scored_next {
                if let Some(candidate_profile) = profiles_by_id.get(candidate_id.as_str()) {
                    let drift = (candidate_profile.bpm - start_bpm).abs();
                    if drift > budget_bpm {
                        scores.composite *= BPM_DRIFT_PENALTY_FACTOR;
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

        // Update genre run length
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
pub(super) fn build_candidate_plan_beam(
    profiles_by_id: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    priority: SequencingPriority,
    beam_width: usize,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    bpm_drift_pct: f64,
    target_bpms: Option<&[f64]>,
) -> Vec<CandidatePlan> {
    let mut remaining_init: HashSet<String> = profiles_by_id.keys().cloned().collect();
    remaining_init.remove(start_track_id);

    let start_bpm = profiles_by_id
        .get(start_track_id)
        .map(|p| p.bpm)
        .unwrap_or(0.0);

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
                // No more tracks to add; carry forward as-is
                expansions.push(beam.clone());
                continue;
            }

            let from_id = beam.ordered_ids.last().unwrap();
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
                    priority,
                    master_tempo,
                    harmonic_style,
                    &scoring_context,
                    play_bpms,
                );

                // BPM trajectory coherence penalty (same as greedy)
                if start_bpm > 0.0 && target_tracks > 1 {
                    let max_position = (target_tracks - 1) as f64;
                    let budget_pct = bpm_drift_pct * (step as f64 / max_position);
                    let budget_bpm = start_bpm * budget_pct / 100.0;
                    let drift = (to_profile.bpm - start_bpm).abs();
                    if drift > budget_bpm {
                        scores.composite *= BPM_DRIFT_PENALTY_FACTOR;
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

        // Sort by mean composite (cumulative / transition_count) descending,
        // break ties by ordered_ids for determinism
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

        // Keep top K
        expansions.truncate(beam_width);
        beams = expansions;
    }

    // Deduplicate identical plans (by ordered_ids)
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

/// Compute a per-position target BPM trajectory based on energy phases.
///
/// - **Warmup** → `start_bpm`
/// - **Build** → linear ramp from `start_bpm` to `end_bpm`
/// - **Peak** → `end_bpm`
/// - **Release** → linear ramp from `end_bpm` back toward `start_bpm`
pub(super) fn compute_bpm_trajectory(
    phases: &[EnergyPhase],
    start_bpm: f64,
    end_bpm: f64,
) -> Vec<f64> {
    if phases.is_empty() {
        return Vec::new();
    }

    // Find span indices for build and release phases
    let build_start = phases.iter().position(|p| *p == EnergyPhase::Build);
    let build_end = phases.iter().rposition(|p| *p == EnergyPhase::Build);
    let release_start = phases.iter().position(|p| *p == EnergyPhase::Release);
    let release_end = phases.iter().rposition(|p| *p == EnergyPhase::Release);

    phases
        .iter()
        .enumerate()
        .map(|(i, phase)| match phase {
            EnergyPhase::Warmup => start_bpm,
            EnergyPhase::Build => {
                let (build_start_idx, build_end_idx) = (build_start.unwrap(), build_end.unwrap());
                if build_start_idx == build_end_idx {
                    (start_bpm + end_bpm) / 2.0
                } else {
                    let progress = (i - build_start_idx) as f64 / (build_end_idx - build_start_idx) as f64;
                    start_bpm + (end_bpm - start_bpm) * progress
                }
            }
            EnergyPhase::Peak => end_bpm,
            EnergyPhase::Release => {
                let (release_start_idx, release_end_idx) = (release_start.unwrap(), release_end.unwrap());
                if release_start_idx == release_end_idx {
                    (end_bpm + start_bpm) / 2.0
                } else {
                    let progress = (i - release_start_idx) as f64 / (release_end_idx - release_start_idx) as f64;
                    end_bpm + (start_bpm - end_bpm) * progress
                }
            }
        })
        .collect()
}

pub(super) fn build_track_profile(
    track: crate::types::Track,
    store_conn: &Connection,
) -> Result<TrackProfile, String> {
    let cache_key = resolve_file_path(&track.file_path).unwrap_or_else(|_| track.file_path.clone());
    let stratum_json = store::get_audio_analysis(store_conn, &cache_key, crate::audio::ANALYZER_STRATUM)
        .map_err(|e| format!("stratum cache read error: {e}"))?
        .and_then(|cached| serde_json::from_str::<serde_json::Value>(&cached.features_json).ok());
    let essentia_data = store::get_audio_analysis(store_conn, &cache_key, crate::audio::ANALYZER_ESSENTIA)
        .map_err(|e| format!("essentia cache read error: {e}"))?
        .and_then(|cached| serde_json::from_str::<crate::audio::EssentiaOutput>(&cached.features_json).ok());

    let bpm = stratum_json
        .as_ref()
        .and_then(|v| v.get("bpm"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(track.bpm)
        .max(0.0);

    let camelot_key = stratum_json
        .as_ref()
        .and_then(|v| v.get("key_camelot").and_then(serde_json::Value::as_str))
        .and_then(parse_camelot_key)
        .or_else(|| key_to_camelot(&track.key));

    let key_display = camelot_key
        .map(format_camelot)
        .unwrap_or_else(|| match track.key.trim() {
            "" => "Unknown".to_string(),
            _ => track.key.clone(),
        });

    let energy = compute_track_energy(essentia_data.as_ref(), bpm);
    let brightness = essentia_data.as_ref().and_then(|e| e.spectral_centroid_mean);
    let rhythm_regularity = essentia_data.as_ref().and_then(|e| e.rhythm_regularity);
    let loudness_range = essentia_data.as_ref().and_then(|e| e.loudness_range);
    let canonical_genre = canonicalize_genre(&track.genre);
    let genre_family = canonical_genre
        .as_deref()
        .map(genre_family_for)
        .unwrap_or(GenreFamily::Other);

    Ok(TrackProfile {
        track,
        camelot_key,
        key_display,
        bpm,
        energy,
        brightness,
        rhythm_regularity,
        loudness_range,
        canonical_genre,
        genre_family,
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn score_transition_profiles(
    from: &TrackProfile,
    to: &TrackProfile,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    priority: SequencingPriority,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    ctx: &ScoringContext,
    play_bpms: Option<(f64, f64)>,
) -> TransitionScores {
    // When play_bpms is set, both tracks are pitched to target BPMs.
    // Compute effective keys based on the pitch shift from native BPM to play BPM.
    // When play_bpms is None, fall back to the existing master_tempo logic.
    let (effective_to_key, pitch_shift_semitones, scoring_from_key, scoring_to_key, bpm) =
        if let Some((from_play_bpm, to_play_bpm)) = play_bpms {
            // Compute effective keys for both tracks based on play BPMs
            let from_shift = if from.bpm > 0.0 && from_play_bpm > 0.0 {
                (12.0 * (from_play_bpm / from.bpm).log2()).round() as i32
            } else {
                0
            };
            let to_shift = if to.bpm > 0.0 && to_play_bpm > 0.0 {
                (12.0 * (to_play_bpm / to.bpm).log2()).round() as i32
            } else {
                0
            };

            let effective_from_key = if !master_tempo && from_shift != 0 {
                from.camelot_key.map(|k| transpose_camelot_key(k, from_shift))
            } else {
                from.camelot_key
            };
            let effective_to_key = if !master_tempo && to_shift != 0 {
                to.camelot_key.map(|k| transpose_camelot_key(k, to_shift))
            } else {
                to.camelot_key
            };

            let effective_to_key_display = if !master_tempo && to_shift != 0 {
                effective_to_key.map(format_camelot)
            } else {
                None
            };

            // BPM axis scores how close the candidate's native BPM is to its target
            let bpm_score = score_bpm_axis(to_play_bpm, to.bpm);

            (effective_to_key_display, to_shift, effective_from_key, effective_to_key, bpm_score)
        } else {
            // Original master_tempo logic
            let (eff_to_key, shift) = if !master_tempo && from.bpm > 0.0 && to.bpm > 0.0 {
                let shift = (12.0 * (from.bpm / to.bpm).log2()).round() as i32;
                if shift != 0 {
                    let transposed = to.camelot_key.map(|k| transpose_camelot_key(k, shift));
                    (transposed.map(format_camelot), shift)
                } else {
                    (None, 0)
                }
            } else {
                (None, 0)
            };

            let scoring_to = if let Some(ref ek) = eff_to_key {
                parse_camelot_key(ek)
            } else {
                to.camelot_key
            };

            let bpm_score = score_bpm_axis(from.bpm, to.bpm);

            (eff_to_key, shift, from.camelot_key, scoring_to, bpm_score)
        };

    let key = score_key_axis(scoring_from_key, scoring_to_key);
    let energy = score_energy_axis(
        from.energy,
        to.energy,
        from_phase,
        to_phase,
        to.loudness_range,
    );
    let genre = score_genre_axis(
        from.canonical_genre.as_deref(),
        to.canonical_genre.as_deref(),
        from.genre_family,
        to.genre_family,
        ctx.genre_run_length,
    );
    let brightness = score_brightness_axis(from.brightness, to.brightness);
    let rhythm = score_rhythm_axis(from.rhythm_regularity, to.rhythm_regularity);
    let brightness_available = from.brightness.is_some() && to.brightness.is_some();
    let rhythm_available = from.rhythm_regularity.is_some() && to.rhythm_regularity.is_some();
    let mut composite = composite_score(
        key.value,
        bpm.value,
        energy.value,
        genre.value,
        if brightness_available {
            Some(brightness.value)
        } else {
            None
        },
        if rhythm_available {
            Some(rhythm.value)
        } else {
            None
        },
        priority,
    );

    // Harmonic style modulation gate: penalize transitions where key score
    // falls below the minimum threshold for the current phase × style.
    if let Some(style) = harmonic_style {
        let min_key = harmonic_style_min_key(style, to_phase);
        if key.value < min_key {
            composite *= HARMONIC_PENALTY_FACTOR;
        }
    }

    let key_relation = key.label.clone();
    let bpm_adjustment_pct = if let Some((_, to_play_bpm)) = play_bpms {
        if to.bpm > 0.0 {
            (to_play_bpm - to.bpm).abs() / to.bpm * 100.0
        } else {
            0.0
        }
    } else if to.bpm > 0.0 {
        (from.bpm - to.bpm).abs() / to.bpm * 100.0
    } else {
        0.0
    };

    TransitionScores {
        key,
        bpm,
        energy,
        genre,
        brightness,
        rhythm,
        composite,
        effective_to_key,
        pitch_shift_semitones,
        key_relation,
        bpm_adjustment_pct,
    }
}

pub(super) fn round_to_3_decimals(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(super) fn score_key_axis(from: Option<CamelotKey>, to: Option<CamelotKey>) -> AxisScore {
    let Some(from) = from else {
        return AxisScore {
            value: 0.1,
            label: "Clash (missing key)".to_string(),
        };
    };
    let Some(to) = to else {
        return AxisScore {
            value: 0.1,
            label: "Clash (missing key)".to_string(),
        };
    };

    if from.number == to.number && from.letter == to.letter {
        return AxisScore {
            value: 1.0,
            label: "Perfect".to_string(),
        };
    }
    if from.number == to.number && from.letter != to.letter {
        return AxisScore {
            value: 0.8,
            label: "Mood shift (A\u{2194}B)".to_string(),
        };
    }

    let clockwise = ((to.number as i16 - from.number as i16 + 12) % 12) as u8;
    if from.letter == to.letter && clockwise == 1 {
        AxisScore {
            value: 0.9,
            label: "Camelot adjacent (+1)".to_string(),
        }
    } else if from.letter == to.letter && clockwise == 11 {
        AxisScore {
            value: 0.9,
            label: "Camelot adjacent (-1)".to_string(),
        }
    } else if from.letter == to.letter && (clockwise == 2 || clockwise == 10) {
        AxisScore {
            value: 0.45,
            label: "Extended (+/-2)".to_string(),
        }
    } else if from.letter != to.letter && (clockwise == 1 || clockwise == 11) {
        AxisScore {
            value: 0.55,
            label: "Energy diagonal (+/-1 cross)".to_string(),
        }
    } else {
        AxisScore {
            value: 0.1,
            label: "Clash".to_string(),
        }
    }
}

pub(super) fn score_bpm_axis(from_bpm: f64, to_bpm: f64) -> AxisScore {
    if from_bpm <= 0.0 {
        return AxisScore {
            value: 0.5,
            label: "Unknown BPM".to_string(),
        };
    }
    let delta = (from_bpm - to_bpm).abs();
    let pct = delta / from_bpm * 100.0;
    if pct <= 1.5 {
        AxisScore {
            value: 1.0,
            label: format!("Seamless ({:.1}%, {:.1} BPM)", pct, delta),
        }
    } else if pct <= 3.0 {
        AxisScore {
            value: 0.85,
            label: format!("Comfortable ({:.1}%, {:.1} BPM)", pct, delta),
        }
    } else if pct <= 5.0 {
        AxisScore {
            value: 0.6,
            label: format!("Noticeable ({:.1}%, {:.1} BPM)", pct, delta),
        }
    } else if pct <= 8.0 {
        AxisScore {
            value: 0.3,
            label: format!("Creative transition needed ({:.1}%, {:.1} BPM)", pct, delta),
        }
    } else {
        AxisScore {
            value: 0.1,
            label: format!("Jarring ({:.1}%, {:.1} BPM)", pct, delta),
        }
    }
}

pub(super) fn score_energy_axis(
    from_energy: f64,
    to_energy: f64,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    to_loudness_range: Option<f64>,
) -> AxisScore {
    let delta = to_energy - from_energy;
    let mut axis = match to_phase {
        Some(EnergyPhase::Warmup) => {
            let phase_requirement_met = (-0.03..=0.12).contains(&delta);
            AxisScore {
                value: if phase_requirement_met { 1.0 } else { 0.5 },
                label: if phase_requirement_met {
                    "Stable/slight rise (warmup phase)".to_string()
                } else {
                    "Too abrupt for warmup".to_string()
                },
            }
        }
        Some(EnergyPhase::Build) => {
            let phase_requirement_met = delta >= 0.03;
            AxisScore {
                value: if phase_requirement_met { 1.0 } else { 0.3 },
                label: if phase_requirement_met {
                    "Rising (build phase)".to_string()
                } else {
                    "Not rising (build phase)".to_string()
                },
            }
        }
        Some(EnergyPhase::Peak) => {
            let phase_requirement_met = to_energy >= 0.65 && delta.abs() <= 0.10;
            AxisScore {
                value: if phase_requirement_met { 1.0 } else { 0.5 },
                label: if phase_requirement_met {
                    "High and stable (peak phase)".to_string()
                } else {
                    "Not high/stable (peak phase)".to_string()
                },
            }
        }
        Some(EnergyPhase::Release) => {
            let phase_requirement_met = delta <= -0.03;
            AxisScore {
                value: if phase_requirement_met { 1.0 } else { 0.3 },
                label: if phase_requirement_met {
                    "Dropping (release phase)".to_string()
                } else {
                    "Not dropping (release phase)".to_string()
                },
            }
        }
        None => AxisScore {
            value: 1.0,
            label: "No phase preference".to_string(),
        },
    };

    let is_phase_boundary = matches!(
        (from_phase, to_phase),
        (Some(previous), Some(current)) if previous != current
    );
    match (to_phase, to_loudness_range) {
        (Some(_), Some(loudness_range)) if is_phase_boundary && loudness_range > 8.0 => {
            axis.value = (axis.value + 0.1).clamp(0.0, 1.0);
            axis.label.push_str(" + dynamic boundary boost");
        }
        (Some(EnergyPhase::Peak), Some(loudness_range)) if !is_phase_boundary && loudness_range < 4.0 => {
            axis.value = (axis.value + 0.05).clamp(0.0, 1.0);
            axis.label.push_str(" + sustained-peak consistency boost");
        }
        _ => {}
    }
    axis
}

pub(super) fn score_genre_axis(
    from_genre: Option<&str>,
    to_genre: Option<&str>,
    from_family: GenreFamily,
    to_family: GenreFamily,
    genre_run_length: u32,
) -> AxisScore {
    let Some(from_genre) = from_genre else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };
    let Some(to_genre) = to_genre else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };

    let genre_compatible = (from_genre.eq_ignore_ascii_case(to_genre))
        || (from_family == to_family && from_family != GenreFamily::Other);

    let mut axis = if from_genre.eq_ignore_ascii_case(to_genre) {
        AxisScore {
            value: 1.0,
            label: "Same genre".to_string(),
        }
    } else if from_family == to_family && from_family != GenreFamily::Other {
        AxisScore {
            value: 0.7,
            label: "Same family".to_string(),
        }
    } else {
        AxisScore {
            value: 0.3,
            label: "Different families".to_string(),
        }
    };

    // Genre stickiness: bonus for staying in the same family, penalty for early switch
    if genre_compatible && from_family != GenreFamily::Other && genre_run_length > 0 && genre_run_length < 5 {
        axis.value = (axis.value + 0.1).min(1.0);
        axis.label.push_str(" + streak bonus");
    } else if !genre_compatible && genre_run_length > 0 && genre_run_length < 2 {
        axis.value = (axis.value - 0.1).max(0.0);
        axis.label.push_str(" + early switch penalty");
    }

    axis
}

fn score_brightness_axis(from_centroid: Option<f64>, to_centroid: Option<f64>) -> AxisScore {
    let Some(from_centroid) = from_centroid else {
        return AxisScore {
            value: 0.5,
            label: "Unknown brightness".to_string(),
        };
    };
    let Some(to_centroid) = to_centroid else {
        return AxisScore {
            value: 0.5,
            label: "Unknown brightness".to_string(),
        };
    };

    let delta = (to_centroid - from_centroid).abs();
    if delta < BRIGHTNESS_SIMILAR_HZ {
        AxisScore {
            value: 1.0,
            label: format!("Similar timbre (delta {:.0} Hz)", delta),
        }
    } else if delta < BRIGHTNESS_SHIFT_HZ {
        AxisScore {
            value: 0.7,
            label: format!("Noticeable brightness shift (delta {:.0} Hz)", delta),
        }
    } else if delta < BRIGHTNESS_JUMP_HZ {
        AxisScore {
            value: 0.4,
            label: format!("Large timbral jump (delta {:.0} Hz)", delta),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Jarring brightness jump (delta {:.0} Hz)", delta),
        }
    }
}

fn score_rhythm_axis(from_regularity: Option<f64>, to_regularity: Option<f64>) -> AxisScore {
    let Some(from_regularity) = from_regularity else {
        return AxisScore {
            value: 0.5,
            label: "Unknown groove".to_string(),
        };
    };
    let Some(to_regularity) = to_regularity else {
        return AxisScore {
            value: 0.5,
            label: "Unknown groove".to_string(),
        };
    };

    let delta = (to_regularity - from_regularity).abs();
    if delta < RHYTHM_MATCHED_DELTA {
        AxisScore {
            value: 1.0,
            label: format!("Matching groove (delta {:.2})", delta),
        }
    } else if delta < RHYTHM_MANAGEABLE_DELTA {
        AxisScore {
            value: 0.7,
            label: format!("Manageable groove shift (delta {:.2})", delta),
        }
    } else if delta < RHYTHM_CHALLENGING_DELTA {
        AxisScore {
            value: 0.4,
            label: format!("Challenging groove shift (delta {:.2})", delta),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Groove clash (delta {:.2})", delta),
        }
    }
}

fn harmonic_style_min_key(style: HarmonicMixingStyle, phase: Option<EnergyPhase>) -> f64 {
    match style {
        HarmonicMixingStyle::Conservative => 0.8,
        HarmonicMixingStyle::Balanced => 0.45,
        HarmonicMixingStyle::Adventurous => match phase {
            Some(EnergyPhase::Warmup) | Some(EnergyPhase::Release) => 0.45,
            Some(EnergyPhase::Build) | Some(EnergyPhase::Peak) | None => 0.1,
        },
    }
}

pub(super) struct PriorityWeights {
    pub key: f64,
    pub bpm: f64,
    pub energy: f64,
    pub genre: f64,
    pub brightness: f64,
    pub rhythm: f64,
}

pub(super) fn priority_weights(priority: SequencingPriority) -> PriorityWeights {
    match priority {
        SequencingPriority::Balanced => PriorityWeights { key: 0.30, bpm: 0.20, energy: 0.18, genre: 0.17, brightness: 0.08, rhythm: 0.07 },
        SequencingPriority::Harmonic => PriorityWeights { key: 0.48, bpm: 0.18, energy: 0.12, genre: 0.08, brightness: 0.08, rhythm: 0.06 },
        SequencingPriority::Energy => PriorityWeights { key: 0.12, bpm: 0.18, energy: 0.42, genre: 0.12, brightness: 0.08, rhythm: 0.08 },
        SequencingPriority::Genre => PriorityWeights { key: 0.18, bpm: 0.18, energy: 0.12, genre: 0.38, brightness: 0.08, rhythm: 0.06 },
    }
}

pub(super) fn composite_score(
    key_score: f64,
    bpm_score: f64,
    energy_score: f64,
    genre_score: f64,
    brightness_score: Option<f64>,
    rhythm_score: Option<f64>,
    priority: SequencingPriority,
) -> f64 {
    let weights = priority_weights(priority);
    let mut weighted_sum = (weights.key * key_score)
        + (weights.bpm * bpm_score)
        + (weights.energy * energy_score)
        + (weights.genre * genre_score);
    let mut total_weight = weights.key + weights.bpm + weights.energy + weights.genre;

    if let Some(brightness) = brightness_score {
        weighted_sum += weights.brightness * brightness;
        total_weight += weights.brightness;
    }
    if let Some(rhythm) = rhythm_score {
        weighted_sum += weights.rhythm * rhythm;
        total_weight += weights.rhythm;
    }

    if total_weight <= f64::EPSILON {
        0.0
    } else {
        weighted_sum / total_weight
    }
}

// BPM proxy normalization (typical club tempo range)
const BPM_PROXY_FLOOR: f64 = 95.0;
const BPM_PROXY_RANGE: f64 = 50.0; // 145 - 95

// Essentia descriptor normalization bounds
const DANCEABILITY_MAX: f64 = 3.0;
const LOUDNESS_FLOOR_LUFS: f64 = -30.0;
const LOUDNESS_RANGE_LUFS: f64 = 30.0;
const ONSET_RATE_MAX: f64 = 10.0;

// Composite energy weights
const ENERGY_W_DANCE: f64 = 0.4;
const ENERGY_W_LOUDNESS: f64 = 0.3;
const ENERGY_W_ONSET: f64 = 0.3;

pub(super) fn compute_track_energy(essentia: Option<&crate::audio::EssentiaOutput>, bpm: f64) -> f64 {
    let bpm_proxy = ((bpm - BPM_PROXY_FLOOR) / BPM_PROXY_RANGE).clamp(0.0, 1.0);
    let Some(essentia) = essentia else {
        return bpm_proxy;
    };

    let danceability = essentia.danceability;
    let loudness_integrated = essentia.loudness_integrated;
    let onset_rate = essentia.onset_rate;

    match (danceability, loudness_integrated, onset_rate) {
        (Some(dance), Some(loudness), Some(onset)) => {
            let normalized_dance = (dance / DANCEABILITY_MAX).clamp(0.0, 1.0);
            let normalized_loudness = ((loudness - LOUDNESS_FLOOR_LUFS) / LOUDNESS_RANGE_LUFS).clamp(0.0, 1.0);
            let onset_rate_normalized = (onset / ONSET_RATE_MAX).clamp(0.0, 1.0);
            ((ENERGY_W_DANCE * normalized_dance) + (ENERGY_W_LOUDNESS * normalized_loudness) + (ENERGY_W_ONSET * onset_rate_normalized))
                .clamp(0.0, 1.0)
        }
        _ => bpm_proxy,
    }
}

fn canonicalize_genre(raw_genre: &str) -> Option<String> {
    let trimmed = raw_genre.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(canonical) = genre::canonical_genre_name(trimmed) {
        return Some(canonical.to_string());
    }
    if let Some(alias_target) = genre::canonical_genre_from_alias(trimmed) {
        return Some(alias_target.to_string());
    }
    None
}

pub(super) fn genre_family_for(canonical_genre: &str) -> GenreFamily {
    genre::genre_family(canonical_genre)
}

pub(super) fn key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    parse_camelot_key(raw_key).or_else(|| musical_key_to_camelot(raw_key))
}

pub(super) fn parse_camelot_key(raw_key: &str) -> Option<CamelotKey> {
    let trimmed = raw_key.trim().to_ascii_uppercase();
    if trimmed.len() < 2 {
        return None;
    }
    let (number, letter_str) = trimmed.split_at(trimmed.len() - 1);
    let letter = letter_str.chars().next()?;
    if letter != 'A' && letter != 'B' {
        return None;
    }
    let number: u8 = number.parse().ok()?;
    if !(1..=12).contains(&number) {
        return None;
    }
    Some(CamelotKey { number, letter })
}

pub(super) fn musical_key_to_camelot(raw_key: &str) -> Option<CamelotKey> {
    let normalized = raw_key.trim().replace('\u{266F}', "#").replace('\u{266D}', "b");
    if normalized.is_empty() {
        return None;
    }
    let lower = normalized.to_ascii_lowercase();

    let (root_raw, is_minor) = if lower.ends_with("minor") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], true)
    } else if lower.ends_with("min") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], true)
    } else if lower.ends_with('m') && normalized.len() > 1 {
        (&normalized[..normalized.len() - 1], true)
    } else if lower.ends_with("major") && normalized.len() > 5 {
        (&normalized[..normalized.len() - 5], false)
    } else if lower.ends_with("maj") && normalized.len() > 3 {
        (&normalized[..normalized.len() - 3], false)
    } else {
        (normalized.as_str(), false)
    };
    let root = normalize_key_root(root_raw)?;

    let (number, letter) = if is_minor {
        match root.as_str() {
            "G#" | "Ab" => (1, 'A'),
            "D#" | "Eb" => (2, 'A'),
            "A#" | "Bb" => (3, 'A'),
            "F" => (4, 'A'),
            "C" => (5, 'A'),
            "G" => (6, 'A'),
            "D" => (7, 'A'),
            "A" => (8, 'A'),
            "E" => (9, 'A'),
            "B" => (10, 'A'),
            "F#" | "Gb" => (11, 'A'),
            "C#" | "Db" => (12, 'A'),
            _ => return None,
        }
    } else {
        match root.as_str() {
            "B" => (1, 'B'),
            "F#" | "Gb" => (2, 'B'),
            "C#" | "Db" => (3, 'B'),
            "G#" | "Ab" => (4, 'B'),
            "D#" | "Eb" => (5, 'B'),
            "A#" | "Bb" => (6, 'B'),
            "F" => (7, 'B'),
            "C" => (8, 'B'),
            "G" => (9, 'B'),
            "D" => (10, 'B'),
            "A" => (11, 'B'),
            "E" => (12, 'B'),
            _ => return None,
        }
    };
    Some(CamelotKey { number, letter })
}

fn normalize_key_root(root: &str) -> Option<String> {
    let stripped: String = root.chars().filter(|ch| !ch.is_whitespace()).collect();
    if stripped.is_empty() {
        return None;
    }
    let mut chars = stripped.chars();
    let letter = chars.next()?.to_ascii_uppercase();
    if !matches!(letter, 'A' | 'B' | 'C' | 'D' | 'E' | 'F' | 'G') {
        return None;
    }

    let accidental = chars.next();
    if chars.next().is_some() {
        return None;
    }

    let normalized = match accidental {
        Some('#') => format!("{letter}#"),
        Some('b') | Some('B') => format!("{letter}b"),
        Some(_) => return None,
        None => letter.to_string(),
    };
    Some(normalized)
}

pub(super) fn format_camelot(key: CamelotKey) -> String {
    format!("{}{}", key.number, key.letter)
}

/// Transpose a Camelot key by the given number of semitones.
/// +1 semitone = +7 Camelot positions mod 12 (circle of fifths).
/// Letter (A/B) is unchanged.
pub(super) fn transpose_camelot_key(key: CamelotKey, semitones: i32) -> CamelotKey {
    // Each semitone = +7 positions on the Camelot wheel (mod 12)
    let steps = ((semitones % 12) * 7).rem_euclid(12);
    let new_number = ((key.number as i32 - 1 + steps) % 12 + 1) as u8;
    CamelotKey {
        number: new_number,
        letter: key.letter,
    }
}

/// Map a genre/style string through the taxonomy.
/// Returns (maps_to, mapping_type) where mapping_type is "exact", "alias", or "unknown".
pub(super) fn map_genre_through_taxonomy(style: &str) -> (Option<String>, &'static str) {
    if let Some(canonical) = genre::canonical_genre_name(style) {
        (Some(canonical.to_string()), "exact")
    } else if let Some(canonical) = genre::canonical_genre_from_alias(style) {
        (Some(canonical.to_string()), "alias")
    } else {
        (None, "unknown")
    }
}
