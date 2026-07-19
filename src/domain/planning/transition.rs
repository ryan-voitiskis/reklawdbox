//! Transition-axis scoring and composition.

use crate::domain::classification::taxonomy::GenreFamily;

use super::{
    AxisScore, CamelotKey, EnergyPhase, HarmonicMixingStyle, PriorityWeights, ScoreAdjustment,
    TrackProfile, TransitionMixingPolicy, TransitionMoment, TransitionScores, format_camelot,
    parse_camelot_key, score_key_with_pitch_shifts, transpose_camelot_key,
};

const BRIGHTNESS_SIMILAR_HZ: f64 = 300.0;
const BRIGHTNESS_SHIFT_HZ: f64 = 800.0;
const BRIGHTNESS_JUMP_HZ: f64 = 1500.0;

const RHYTHM_MATCHED_DELTA: f64 = 0.1;
const RHYTHM_MANAGEABLE_DELTA: f64 = 0.25;
const RHYTHM_CHALLENGING_DELTA: f64 = 0.5;

pub(crate) fn score_key_axis(from: Option<CamelotKey>, to: Option<CamelotKey>) -> AxisScore {
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

pub(crate) fn score_transition_profiles(
    from: &TrackProfile,
    to: &TrackProfile,
    mixing: TransitionMixingPolicy<'_>,
    moment: TransitionMoment,
) -> TransitionScores {
    let TransitionMixingPolicy {
        weights,
        master_tempo,
        harmonic_style,
    } = mixing;
    let TransitionMoment {
        from_phase,
        to_phase,
        genre_run_length,
        play_bpms,
    } = moment;
    let (
        effective_to_key,
        pitch_shift_semitones,
        scoring_from_key,
        scoring_to_key,
        bpm,
        exact_from_shift,
        exact_to_shift,
    ) = if let Some((from_play_bpm, to_play_bpm)) = play_bpms {
        let exact_from = if from.bpm > 0.0 && from_play_bpm > 0.0 {
            12.0 * (from_play_bpm / from.bpm).log2()
        } else {
            0.0
        };
        let exact_to = if to.bpm > 0.0 && to_play_bpm > 0.0 {
            12.0 * (to_play_bpm / to.bpm).log2()
        } else {
            0.0
        };
        let from_shift = exact_from.round() as i32;
        let to_shift = exact_to.round() as i32;

        let effective_from_key = if !master_tempo && from_shift != 0 {
            from.camelot_key
                .map(|k| transpose_camelot_key(k, from_shift))
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

        let bpm_score = score_bpm_axis(to_play_bpm, to.bpm);

        (
            effective_to_key_display,
            to_shift,
            effective_from_key,
            effective_to_key,
            bpm_score,
            if master_tempo { 0.0 } else { exact_from },
            if master_tempo { 0.0 } else { exact_to },
        )
    } else {
        let (eff_to_key, shift, exact_to) = if !master_tempo && from.bpm > 0.0 && to.bpm > 0.0 {
            let exact = 12.0 * (from.bpm / to.bpm).log2();
            let integer_shift = exact.round() as i32;
            if integer_shift != 0 {
                let transposed = to
                    .camelot_key
                    .map(|k| transpose_camelot_key(k, integer_shift));
                (transposed.map(format_camelot), integer_shift, exact)
            } else {
                (None, 0, exact)
            }
        } else {
            (None, 0, 0.0)
        };

        let scoring_to = if let Some(ref ek) = eff_to_key {
            parse_camelot_key(ek)
        } else {
            to.camelot_key
        };

        let bpm_score = score_bpm_axis(from.bpm, to.bpm);

        (
            eff_to_key,
            shift,
            from.camelot_key,
            scoring_to,
            bpm_score,
            0.0,
            exact_to,
        )
    };

    // Interpolate between bracketing integer transpositions to avoid the cliff
    // where rounding a fractional semitone shift jumps 7 Camelot positions.
    let key = if exact_from_shift.abs() > 0.01 || exact_to_shift.abs() > 0.01 {
        score_key_with_pitch_shifts(
            from.camelot_key,
            to.camelot_key,
            exact_from_shift,
            exact_to_shift,
        )
    } else {
        score_key_axis(scoring_from_key, scoring_to_key)
    };
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
        genre_run_length,
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
        weights,
    );

    let mut adjustments = Vec::new();

    // Axis bonuses/penalties were already baked into axis scores; compute
    // their weighted composite impact for transparency reporting.
    let mut total_weight = weights.key + weights.bpm + weights.energy + weights.genre;
    if brightness_available {
        total_weight += weights.brightness;
    }
    if rhythm_available {
        total_weight += weights.rhythm;
    }
    if total_weight > f64::EPSILON {
        if genre.label.contains("streak bonus") {
            let delta = weights.genre * 0.1 / total_weight;
            adjustments.push(ScoreAdjustment {
                kind: "genre_streak",
                delta,
                composite_without: composite - delta,
                reason: "Genre family streak bonus (+0.1 on genre axis)".to_string(),
            });
        }
        if genre.label.contains("early switch penalty") {
            let delta = -(weights.genre * 0.1 / total_weight);
            adjustments.push(ScoreAdjustment {
                kind: "genre_early_switch",
                delta,
                composite_without: composite - delta,
                reason: "Genre family switched too early (-0.1 on genre axis)".to_string(),
            });
        }
        if energy.label.contains("dynamic boundary boost") {
            let delta = weights.energy * 0.1 / total_weight;
            adjustments.push(ScoreAdjustment {
                kind: "phase_boundary_boost",
                delta,
                composite_without: composite - delta,
                reason: "Phase boundary with dynamic range (+0.1 on energy axis)".to_string(),
            });
        }
        if energy.label.contains("sustained-peak consistency boost") {
            let delta = weights.energy * 0.05 / total_weight;
            adjustments.push(ScoreAdjustment {
                kind: "sustained_peak",
                delta,
                composite_without: composite - delta,
                reason: "Sustained peak with tight loudness range (+0.05 on energy axis)"
                    .to_string(),
            });
        }
    }

    if let Some(style) = harmonic_style {
        let min_key = harmonic_style_min_key(style, to_phase);
        if key.value < min_key {
            let composite_without = composite;
            let factor = harmonic_penalty_factor(style);
            composite *= factor;
            adjustments.push(ScoreAdjustment {
                kind: "harmonic_gate",
                delta: composite - composite_without,
                composite_without,
                reason: format!(
                    "Key score {:.2} below {style:?} threshold {:.2} — {factor}x penalty",
                    key.value, min_key,
                ),
            });
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
        adjustments,
    }
}

pub(crate) fn round_to_3_decimals(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(crate) fn bpm_pitch_shift(native_bpm: f64, ref_bpm: f64) -> f64 {
    if native_bpm > 0.0 {
        12.0 * (ref_bpm / native_bpm).log2()
    } else {
        0.0
    }
}

pub(crate) fn score_bpm_axis(from_bpm: f64, to_bpm: f64) -> AxisScore {
    if from_bpm <= 0.0 || to_bpm <= 0.0 {
        return AxisScore {
            value: 0.5,
            label: "Unknown BPM".to_string(),
        };
    }
    let delta = (from_bpm - to_bpm).abs();
    let pct = delta / from_bpm * 100.0;
    let value = (-0.019 * pct * pct).exp();
    let label_category = if pct < 2.0 {
        "Seamless"
    } else if pct < 4.0 {
        "Comfortable"
    } else if pct < 6.0 {
        "Noticeable"
    } else if pct < 9.0 {
        "Creative transition needed"
    } else {
        "Jarring"
    };
    AxisScore {
        value,
        label: format!("{label_category} ({pct:.1}%, {delta:.1} BPM)"),
    }
}

pub(crate) fn score_energy_axis(
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
        (Some(EnergyPhase::Peak), Some(loudness_range))
            if !is_phase_boundary && loudness_range < 4.0 =>
        {
            axis.value = (axis.value + 0.05).clamp(0.0, 1.0);
            axis.label.push_str(" + sustained-peak consistency boost");
        }
        _ => {}
    }
    axis
}

pub(crate) fn score_genre_axis(
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

    if genre_compatible
        && from_family != GenreFamily::Other
        && genre_run_length > 0
        && genre_run_length < 5
    {
        axis.value = (axis.value + 0.1).min(1.0);
        axis.label.push_str(" + streak bonus");
    } else if !genre_compatible && genre_run_length > 0 && genre_run_length < 2 {
        axis.value = (axis.value - 0.1).max(0.0);
        axis.label.push_str(" + early switch penalty");
    }

    axis
}

pub(crate) fn score_brightness_axis(
    from_centroid: Option<f64>,
    to_centroid: Option<f64>,
) -> AxisScore {
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
            label: format!("Similar timbre (delta {delta:.0} Hz)"),
        }
    } else if delta < BRIGHTNESS_SHIFT_HZ {
        AxisScore {
            value: 0.7,
            label: format!("Noticeable brightness shift (delta {delta:.0} Hz)"),
        }
    } else if delta < BRIGHTNESS_JUMP_HZ {
        AxisScore {
            value: 0.4,
            label: format!("Large timbral jump (delta {delta:.0} Hz)"),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Jarring brightness jump (delta {delta:.0} Hz)"),
        }
    }
}

pub(crate) fn score_rhythm_axis(
    from_regularity: Option<f64>,
    to_regularity: Option<f64>,
) -> AxisScore {
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
            label: format!("Matching groove (delta {delta:.2})"),
        }
    } else if delta < RHYTHM_MANAGEABLE_DELTA {
        AxisScore {
            value: 0.7,
            label: format!("Manageable groove shift (delta {delta:.2})"),
        }
    } else if delta < RHYTHM_CHALLENGING_DELTA {
        AxisScore {
            value: 0.4,
            label: format!("Challenging groove shift (delta {delta:.2})"),
        }
    } else {
        AxisScore {
            value: 0.2,
            label: format!("Groove clash (delta {delta:.2})"),
        }
    }
}

// ---------------------------------------------------------------------------
// Pool-specific axis functions (symmetric, no sequential context)
// ---------------------------------------------------------------------------

fn harmonic_penalty_factor(style: HarmonicMixingStyle) -> f64 {
    match style {
        HarmonicMixingStyle::Conservative => 0.1,
        HarmonicMixingStyle::Balanced | HarmonicMixingStyle::Adventurous => 0.5,
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

pub(crate) fn composite_score(
    key_score: f64,
    bpm_score: f64,
    energy_score: f64,
    genre_score: f64,
    brightness_score: Option<f64>,
    rhythm_score: Option<f64>,
    weights: &PriorityWeights,
) -> f64 {
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
