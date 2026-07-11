//! Kick-pattern detector for distinguishing straight 4/4, broken-beat, and
//! halftime kick placement.
//!
//! The detector intentionally stops at feature extraction. Genre-classifier
//! rules should consume this only after real-track validation.

use crate::analysis::result::{BeatGrid, KickPattern, KickPatternAnalysis, RateBasis};
use crate::config::AnalysisConfig;
use crate::error::AnalysisError;
use crate::features::onset::band::detect_band_onsets;
use crate::features::sections::{SectionKind, TrackSection};
use std::collections::BTreeSet;

/// Beat positions per bar in the kick histogram.
pub const KICK_PATTERN_ROWS: usize = 4;

/// Sixteenth-subdivision bins inside each beat.
pub const KICK_PATTERN_COLS: usize = 16;

const HISTOGRAM_LEN: usize = KICK_PATTERN_ROWS * KICK_PATTERN_COLS;
const BIN_SIGMA: f32 = 0.75;

#[derive(Debug, Clone)]
struct KickHistogram {
    bins: [f32; HISTOGRAM_LEN],
    bar_count: f32,
    /// Beat-level kick candidates after collapsing dense low-band subdivision
    /// activity. The detailed histogram still keeps subdivision placement for
    /// template scoring, but density should describe musical kick anchors.
    event_count: usize,
}

/// Detect kick-band onsets and classify their beat-relative placement.
///
/// When `sections` contains at least one `MainGroove` section, onsets and bar
/// counts are limited to those sections. Otherwise the detector uses the full
/// analysed track and reports `RateBasis::Track`.
pub fn detect_kick_pattern(
    spec: &[Vec<f32>],
    sample_rate: u32,
    beat_grid: &BeatGrid,
    bpm: f32,
    config: &AnalysisConfig,
    sections: Option<&[TrackSection]>,
) -> Result<KickPatternAnalysis, AnalysisError> {
    let frame_size = config.frame_size;
    let hop_size = config.hop_size;

    if hop_size == 0 || sample_rate == 0 {
        return Err(AnalysisError::InvalidInput(
            "hop_size and sample_rate must be > 0".to_string(),
        ));
    }

    beat_grid.validate()?;

    let onsets = detect_band_onsets(
        spec,
        sample_rate,
        frame_size,
        (
            config.kick_pattern_band_low_hz,
            config.kick_pattern_band_high_hz,
        ),
        config.kick_pattern_onset_threshold_percentile,
    )?;

    let main_groove_ranges: Vec<(f32, f32)> = sections
        .map(|secs| {
            secs.iter()
                .filter(|s| s.kind == SectionKind::MainGroove)
                .map(|s| (s.start_seconds, s.end_seconds))
                .collect()
        })
        .unwrap_or_default();
    let rate_basis = if main_groove_ranges.is_empty() {
        RateBasis::Track
    } else {
        RateBasis::MainGroove
    };

    let frames_per_second = sample_rate as f32 / hop_size as f32;
    let filtered_onsets: Vec<usize> = if main_groove_ranges.is_empty() {
        onsets
    } else {
        onsets
            .into_iter()
            .filter(|&frame| {
                let t = frame as f32 / frames_per_second;
                main_groove_ranges
                    .iter()
                    .any(|&(start, end)| t >= start && t < end)
            })
            .collect()
    };

    let histogram = kick_histogram_from_onsets(
        &filtered_onsets,
        hop_size,
        sample_rate,
        beat_grid,
        &main_groove_ranges,
    )?;
    let kicks_per_bar = capped_kicks_per_bar(histogram.event_count, histogram.bar_count);
    let (pattern, confidence) = classify_kick_histogram(
        &histogram.bins,
        kicks_per_bar,
        bpm,
        config.kick_pattern_sparse_threshold,
        config.kick_pattern_min_template_score,
        config.kick_pattern_halftime_min_bpm,
    );

    Ok(KickPatternAnalysis {
        pattern,
        confidence,
        kicks_per_bar,
        onset_count: histogram.event_count as u32,
        histogram: histogram.bins.to_vec(),
        rate_basis,
    })
}

fn kick_histogram_from_onsets(
    onset_frames: &[usize],
    hop_size: usize,
    sample_rate: u32,
    beat_grid: &BeatGrid,
    ranges: &[(f32, f32)],
) -> Result<KickHistogram, AnalysisError> {
    if beat_grid.beats.len() < 2 {
        return Ok(KickHistogram {
            bins: [0.0; HISTOGRAM_LEN],
            bar_count: 0.0,
            event_count: 0,
        });
    }

    let mut bins = [0.0_f32; HISTOGRAM_LEN];
    let mut occupied_cells = BTreeSet::new();
    let mut occupied_beats = BTreeSet::new();
    let sr = sample_rate as f32;
    let beats = &beat_grid.beats;
    let bars = &beat_grid.bars;

    for &frame in onset_frames {
        let onset_time = frame as f32 * hop_size as f32 / sr;
        let mut beat_idx = match beats
            .binary_search_by(|b| b.partial_cmp(&onset_time).expect("validated finite"))
        {
            Ok(i) => i,
            Err(0) => continue,
            Err(i) => i - 1,
        };
        if beat_idx + 1 >= beats.len() {
            continue;
        }

        let beat_period = beats[beat_idx + 1] - beats[beat_idx];
        if beat_period <= 0.0 || !beat_period.is_finite() {
            continue;
        }

        let mut offset = ((onset_time - beats[beat_idx]) / beat_period).clamp(0.0, 1.0);
        // STFT frame quantisation can put an exactly-on-beat transient a few
        // milliseconds before the beat, which otherwise bins as a late
        // previous-beat event. Snap only the extreme tail; genuine 16th-note
        // anticipations remain well below this threshold.
        let mut placement_time = onset_time;
        if offset > 0.90 && beat_idx + 2 < beats.len() {
            beat_idx += 1;
            offset = 0.0;
            placement_time = beats[beat_idx];
        }

        let beat_in_bar = beat_in_bar(beat_idx, placement_time, beats, bars);
        let bar_idx = bar_index(beat_idx, placement_time, bars);
        let col = quantized_col(offset);
        occupied_cells.insert((bar_idx, beat_in_bar, col));
        occupied_beats.insert((bar_idx, beat_in_bar));
    }

    for &(_, beat_in_bar, col) in &occupied_cells {
        add_soft_bin_at_col(&mut bins, beat_in_bar, col);
    }

    let bar_count = count_bars(beat_grid, ranges);
    Ok(KickHistogram {
        bins,
        bar_count,
        event_count: occupied_beats.len(),
    })
}

fn add_soft_bin_at_col(bins: &mut [f32; HISTOGRAM_LEN], beat_in_bar: usize, col: usize) {
    let target = col as f32;
    let two_sigma_sq = 2.0 * BIN_SIGMA * BIN_SIGMA;
    for target_col in 0..KICK_PATTERN_COLS {
        let mut d = (target_col as f32 - target).abs();
        if d > KICK_PATTERN_COLS as f32 / 2.0 {
            d = KICK_PATTERN_COLS as f32 - d;
        }
        bins[beat_in_bar * KICK_PATTERN_COLS + target_col] += (-(d * d) / two_sigma_sq).exp();
    }
}

fn quantized_col(offset: f32) -> usize {
    ((offset * KICK_PATTERN_COLS as f32).round() as usize) % KICK_PATTERN_COLS
}

fn beat_in_bar(beat_idx: usize, onset_time: f32, beats: &[f32], bars: &[f32]) -> usize {
    if bars.is_empty() || onset_time < bars[0] {
        return beat_idx % KICK_PATTERN_ROWS;
    }
    let bar_pos = bars.partition_point(|&b| b <= onset_time);
    if bar_pos == 0 {
        return beat_idx % KICK_PATTERN_ROWS;
    }
    let bar_start = bars[bar_pos - 1];
    let first_beat_in_bar = beats.partition_point(|&b| b < bar_start);
    beat_idx
        .saturating_sub(first_beat_in_bar)
        .min(KICK_PATTERN_ROWS - 1)
}

fn bar_index(beat_idx: usize, onset_time: f32, bars: &[f32]) -> usize {
    if bars.is_empty() || onset_time < bars[0] {
        return beat_idx / KICK_PATTERN_ROWS;
    }
    let bar_pos = bars.partition_point(|&b| b <= onset_time);
    if bar_pos == 0 {
        beat_idx / KICK_PATTERN_ROWS
    } else {
        bar_pos - 1
    }
}

fn count_bars(beat_grid: &BeatGrid, ranges: &[(f32, f32)]) -> f32 {
    if beat_grid.bars.is_empty() {
        return (beat_grid.beats.len() as f32 / KICK_PATTERN_ROWS as f32).max(0.0);
    }
    if ranges.is_empty() {
        return beat_grid.bars.len() as f32;
    }
    let count = beat_grid
        .bars
        .iter()
        .enumerate()
        .filter(|(idx, &bar_start)| {
            let bar_end = beat_grid
                .bars
                .get(idx + 1)
                .copied()
                .or_else(|| beat_grid.beats.last().copied())
                .unwrap_or(bar_start);
            ranges
                .iter()
                .any(|&(start, end)| bar_end > start && bar_start < end)
        })
        .count();
    count as f32
}

fn capped_kicks_per_bar(event_count: usize, bar_count: f32) -> f32 {
    if bar_count > 0.0 {
        (event_count as f32 / bar_count).min(KICK_PATTERN_ROWS as f32)
    } else {
        0.0
    }
}

fn classify_kick_histogram(
    observed: &[f32; HISTOGRAM_LEN],
    kicks_per_bar: f32,
    bpm: f32,
    sparse_threshold: f32,
    min_template_score: f32,
    halftime_min_bpm: f32,
) -> (KickPattern, f32) {
    if kicks_per_bar < sparse_threshold {
        let confidence = if sparse_threshold > 0.0 {
            1.0 - (kicks_per_bar / sparse_threshold).clamp(0.0, 1.0)
        } else {
            1.0
        };
        return (KickPattern::Sparse, confidence.clamp(0.5, 1.0));
    }

    let total_mass: f32 = observed.iter().sum();
    if total_mass <= 0.0 || !total_mass.is_finite() {
        return (KickPattern::Sparse, 1.0);
    }

    let templates = [
        (
            KickPattern::FourOnFloor,
            template(&[(0, 0, 1.0), (1, 0, 1.0), (2, 0, 1.0), (3, 0, 1.0)]),
            4.0,
        ),
        (
            KickPattern::BrokenBeat,
            template(&[(0, 0, 1.0), (1, 8, 1.0), (2, 0, 1.0), (3, 8, 1.0)]),
            4.0,
        ),
        (
            KickPattern::BrokenBeat,
            template(&[(0, 0, 1.0), (1, 3, 1.0), (2, 0, 1.0), (3, 3, 1.0)]),
            4.0,
        ),
        (
            KickPattern::BrokenBeat,
            template(&[(0, 0, 1.0), (1, 4, 1.0), (2, 0, 1.0), (3, 4, 1.0)]),
            4.0,
        ),
        (
            KickPattern::Halftime,
            template(&[(0, 0, 1.0), (2, 0, 1.0)]),
            2.0,
        ),
    ];

    let mut best = (KickPattern::Irregular, 0.0_f32, 0.0_f32);
    let mut best_non_halftime = (KickPattern::Irregular, 0.0_f32, 0.0_f32);
    for (pattern, tmpl, expected_kicks_per_bar) in templates {
        let score = cosine(observed, &tmpl);
        let density = (kicks_per_bar / expected_kicks_per_bar)
            .clamp(0.0, 1.0)
            .sqrt();
        let confidence = score * density;
        if score > best.1 {
            best = (pattern, score, confidence);
        }
        if pattern != KickPattern::Halftime && score > best_non_halftime.1 {
            best_non_halftime = (pattern, score, confidence);
        }
    }

    let offbeat_mass = observed[idx(1, 8)] + observed[idx(3, 8)];
    let offbeat_ratio = offbeat_mass / total_mass;
    if best.0 == KickPattern::FourOnFloor && offbeat_ratio > 0.30 {
        best = (
            KickPattern::BrokenBeat,
            best.1.max(offbeat_ratio),
            best.2.max(offbeat_ratio),
        );
    }

    if best.0 == KickPattern::Halftime && kicks_per_bar > 3.0 {
        best = if best_non_halftime.1 >= min_template_score {
            best_non_halftime
        } else {
            (
                KickPattern::Irregular,
                best.1,
                (1.0 - best.1).clamp(0.4, 1.0),
            )
        };
    }

    if best.1 < min_template_score {
        return (KickPattern::Irregular, (1.0 - best.1).clamp(0.4, 1.0));
    }

    if best.0 == KickPattern::Halftime && bpm < halftime_min_bpm {
        return (KickPattern::FourOnFloor, (best.2 * 0.7).clamp(0.0, 1.0));
    }

    (best.0, best.2.clamp(0.0, 1.0))
}

fn template(points: &[(usize, usize, f32)]) -> [f32; HISTOGRAM_LEN] {
    let mut out = [0.0_f32; HISTOGRAM_LEN];
    for &(row, col, weight) in points {
        if row < KICK_PATTERN_ROWS && col < KICK_PATTERN_COLS {
            out[idx(row, col)] = weight;
        }
    }
    out
}

fn cosine(a: &[f32; HISTOGRAM_LEN], b: &[f32; HISTOGRAM_LEN]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 || !norm_a.is_finite() || !norm_b.is_finite() {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

const fn idx(row: usize, col: usize) -> usize {
    row * KICK_PATTERN_COLS + col
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;
    const HOP: usize = 512;

    fn beat_grid_4on4(bpm: f32, bars: usize) -> BeatGrid {
        let beat_period = 60.0 / bpm;
        let total_beats = bars * 4 + 1;
        let beats: Vec<f32> = (0..total_beats).map(|i| i as f32 * beat_period).collect();
        let bar_starts: Vec<f32> = (0..bars)
            .map(|bar| bar as f32 * beat_period * 4.0)
            .collect();
        BeatGrid {
            downbeats: bar_starts.clone(),
            beats,
            bars: bar_starts,
        }
    }

    fn frame(t: f32) -> usize {
        (t * SR as f32 / HOP as f32).round() as usize
    }

    fn analyze_times(times: &[f32], bpm: f32, bars: usize) -> KickPatternAnalysis {
        let grid = beat_grid_4on4(bpm, bars);
        let onset_frames: Vec<usize> = times.iter().copied().map(frame).collect();
        let hist = kick_histogram_from_onsets(&onset_frames, HOP, SR, &grid, &[]).unwrap();
        let kicks_per_bar = capped_kicks_per_bar(hist.event_count, hist.bar_count);
        let (pattern, confidence) =
            classify_kick_histogram(&hist.bins, kicks_per_bar, bpm, 0.5, 0.4, 100.0);
        KickPatternAnalysis {
            pattern,
            confidence,
            kicks_per_bar,
            onset_count: onset_frames.len() as u32,
            histogram: hist.bins.to_vec(),
            rate_basis: RateBasis::Track,
        }
    }

    fn repeated_pattern(bpm: f32, bars: usize, offsets: &[f32]) -> Vec<f32> {
        let beat_period = 60.0 / bpm;
        let bar_period = beat_period * 4.0;
        let mut times = Vec::new();
        for bar in 0..bars {
            let bar_start = bar as f32 * bar_period;
            for &offset_beats in offsets {
                times.push(bar_start + offset_beats * beat_period);
            }
        }
        times
    }

    #[test]
    fn classifies_four_on_floor() {
        let times = repeated_pattern(128.0, 8, &[0.0, 1.0, 2.0, 3.0]);
        let result = analyze_times(&times, 128.0, 8);
        assert_eq!(result.pattern, KickPattern::FourOnFloor);
        assert!(result.confidence > 0.6);
        assert!((result.kicks_per_bar - 4.0).abs() < 0.01);
    }

    #[test]
    fn duplicate_onsets_in_same_metric_cell_count_once() {
        let bpm = 128.0;
        let grid = beat_grid_4on4(bpm, 4);
        let beat_period = 60.0 / bpm;
        let mut times = Vec::new();
        for bar in 0..4 {
            let bar_start = bar as f32 * beat_period * 4.0;
            for beat in 0..4 {
                let t = bar_start + beat as f32 * beat_period;
                times.extend([t, t + 0.004, t + 0.008]);
            }
        }
        let onset_frames: Vec<usize> = times.iter().copied().map(frame).collect();
        let hist = kick_histogram_from_onsets(&onset_frames, HOP, SR, &grid, &[]).unwrap();
        assert_eq!(hist.event_count, 16);
        assert_eq!(hist.bar_count, 4.0);
    }

    #[test]
    fn subdivision_activity_counts_once_per_beat() {
        let bpm = 128.0;
        let grid = beat_grid_4on4(bpm, 2);
        let beat_period = 60.0 / bpm;
        let mut times = Vec::new();
        for bar in 0..2 {
            let bar_start = bar as f32 * beat_period * 4.0;
            for beat in 0..4 {
                let beat_start = bar_start + beat as f32 * beat_period;
                times.extend([
                    beat_start,
                    beat_start + 0.25 * beat_period,
                    beat_start + 0.50 * beat_period,
                    beat_start + 0.75 * beat_period,
                ]);
            }
        }
        let onset_frames: Vec<usize> = times.iter().copied().map(frame).collect();
        let hist = kick_histogram_from_onsets(&onset_frames, HOP, SR, &grid, &[]).unwrap();
        assert_eq!(hist.event_count, 8);
        assert_eq!(hist.bar_count, 2.0);
    }

    #[test]
    fn classifies_broken_beat() {
        let times = repeated_pattern(128.0, 8, &[0.0, 1.5, 2.0, 3.5]);
        let result = analyze_times(&times, 128.0, 8);
        assert_eq!(result.pattern, KickPattern::BrokenBeat);
        assert!(result.confidence > 0.6);
    }

    #[test]
    fn classifies_early_syncopated_breakbeat() {
        let times = repeated_pattern(123.0, 8, &[0.0, 1.25, 2.0, 3.25]);
        let result = analyze_times(&times, 123.0, 8);
        assert_eq!(result.pattern, KickPattern::BrokenBeat);
        assert!(result.confidence > 0.6);
        assert!((result.kicks_per_bar - 4.0).abs() < 0.01);
    }

    #[test]
    fn classifies_halftime_above_minimum_bpm() {
        let times = repeated_pattern(140.0, 8, &[0.0, 2.0]);
        let result = analyze_times(&times, 140.0, 8);
        assert_eq!(result.pattern, KickPattern::Halftime);
        assert!(result.confidence > 0.6);
    }

    #[test]
    fn dense_syncopation_blocks_halftime_label() {
        let times = repeated_pattern(140.0, 8, &[0.0, 1.75, 2.0, 3.75]);
        let result = analyze_times(&times, 140.0, 8);
        assert_ne!(result.pattern, KickPattern::Halftime);
        assert!((result.kicks_per_bar - 4.0).abs() < 0.01);
    }

    #[test]
    fn collapses_low_bpm_halftime_to_four_on_floor() {
        let times = repeated_pattern(90.0, 8, &[0.0, 2.0]);
        let result = analyze_times(&times, 90.0, 8);
        assert_eq!(result.pattern, KickPattern::FourOnFloor);
        assert!(result.confidence < 0.8);
    }

    #[test]
    fn classifies_sparse() {
        let times = repeated_pattern(128.0, 8, &[0.0]);
        let result = analyze_times(&times[..2], 128.0, 8);
        assert_eq!(result.pattern, KickPattern::Sparse);
        assert!(result.confidence >= 0.5);
    }

    #[test]
    fn classifies_irregular_when_no_template_fits() {
        let times = repeated_pattern(128.0, 8, &[0.25, 1.75, 2.25, 3.5]);
        let result = analyze_times(&times, 128.0, 8);
        assert_eq!(result.pattern, KickPattern::Irregular);
        assert!(result.confidence >= 0.4);
    }

    #[test]
    fn limits_onsets_to_main_groove_sections() {
        let bpm = 128.0;
        let beat_period = 60.0 / bpm;
        let grid = beat_grid_4on4(bpm, 8);
        let times = repeated_pattern(bpm, 8, &[0.0, 1.0, 2.0, 3.0]);
        let onset_frames: Vec<usize> = times.iter().copied().map(frame).collect();
        let ranges = [(2.0 * 4.0 * beat_period, 4.0 * 4.0 * beat_period)];
        let hist = kick_histogram_from_onsets(&onset_frames, HOP, SR, &grid, &ranges).unwrap();
        assert_eq!(hist.bar_count, 2.0);
    }

    #[test]
    fn counts_bars_that_overlap_section_ranges() {
        let bpm = 120.0;
        let beat_period = 60.0 / bpm;
        let bar_period = beat_period * 4.0;
        let grid = beat_grid_4on4(bpm, 4);
        let ranges = [(0.25 * bar_period, 2.25 * bar_period)];
        assert_eq!(count_bars(&grid, &ranges), 3.0);
    }

    #[test]
    fn caps_reported_density_at_four_beat_anchors() {
        assert_eq!(capped_kicks_per_bar(401, 100.0), 4.0);
        assert_eq!(capped_kicks_per_bar(0, 0.0), 0.0);
    }
}
