//! Track-section detection.
//!
//! Real electronic tracks have intro / main-groove / breakdown / outro
//! sections with very different content. Track-level feature aggregation
//! (mean kick spacing across the whole track, mean stab onset density,
//! etc.) blends real signal in the main groove with noise in the
//! intro/outro and the (often kickless) breakdown.
//!
//! This module returns a `Vec<TrackSection>` so downstream features can
//! restrict their statistics to the sections that actually contain the
//! signal they're trying to characterise — e.g. compute kick-pattern
//! features only over MainGroove sections, sub-rumble only over
//! kick-active sections, etc.
//!
//! The classification is deliberately coarse: four labels, identified
//! purely from sliding-window kick density + RMS energy. It's enough to
//! tell "kick present and loud" from "kick absent" from "loud but no kick".
//! Finer structural analysis (verse/chorus/middle-eight) is out of scope.

use crate::error::AnalysisError;

use serde::{Deserialize, Serialize};

/// Coarse section type. Identified from kick-density + RMS energy only.
///
/// Marked `#[non_exhaustive]` so adding a fifth variant (e.g. `Buildup`,
/// `Drop`) later is not a breaking change for downstream `match` arms.
/// Internal `match`es should use a `_` arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SectionKind {
    /// Low energy, low kick density, near track start.
    Intro,
    /// High energy AND high kick density. The "drop" / main beat.
    MainGroove,
    /// High energy but low kick density (or low energy mid-track) —
    /// drums removed for a buildup or harmonic-only section.
    Breakdown,
    /// Low energy, low kick density, near track end.
    Outro,
}

/// One contiguous section of the track.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackSection {
    /// Start time in seconds from the audio's sample 0.
    pub start_seconds: f32,
    /// End time in seconds (exclusive).
    pub end_seconds: f32,
    /// Section type — see `SectionKind`.
    pub kind: SectionKind,
    /// Mean RMS amplitude in the kick band (40–120 Hz). High during a
    /// 4-on-the-floor kick, low when the kick drops out for a breakdown
    /// or ramps in/out at intro/outro. Not a kick *count* — a presence
    /// proxy that's robust to bassline contamination.
    pub kick_band_rms: f32,
    /// Mean RMS amplitude in the broadband range (200–8000 Hz). High
    /// during full-mix sections, low at intro/outro fade.
    pub broadband_rms: f32,
}

const SECTION_WINDOW_SECONDS: f32 = 4.0;
const SECTION_HOP_SECONDS: f32 = 1.0;
/// Sections shorter than this are merged into their longer neighbours.
/// Stops sub-bar flicker from creating a false breakdown every 8 beats.
const SECTION_MIN_LENGTH_SECONDS: f32 = 8.0;
/// Threshold = min + this × (max − min). Range-relative rather than
/// percentile-based so the labels don't depend on what fraction of the
/// track each section type occupies — a 50% intro doesn't pull the
/// threshold up into the main-groove energy band, which percentile
/// thresholds do for bimodal distributions.
const RANGE_FRACTION: f32 = 0.40;
/// Below this coefficient-of-variation (range / mean), the signal is too
/// uniform to threshold meaningfully — we treat every window as
/// "high" so uniform tracks don't get false-split.
const MIN_COEFFICIENT_OF_VARIATION: f32 = 0.40;
/// First/last fraction of the track that can be labelled Intro/Outro.
/// Mid-track low-energy sections are Breakdowns regardless.
const INTRO_OUTRO_FRACTION: f32 = 0.30;

const KICK_BAND_LOW_HZ: f32 = 40.0;
const KICK_BAND_HIGH_HZ: f32 = 120.0;
const BROADBAND_LOW_HZ: f32 = 200.0;
const BROADBAND_HIGH_HZ: f32 = 8000.0;

/// Detect coarse track sections (Intro / MainGroove / Breakdown / Outro)
/// from a magnitude spectrogram.
///
/// Uses two signals over a sliding 4 s window with 1 s hop:
/// - **Kick-band RMS** (40–120 Hz) — presence proxy for the kick.
///   Robust to bassline contamination because it integrates power, not
///   onset counts (which the dub_stab work showed pick up basslines too).
/// - **Broadband RMS** (200–8000 Hz) — overall mix loudness.
///
/// Each signal is thresholded at `min + RANGE_FRACTION × (max − min)`,
/// gated by a coefficient-of-variation check so uniform tracks don't get
/// false-split into spurious sections.
pub fn detect_track_sections(
    spec: &[Vec<f32>],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
) -> Result<Vec<TrackSection>, AnalysisError> {
    if hop_size == 0 || sample_rate == 0 || frame_size == 0 {
        return Err(AnalysisError::InvalidInput(
            "hop_size, sample_rate, frame_size must all be > 0".to_string(),
        ));
    }
    if spec.is_empty() {
        return Ok(Vec::new());
    }

    let frames_per_second = sample_rate as f32 / hop_size as f32;
    let track_duration_seconds = spec.len() as f32 / frames_per_second;
    if track_duration_seconds < SECTION_WINDOW_SECONDS {
        return Ok(vec![TrackSection {
            start_seconds: 0.0,
            end_seconds: track_duration_seconds,
            kind: SectionKind::MainGroove,
            kick_band_rms: 0.0,
            broadband_rms: rms_for_range(spec, 0, spec.len(), 0, spec[0].len()),
        }]);
    }

    let n_bins = spec[0].len();
    let bin_width_hz = sample_rate as f32 / frame_size as f32;
    // Use `.ceil()` for low edges and `.floor()` for high edges so the
    // resulting bin range is strictly inside the named Hz band — avoids
    // pulling in sub-band rumble that's not part of the kick. (At
    // 44.1 kHz / 2048, `.floor(40 / 21.53) = 1` covers ~21–43 Hz; ceil
    // gives bin 2, which starts at ~43 Hz.)
    let kick_bin_low = (KICK_BAND_LOW_HZ / bin_width_hz).ceil() as usize;
    let kick_bin_high = ((KICK_BAND_HIGH_HZ / bin_width_hz).floor() as usize).min(n_bins - 1);
    let broad_bin_low = (BROADBAND_LOW_HZ / bin_width_hz).ceil() as usize;
    let broad_bin_high = ((BROADBAND_HIGH_HZ / bin_width_hz).floor() as usize).min(n_bins - 1);

    let window_frames = (SECTION_WINDOW_SECONDS * frames_per_second).round() as usize;
    let hop_frames = (SECTION_HOP_SECONDS * frames_per_second).round() as usize;
    if hop_frames == 0 {
        return Err(AnalysisError::InvalidInput(
            "section hop resolved to 0 frames; sample_rate / hop_size too small".to_string(),
        ));
    }

    // Stride windows across the spectrogram. Last window is right-aligned to
    // the spectrogram tail so we don't drop trailing audio.
    let mut window_starts: Vec<usize> = (0..)
        .map(|i| i * hop_frames)
        .take_while(|&s| s + window_frames <= spec.len())
        .collect();
    if window_starts.last().copied() != Some(spec.len() - window_frames)
        && spec.len() >= window_frames
    {
        window_starts.push(spec.len() - window_frames);
    }
    let n_windows = window_starts.len();
    if n_windows == 0 {
        return Ok(vec![TrackSection {
            start_seconds: 0.0,
            end_seconds: track_duration_seconds,
            kind: SectionKind::MainGroove,
            kick_band_rms: 0.0,
            broadband_rms: rms_for_range(spec, 0, spec.len(), 0, n_bins),
        }]);
    }

    let mut window_kick = Vec::with_capacity(n_windows);
    let mut window_broad = Vec::with_capacity(n_windows);
    for &start in &window_starts {
        let end = start + window_frames;
        window_kick.push(rms_for_range(
            spec,
            start,
            end,
            kick_bin_low,
            kick_bin_high + 1,
        ));
        window_broad.push(rms_for_range(
            spec,
            start,
            end,
            broad_bin_low,
            broad_bin_high + 1,
        ));
    }

    // Two binary signals: is this window kick-active? broadband-loud?
    let kick_high = build_high_mask(&window_kick);
    let broad_high = build_high_mask(&window_broad);

    let labels: Vec<SectionKind> = window_starts
        .iter()
        .enumerate()
        .map(|(i, &start)| {
            let centre_frame = start + window_frames / 2;
            let centre_seconds = centre_frame as f32 / frames_per_second;
            let track_position = centre_seconds / track_duration_seconds;
            classify_window(broad_high[i], kick_high[i], track_position)
        })
        .collect();

    let mut sections = run_length_encode(
        &labels,
        &window_starts,
        &window_broad,
        &window_kick,
        frames_per_second,
        window_frames,
    );
    merge_short_sections(&mut sections, SECTION_MIN_LENGTH_SECONDS);
    // After short-section absorption, neighbouring sections on opposite
    // sides of an absorbed run can end up adjacent and same-kind (the
    // absorbed section was the only thing separating them). Collapse
    // those before returning.
    collapse_adjacent_same_kind(&mut sections);
    Ok(sections)
}

/// Build a per-window "high signal" boolean mask. A signal is meaningfully
/// non-uniform iff (max − min) / mean ≥ MIN_COEFFICIENT_OF_VARIATION; below
/// that the track is too uniform to split, so every window is "high".
fn build_high_mask(values: &[f32]) -> Vec<bool> {
    if values.is_empty() {
        return Vec::new();
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &v in values {
        if v < min {
            min = v;
        }
        if v > max {
            max = v;
        }
    }
    let range = max - min;
    let cv = if mean.abs() > 1e-9 { range / mean } else { 0.0 };
    if cv < MIN_COEFFICIENT_OF_VARIATION {
        // Uniform signal — call every window "high" so the other axis (or
        // track-position fallback) drives the labelling.
        return vec![true; values.len()];
    }
    let threshold = min + RANGE_FRACTION * range;
    values.iter().map(|&v| v > threshold).collect()
}

fn rms_for_range(
    spec: &[Vec<f32>],
    frame_start: usize,
    frame_end: usize,
    bin_start: usize,
    bin_end: usize,
) -> f32 {
    if frame_start >= frame_end || frame_end > spec.len() {
        return 0.0;
    }
    let mut sum_sq = 0.0_f32;
    let mut n = 0_u32;
    for frame in &spec[frame_start..frame_end] {
        let bin_lo = bin_start.min(frame.len());
        let bin_hi = bin_end.min(frame.len());
        for &v in &frame[bin_lo..bin_hi] {
            sum_sq += v * v;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        (sum_sq / n as f32).sqrt()
    }
}

fn classify_window(high_energy: bool, high_kicks: bool, track_position: f32) -> SectionKind {
    match (high_energy, high_kicks) {
        (true, true) => SectionKind::MainGroove,
        (true, false) => SectionKind::Breakdown,
        (false, _) => {
            if track_position < INTRO_OUTRO_FRACTION {
                SectionKind::Intro
            } else if track_position > 1.0 - INTRO_OUTRO_FRACTION {
                SectionKind::Outro
            } else {
                SectionKind::Breakdown
            }
        }
    }
}

fn run_length_encode(
    labels: &[SectionKind],
    window_starts: &[usize],
    window_broad: &[f32],
    window_kick: &[f32],
    frames_per_second: f32,
    window_frames: usize,
) -> Vec<TrackSection> {
    let mut sections = Vec::new();
    if labels.is_empty() {
        return sections;
    }
    // Each section's end is the right edge of its LAST window. Because
    // windows are wider than the hop, neighbouring sections overlap by
    // `window_frames - hop_frames` seconds — that overlap represents
    // transition uncertainty (the windows themselves overlap), not a
    // bug. Downstream callers using sections for filtering should treat
    // the overlap inclusively.
    let mut run_start_idx = 0;
    for i in 1..=labels.len() {
        if i == labels.len() || labels[i] != labels[run_start_idx] {
            let start_frame = window_starts[run_start_idx];
            let end_frame = window_starts[i - 1] + window_frames;
            sections.push(TrackSection {
                start_seconds: start_frame as f32 / frames_per_second,
                end_seconds: end_frame as f32 / frames_per_second,
                kind: labels[run_start_idx],
                kick_band_rms: mean(&window_kick[run_start_idx..i]),
                broadband_rms: mean(&window_broad[run_start_idx..i]),
            });
            run_start_idx = i;
        }
    }
    sections
}

fn mean(xs: &[f32]) -> f32 {
    if xs.is_empty() {
        0.0
    } else {
        xs.iter().sum::<f32>() / xs.len() as f32
    }
}

/// Repeatedly merges any section shorter than `min_length_seconds` into its
/// longer neighbour. Stops when no short section remains, or when only one
/// section is left.
fn merge_short_sections(sections: &mut Vec<TrackSection>, min_length_seconds: f32) {
    while sections.len() > 1 {
        let short_idx = sections
            .iter()
            .enumerate()
            .find(|(_, s)| s.end_seconds - s.start_seconds < min_length_seconds)
            .map(|(i, _)| i);
        let Some(idx) = short_idx else { break };

        // Merge with the longer neighbour. If only one neighbour exists
        // (boundary), use it.
        let merge_into = match (idx == 0, idx == sections.len() - 1) {
            (true, _) => idx + 1,
            (_, true) => idx - 1,
            _ => {
                let prev_len = sections[idx - 1].end_seconds - sections[idx - 1].start_seconds;
                let next_len = sections[idx + 1].end_seconds - sections[idx + 1].start_seconds;
                if prev_len >= next_len {
                    idx - 1
                } else {
                    idx + 1
                }
            }
        };
        let removed = sections.remove(idx);
        let target_idx = if merge_into > idx {
            merge_into - 1
        } else {
            merge_into
        };
        let target = &mut sections[target_idx];
        // Extend the target to absorb the removed section's time range.
        // Energy and kick density stay the target's (the more representative).
        target.start_seconds = target.start_seconds.min(removed.start_seconds);
        target.end_seconds = target.end_seconds.max(removed.end_seconds);
    }
}

/// Collapses adjacent same-kind sections into a single section. Energy
/// and kick stats are recomputed as duration-weighted means so the
/// merged section represents the union faithfully.
fn collapse_adjacent_same_kind(sections: &mut Vec<TrackSection>) {
    if sections.len() < 2 {
        return;
    }
    let mut out: Vec<TrackSection> = Vec::with_capacity(sections.len());
    for s in sections.drain(..) {
        match out.last_mut() {
            Some(prev) if prev.kind == s.kind => {
                let prev_dur = (prev.end_seconds - prev.start_seconds).max(0.0);
                let s_dur = (s.end_seconds - s.start_seconds).max(0.0);
                let total = prev_dur + s_dur;
                if total > 0.0 {
                    prev.kick_band_rms =
                        (prev.kick_band_rms * prev_dur + s.kick_band_rms * s_dur) / total;
                    prev.broadband_rms =
                        (prev.broadband_rms * prev_dur + s.broadband_rms * s_dur) / total;
                }
                prev.start_seconds = prev.start_seconds.min(s.start_seconds);
                prev.end_seconds = prev.end_seconds.max(s.end_seconds);
            }
            _ => out.push(s),
        }
    }
    *sections = out;
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44100;
    const FRAME: usize = 2048;
    const HOP: usize = 512;
    const FRAMES_PER_SECOND: f32 = SR as f32 / HOP as f32;

    /// Build a fake spectrogram with controllable per-frame energy +
    /// kick presence. `regions[i] = (n_seconds, energy, has_kicks)`.
    ///
    /// Mid-band bins (above the kick range) carry `energy` so the RMS
    /// level reflects the region. Kick-band bins are zero by default;
    /// every 0.5 s a kick is "fired" by setting the kick-band bins to a
    /// loud value for one frame, producing a real spectral-flux event
    /// `detect_band_onsets` will pick up.
    fn synth_spec(regions: &[(f32, f32, bool)]) -> Vec<Vec<f32>> {
        let n_bins = FRAME / 2 + 1;
        let kick_bin_low = (40.0 * FRAME as f32 / SR as f32) as usize;
        let kick_bin_high = (120.0 * FRAME as f32 / SR as f32) as usize;
        let mut spec = Vec::new();
        let mut frame_idx = 0;
        for &(secs, energy, has_kicks) in regions {
            let n_frames = (secs * FRAMES_PER_SECOND) as usize;
            for _ in 0..n_frames {
                // Default: zero in kick band, `energy` everywhere above.
                let mut frame = vec![0.0_f32; n_bins];
                for slot in frame.iter_mut().skip(kick_bin_high + 1) {
                    *slot = energy;
                }
                if has_kicks {
                    let kick_period_frames = (0.5 * FRAMES_PER_SECOND) as usize;
                    if frame_idx % kick_period_frames == 0 {
                        // Loud kick: spike the kick band to 10 × energy
                        // (well above the per-frame max from the mid
                        // band, so band normalisation doesn't flatten it).
                        for slot in frame.iter_mut().take(kick_bin_high + 1).skip(kick_bin_low) {
                            *slot = energy * 10.0;
                        }
                    }
                }
                spec.push(frame);
                frame_idx += 1;
            }
        }
        spec
    }

    #[test]
    fn empty_spectrogram_returns_empty() {
        let sections = detect_track_sections(&[], SR, FRAME, HOP).unwrap();
        assert!(sections.is_empty());
    }

    #[test]
    fn very_short_track_returns_single_main_groove() {
        // 2 seconds of audio — below the SECTION_WINDOW_SECONDS = 4s minimum.
        let spec = synth_spec(&[(2.0, 1.0, true)]);
        let sections = detect_track_sections(&spec, SR, FRAME, HOP).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, SectionKind::MainGroove);
        assert!(sections[0].end_seconds > 1.5 && sections[0].end_seconds <= 2.1);
    }

    #[test]
    fn intro_then_main_then_breakdown_then_main_then_outro() {
        // Build a synthetic track structured like a real techno track.
        // 10s low-energy intro, 30s main, 10s breakdown (loud but no kicks),
        // 30s main, 10s low-energy outro = 90s total.
        let spec = synth_spec(&[
            (10.0, 0.05, false), // intro
            (30.0, 1.0, true),   // main
            (10.0, 1.0, false),  // breakdown — loud, no kicks
            (30.0, 1.0, true),   // main
            (10.0, 0.05, false), // outro
        ]);
        let sections = detect_track_sections(&spec, SR, FRAME, HOP).unwrap();

        // The order of section kinds should match the input.
        let kinds: Vec<SectionKind> = sections.iter().map(|s| s.kind).collect();
        assert_eq!(
            kinds,
            vec![
                SectionKind::Intro,
                SectionKind::MainGroove,
                SectionKind::Breakdown,
                SectionKind::MainGroove,
                SectionKind::Outro,
            ],
            "got sections: {:#?}",
            sections,
        );

        // Boundaries should be near the synthesized boundaries. Tolerance is
        // generous because the 4 s window blurs transitions and the merge
        // step can push boundaries by half a window in either direction.
        for (got, want) in sections
            .iter()
            .take(4)
            .map(|s| s.end_seconds)
            .zip([10.0, 40.0, 50.0, 80.0])
        {
            assert!(
                (got - want).abs() < 6.0,
                "boundary {got:.1}s expected near {want:.1}s; sections={sections:#?}"
            );
        }
    }

    #[test]
    fn merge_short_sections_collapses_sub_min_length_runs() {
        // Synthesize a track with a 2-second "fake breakdown" in the middle
        // of an otherwise-uninterrupted main groove. The 2s breakdown is
        // shorter than SECTION_MIN_LENGTH_SECONDS = 8s, so it should be
        // merged into the surrounding main groove.
        let spec = synth_spec(&[
            (30.0, 1.0, true), // main
            (2.0, 1.0, false), // 2s blip — too short to be a real breakdown
            (30.0, 1.0, true), // main
        ]);
        let sections = detect_track_sections(&spec, SR, FRAME, HOP).unwrap();
        // The 2s blip should be absorbed.
        assert!(
            sections.iter().all(|s| s.kind != SectionKind::Breakdown),
            "expected no Breakdown after merge, got: {:#?}",
            sections,
        );
    }

    #[test]
    fn classify_window_routes_correctly() {
        // High E + high K → MainGroove regardless of position.
        assert_eq!(classify_window(true, true, 0.0), SectionKind::MainGroove);
        assert_eq!(classify_window(true, true, 0.95), SectionKind::MainGroove);
        // High E + low K → Breakdown regardless of position.
        assert_eq!(classify_window(true, false, 0.5), SectionKind::Breakdown);
        // Low E early → Intro.
        assert_eq!(classify_window(false, false, 0.05), SectionKind::Intro);
        // Low E late → Outro.
        assert_eq!(classify_window(false, false, 0.95), SectionKind::Outro);
        // Low E mid-track → Breakdown (kickless quiet bridge).
        assert_eq!(classify_window(false, false, 0.5), SectionKind::Breakdown);
    }

    #[test]
    fn merge_short_sections_handles_first_and_last_position() {
        // Section list: [short Main, long Breakdown, short Main]. Both ends
        // are short and should merge inward.
        let mut sections = vec![
            TrackSection {
                start_seconds: 0.0,
                end_seconds: 2.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 1.5,
                broadband_rms: 1.0,
            },
            TrackSection {
                start_seconds: 2.0,
                end_seconds: 30.0,
                kind: SectionKind::Breakdown,
                kick_band_rms: 0.0,
                broadband_rms: 1.0,
            },
            TrackSection {
                start_seconds: 30.0,
                end_seconds: 32.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 1.5,
                broadband_rms: 1.0,
            },
        ];
        merge_short_sections(&mut sections, 8.0);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, SectionKind::Breakdown);
        assert_eq!(sections[0].start_seconds, 0.0);
        assert_eq!(sections[0].end_seconds, 32.0);
    }

    #[test]
    fn collapse_adjacent_same_kind_merges_post_absorption_pairs() {
        // Recovery IDea-style structure: long Main, short Breakdown,
        // long Main. After short-section absorption the Breakdown is
        // gone but two adjacent Main sections remain — collapse should
        // fuse them.
        let mut sections = vec![
            TrackSection {
                start_seconds: 0.0,
                end_seconds: 30.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 1.0,
                broadband_rms: 1.0,
            },
            TrackSection {
                start_seconds: 30.0,
                end_seconds: 60.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 2.0,
                broadband_rms: 2.0,
            },
        ];
        collapse_adjacent_same_kind(&mut sections);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].kind, SectionKind::MainGroove);
        assert_eq!(sections[0].start_seconds, 0.0);
        assert_eq!(sections[0].end_seconds, 60.0);
        // Stats are duration-weighted means; both sections were 30 s, so
        // the merge of (1.0, 2.0) is (1.5, 1.5).
        assert!((sections[0].kick_band_rms - 1.5).abs() < 1e-5);
        assert!((sections[0].broadband_rms - 1.5).abs() < 1e-5);
    }

    #[test]
    fn collapse_adjacent_same_kind_leaves_alternating_pattern() {
        let mut sections = vec![
            TrackSection {
                start_seconds: 0.0,
                end_seconds: 30.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 1.0,
                broadband_rms: 1.0,
            },
            TrackSection {
                start_seconds: 30.0,
                end_seconds: 50.0,
                kind: SectionKind::Breakdown,
                kick_band_rms: 0.0,
                broadband_rms: 1.0,
            },
            TrackSection {
                start_seconds: 50.0,
                end_seconds: 80.0,
                kind: SectionKind::MainGroove,
                kick_band_rms: 1.0,
                broadband_rms: 1.0,
            },
        ];
        collapse_adjacent_same_kind(&mut sections);
        assert_eq!(sections.len(), 3);
    }
}
