//! Time signature detection
//!
//! Detects musical time signature by analyzing beat patterns and accent structures.
//! Supports common time signatures: 4/4, 3/4, 6/8.
//!
//! This module jointly scores meter and downbeat phase from onset accents. Beat
//! timing alone cannot distinguish regular 3/4, 4/4, and 6/8 grids, so uniform or
//! weak evidence deliberately falls back to low-confidence 4/4.
//!
//! # Algorithm
//!
//! 1. Match tracked beats to nearby onset evidence
//! 2. Normalize onset accents robustly
//! 3. Score every supported meter and downbeat phase
//! 4. Reject weak, ambiguous, or insufficient evidence
//! 5. Return the selected meter, phase, and bounded confidence
//!
//! # Example
//!
//! ```no_run
//! use stratum_dsp::features::beat_tracking::time_signature::detect_time_signature;
//!
//! let beats = vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5]; // Beat times in seconds
//! let bpm = 120.0;
//!
//! let (time_sig, confidence) = detect_time_signature(&beats, bpm)?;
//!
//! println!("Time signature: {} (confidence: {:.2})", time_sig.name(), confidence);
//! # Ok::<(), stratum_dsp::AnalysisError>(())
//! ```

use crate::error::AnalysisError;
use crate::features::beat_tracking::BeatEvidence;

/// Numerical stability epsilon
const EPSILON: f32 = 1e-10;
/// Accent ranges at or below this value contain no useful contrast.
const UNIFORM_ACCENT_EPSILON: f32 = 1e-6;
/// Maximum distance between a tracked beat and its onset evidence.
const ONSET_MATCH_BEAT_FRACTION: f32 = 0.25;
/// At least this many fully mapped bars are required for a positive claim.
const MIN_COMPLETE_BARS: usize = 3;
/// Minimum candidate score for a positive meter claim.
const MIN_CANDIDATE_SCORE: f32 = 0.10;
/// Minimum separation between the best and runner-up candidates.
const MIN_CANDIDATE_MARGIN: f32 = 0.05;
/// Confidence returned when meter evidence is ambiguous or insufficient.
const FALLBACK_CONFIDENCE: f32 = 0.20;
/// Primary downbeat contribution to a compound 6/8 score.
const SIX_EIGHT_PRIMARY_WEIGHT: f32 = 0.65;
/// Beat-four contribution to a compound 6/8 score.
const SIX_EIGHT_SECONDARY_WEIGHT: f32 = 0.35;

/// Musical time signature
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeSignature {
    /// 4/4 time (common time)
    FourFour,
    /// 3/4 time (waltz time)
    ThreeFour,
    /// 6/8 time (compound duple)
    SixEight,
}

impl TimeSignature {
    /// Get beats per bar for this time signature
    pub fn beats_per_bar(&self) -> u32 {
        match self {
            TimeSignature::FourFour => 4,
            TimeSignature::ThreeFour => 3,
            TimeSignature::SixEight => 6,
        }
    }

    /// Get name as string (e.g., "4/4", "3/4", "6/8")
    pub fn name(&self) -> &'static str {
        match self {
            TimeSignature::FourFour => "4/4",
            TimeSignature::ThreeFour => "3/4",
            TimeSignature::SixEight => "6/8",
        }
    }
}

/// Internal meter decision with explicit downbeat phase.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MeterDetection {
    /// Selected meter.
    pub(crate) time_signature: TimeSignature,
    /// Index of the first observed downbeat, always less than beats per bar.
    pub(crate) downbeat_phase: usize,
    /// Bounded decision confidence.
    pub(crate) confidence: f32,
}

impl MeterDetection {
    fn fallback() -> Self {
        Self {
            time_signature: TimeSignature::FourFour,
            downbeat_phase: 0,
            confidence: FALLBACK_CONFIDENCE,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CandidateScore {
    time_signature: TimeSignature,
    downbeat_phase: usize,
    score: f32,
}

#[derive(Debug, Clone, PartialEq)]
struct MappedAccentEvidence {
    /// Normalized accents aligned to tracked beats; unmatched beats are zero.
    accents: Vec<f32>,
    /// Whether each tracked beat received a unique nearby onset.
    mapped: Vec<bool>,
}

/// Detect time signature from beat pattern
///
/// Compatibility API for callers that only have beat times.
///
/// Uniform evidence is intentionally ambiguous, so this wrapper returns the
/// documented low-confidence 4/4 fallback. Callers with audio-derived accents
/// use the crate-private evidence-aware detector.
///
/// # Arguments
///
/// * `beats` - Beat times in seconds (sorted)
/// * `bpm_estimate` - BPM estimate (for context)
///
/// # Returns
///
/// Detected time signature with confidence score
///
pub fn detect_time_signature(
    beats: &[f32],
    bpm_estimate: f32,
) -> Result<(TimeSignature, f32), AnalysisError> {
    let uniform_accents = vec![1.0; beats.len()];
    let detection = detect_time_signature_with_evidence(
        beats,
        bpm_estimate,
        BeatEvidence {
            onset_times: beats,
            accents: &uniform_accents,
        },
    )?;
    Ok((detection.time_signature, detection.confidence))
}

/// Detect meter and phase from tracked beats plus onset-accent evidence.
pub(crate) fn detect_time_signature_with_evidence(
    beats: &[f32],
    bpm_estimate: f32,
    evidence: BeatEvidence<'_>,
) -> Result<MeterDetection, AnalysisError> {
    if !bpm_estimate.is_finite() || bpm_estimate <= EPSILON {
        return Err(AnalysisError::InvalidInput(format!(
            "Invalid BPM for time signature detection: {bpm_estimate}"
        )));
    }

    validate_sorted_non_negative(beats, "beats")?;
    evidence.validate()?;
    if beats.is_empty() || evidence.onset_times.is_empty() {
        return Ok(MeterDetection::fallback());
    }

    let min_accent = evidence
        .accents
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min);
    let max_accent = evidence
        .accents
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max);
    let accent_range = max_accent - min_accent;
    if accent_range <= UNIFORM_ACCENT_EPSILON {
        return Ok(MeterDetection::fallback());
    }

    let normalized_accents: Vec<f32> = evidence
        .accents
        .iter()
        .map(|accent| (accent - min_accent) / accent_range)
        .collect();
    let mapped_evidence = map_accents_to_beats(
        beats,
        evidence.onset_times,
        &normalized_accents,
        bpm_estimate,
    );
    let mapped_count = mapped_evidence
        .mapped
        .iter()
        .filter(|mapped| **mapped)
        .count();
    let coverage = mapped_count as f32 / beats.len() as f32;

    let mut candidates = Vec::new();
    for time_signature in [
        TimeSignature::FourFour,
        TimeSignature::ThreeFour,
        TimeSignature::SixEight,
    ] {
        for downbeat_phase in 0..time_signature.beats_per_bar() as usize {
            if let Some(score) = score_candidate(
                &mapped_evidence,
                time_signature,
                downbeat_phase,
                MIN_COMPLETE_BARS,
            ) {
                candidates.push(CandidateScore {
                    time_signature,
                    downbeat_phase,
                    score: score * coverage,
                });
            }
        }
    }

    if candidates.is_empty() {
        return Ok(MeterDetection::fallback());
    }

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| candidate_order(*left).cmp(&candidate_order(*right)))
    });

    let best = candidates[0];
    let runner_up_score = candidates.get(1).map_or(0.0, |candidate| candidate.score);
    let margin = best.score - runner_up_score;
    if best.score < MIN_CANDIDATE_SCORE || margin < MIN_CANDIDATE_MARGIN {
        return Ok(MeterDetection::fallback());
    }

    let confidence =
        (coverage * (0.5 * best.score.max(0.0) + 0.5 * margin.max(0.0))).clamp(0.0, 1.0);
    Ok(MeterDetection {
        time_signature: best.time_signature,
        downbeat_phase: best.downbeat_phase,
        confidence,
    })
}

fn validate_sorted_non_negative(values: &[f32], name: &str) -> Result<(), AnalysisError> {
    for (index, value) in values.iter().enumerate() {
        if !value.is_finite() || *value < 0.0 {
            return Err(AnalysisError::InvalidInput(format!(
                "{name}[{index}] must be finite and non-negative, got {value}"
            )));
        }
    }
    if let Some((index, pair)) = values
        .windows(2)
        .enumerate()
        .find(|(_, pair)| pair[0] > pair[1])
    {
        return Err(AnalysisError::InvalidInput(format!(
            "{name} must be sorted: {name}[{index}]={} exceeds {name}[{}]={}",
            pair[0],
            index + 1,
            pair[1]
        )));
    }
    Ok(())
}

fn map_accents_to_beats(
    beats: &[f32],
    onset_times: &[f32],
    normalized_accents: &[f32],
    bpm_estimate: f32,
) -> MappedAccentEvidence {
    let tolerance = ONSET_MATCH_BEAT_FRACTION * 60.0 / bpm_estimate;
    let mut used = vec![false; onset_times.len()];
    let mut accents = Vec::with_capacity(beats.len());
    let mut mapped = Vec::with_capacity(beats.len());

    for beat in beats {
        let nearest = onset_times
            .iter()
            .enumerate()
            .filter(|(index, _)| !used[*index])
            .map(|(index, onset)| (index, (*onset - *beat).abs()))
            .filter(|(_, distance)| *distance <= tolerance)
            .min_by(|left, right| {
                left.1
                    .total_cmp(&right.1)
                    .then_with(|| left.0.cmp(&right.0))
            });

        if let Some((index, _)) = nearest {
            used[index] = true;
            accents.push(normalized_accents[index]);
            mapped.push(true);
        } else {
            accents.push(0.0);
            mapped.push(false);
        }
    }

    MappedAccentEvidence { accents, mapped }
}

fn score_candidate(
    evidence: &MappedAccentEvidence,
    time_signature: TimeSignature,
    downbeat_phase: usize,
    minimum_complete_bars: usize,
) -> Option<f32> {
    let beats_per_bar = time_signature.beats_per_bar() as usize;
    if downbeat_phase >= beats_per_bar {
        return None;
    }

    let scored_bars: Vec<(&[f32], &[bool])> = (downbeat_phase..evidence.accents.len())
        .step_by(beats_per_bar)
        .filter_map(|start| {
            Some((
                evidence.accents.get(start..start + beats_per_bar)?,
                evidence.mapped.get(start..start + beats_per_bar)?,
            ))
        })
        .collect();
    let fully_mapped_bars = scored_bars
        .iter()
        .filter(|(_, mapped)| mapped.iter().all(|mapped| *mapped))
        .count();
    if fully_mapped_bars < minimum_complete_bars {
        return None;
    }

    let primary_mean = mean(scored_bars.iter().map(|(accents, _)| accents[0]));

    match time_signature {
        TimeSignature::ThreeFour | TimeSignature::FourFour => {
            let other_mean = mean(
                scored_bars
                    .iter()
                    .flat_map(|(accents, _)| accents[1..].iter().copied()),
            );
            Some(primary_mean - other_mean)
        }
        TimeSignature::SixEight => {
            let secondary_mean = mean(scored_bars.iter().map(|(accents, _)| accents[3]));
            if secondary_mean > primary_mean {
                return None;
            }
            let other_mean = mean(
                scored_bars
                    .iter()
                    .flat_map(|(accents, _)| [1, 2, 4, 5].into_iter().map(|index| accents[index])),
            );
            Some(
                SIX_EIGHT_PRIMARY_WEIGHT * primary_mean
                    + SIX_EIGHT_SECONDARY_WEIGHT * secondary_mean
                    - other_mean,
            )
        }
    }
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0, 0_usize), |(sum, count), value| {
        (sum + value, count + 1)
    });
    if count == 0 {
        0.0
    } else {
        sum / count as f32
    }
}

fn candidate_order(candidate: CandidateScore) -> (u8, usize) {
    let meter_order = match candidate.time_signature {
        TimeSignature::FourFour => 0,
        TimeSignature::ThreeFour => 1,
        TimeSignature::SixEight => 2,
    };
    (meter_order, candidate.downbeat_phase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::beat_tracking::BeatEvidence;

    fn regular_beats(count: usize) -> Vec<f32> {
        (0..count).map(|index| index as f32 * 0.5).collect()
    }

    fn accents_for_meter(count: usize, beats_per_bar: usize, downbeat_phase: usize) -> Vec<f32> {
        (0..count)
            .map(|index| {
                if index % beats_per_bar == downbeat_phase {
                    1.0
                } else {
                    0.1
                }
            })
            .collect()
    }

    fn detect_with_accents(beats: &[f32], accents: &[f32]) -> MeterDetection {
        detect_time_signature_with_evidence(
            beats,
            120.0,
            BeatEvidence {
                onset_times: beats,
                accents,
            },
        )
        .unwrap()
    }

    #[test]
    fn uniform_evidence_falls_back_to_low_confidence_four_four() {
        let beats = regular_beats(16);
        let detection = detect_with_accents(&beats, &vec![0.5; beats.len()]);

        assert_eq!(detection.time_signature, TimeSignature::FourFour);
        assert_eq!(detection.downbeat_phase, 0);
        assert!((0.0..=0.25).contains(&detection.confidence));
    }

    #[test]
    fn accent_evidence_selects_four_four_and_shifted_phase() {
        let beats = regular_beats(18);

        for phase in [0, 2] {
            let accents = accents_for_meter(beats.len(), 4, phase);
            let detection = detect_with_accents(&beats, &accents);

            assert_eq!(detection.time_signature, TimeSignature::FourFour);
            assert_eq!(detection.downbeat_phase, phase);
            assert!((0.0..=1.0).contains(&detection.confidence));
            assert!(detection.confidence > 0.20);
        }
    }

    #[test]
    fn accent_evidence_selects_three_four_and_phase() {
        let beats = regular_beats(14);
        let accents = accents_for_meter(beats.len(), 3, 1);
        let detection = detect_with_accents(&beats, &accents);

        assert_eq!(detection.time_signature, TimeSignature::ThreeFour);
        assert_eq!(detection.downbeat_phase, 1);
        assert!((0.0..=1.0).contains(&detection.confidence));
        assert!(detection.confidence > 0.20);
    }

    #[test]
    fn compound_accents_select_six_eight_and_phase() {
        let beats = regular_beats(26);
        let phase = 2;
        let accents: Vec<f32> = (0..beats.len())
            .map(|index| {
                if index % 6 == phase {
                    1.0
                } else if index % 6 == (phase + 3) % 6 {
                    0.55
                } else {
                    0.1
                }
            })
            .collect();
        let detection = detect_with_accents(&beats, &accents);

        assert_eq!(detection.time_signature, TimeSignature::SixEight);
        assert_eq!(detection.downbeat_phase, phase);
        assert!((0.0..=1.0).contains(&detection.confidence));
        assert!(detection.confidence > 0.20);
    }

    #[test]
    fn insufficient_evidence_falls_back() {
        let beats = regular_beats(8);
        let accents = accents_for_meter(beats.len(), 4, 0);
        let detection = detect_with_accents(&beats, &accents);

        assert_eq!(detection.time_signature, TimeSignature::FourFour);
        assert_eq!(detection.downbeat_phase, 0);
        assert!((0.0..=0.25).contains(&detection.confidence));
    }

    #[test]
    fn unmatched_beats_receive_zero_accent_without_reusing_onsets() {
        let mapped = map_accents_to_beats(&[0.0, 0.5, 1.0], &[0.0, 1.0], &[0.8, 0.4], 120.0);

        assert_eq!(mapped.accents, vec![0.8, 0.0, 0.4]);
        assert_eq!(mapped.mapped, vec![true, false, true]);
    }

    #[test]
    fn partially_mapped_contradictory_bars_force_fallback() {
        let beats = regular_beats(48);
        let mut onset_times = Vec::new();
        let mut accents = Vec::new();

        for (index, time) in beats.iter().copied().enumerate() {
            // Establish exactly three fully mapped 4/4 bars, then provide nine
            // partial bars whose strongest evidence contradicts phase zero.
            // Dropping partial bars would incorrectly retain a positive 4/4 claim.
            if index < 12 || index % 4 != 0 {
                onset_times.push(time);
                accents.push(if index % 4 == 0 || index % 4 == 1 {
                    1.0
                } else {
                    0.1
                });
            }
        }

        let detection = detect_time_signature_with_evidence(
            &beats,
            120.0,
            BeatEvidence {
                onset_times: &onset_times,
                accents: &accents,
            },
        )
        .unwrap();

        assert_eq!(detection.time_signature, TimeSignature::FourFour);
        assert_eq!(detection.downbeat_phase, 0);
        assert_eq!(detection.confidence, FALLBACK_CONFIDENCE);
    }

    #[test]
    fn malformed_evidence_is_rejected() {
        let beats = regular_beats(16);
        let valid_accents = accents_for_meter(beats.len(), 4, 0);

        for (onset_times, accents) in [
            (
                {
                    let mut values = beats.clone();
                    values[3] = f32::NAN;
                    values
                },
                valid_accents.clone(),
            ),
            (
                {
                    let mut values = beats.clone();
                    values[3] = -0.5;
                    values
                },
                valid_accents.clone(),
            ),
            (
                {
                    let mut values = beats.clone();
                    values.swap(3, 4);
                    values
                },
                valid_accents.clone(),
            ),
            (beats.clone(), {
                let mut values = valid_accents.clone();
                values[3] = f32::INFINITY;
                values
            }),
            (beats.clone(), {
                let mut values = valid_accents.clone();
                values[3] = -0.1;
                values
            }),
            (beats[..beats.len() - 1].to_vec(), valid_accents.clone()),
        ] {
            let error = detect_time_signature_with_evidence(
                &beats,
                120.0,
                BeatEvidence {
                    onset_times: &onset_times,
                    accents: &accents,
                },
            )
            .unwrap_err();
            assert!(matches!(error, AnalysisError::InvalidInput(_)));
        }
    }

    #[test]
    fn public_compatibility_signature_uses_uniform_fallback() {
        let beats = regular_beats(16);

        let (time_sig, confidence) = detect_time_signature(&beats, 120.0).unwrap();

        assert_eq!(time_sig, TimeSignature::FourFour);
        assert!((0.0..=0.25).contains(&confidence));
    }

    #[test]
    fn test_time_signature_beats_per_bar() {
        assert_eq!(TimeSignature::FourFour.beats_per_bar(), 4);
        assert_eq!(TimeSignature::ThreeFour.beats_per_bar(), 3);
        assert_eq!(TimeSignature::SixEight.beats_per_bar(), 6);
    }

    #[test]
    fn test_time_signature_name() {
        assert_eq!(TimeSignature::FourFour.name(), "4/4");
        assert_eq!(TimeSignature::ThreeFour.name(), "3/4");
        assert_eq!(TimeSignature::SixEight.name(), "6/8");
    }
}
