//! Chord-stab detector for Dub Techno.
//!
//! Currently implements Stages 1 and 2 of the four-stage pipeline:
//!
//! - Stage 1 — mid-band onset detection with kick-coincidence masking.
//! - Stage 2 — beat-relative offset histogram (soft-binned, per-bar).
//!
//! Stages 3 (template scoring) and 4 (decay confirmation) plus the final
//! score combination are tracked for a follow-up PR. No `AnalysisResult`
//! integration yet — this module is internal.
//!
//! Design rationale and parameter justification:
//! `docs/tmp/chord-stab-detector-plan.md`,
//! `docs/tmp/kick-bleed-investigation.md`.

use crate::analysis::result::BeatGrid;
use crate::error::AnalysisError;
use crate::features::onset::band::detect_band_onsets;

/// Bin count for the beat-relative offset histogram. 32 bins gives sub-16th
/// resolution suitable for soft-binning realistic micro-timing drift.
pub const HISTOGRAM_BINS: usize = 32;

/// Soft-binning Gaussian σ in fractional-beat units. Half a bin: each onset
/// spreads its weight over ~3 neighbouring bins instead of snapping hard to
/// one, absorbing micro-timing without smearing across the histogram.
const SOFT_BIN_SIGMA: f32 = 0.5 / HISTOGRAM_BINS as f32;

/// Configuration for the dub-stab detector.
///
/// `kick_mask_window_ms = 80.0` and the symmetric mask shape are derived
/// from the 5-track real-audio cross-validation in
/// `docs/tmp/kick-bleed-investigation.md` (Maurizio, cv313, Deepchord,
/// Monolake, Rhythm & Sound — mean pre/post bleed ratio ≈ 1.0).
///
/// Other defaults (band edges, kick band, percentile) come from
/// `docs/tmp/chord-stab-detector-plan.md` and the project-wide
/// spectral-flux onset conventions.
#[derive(Debug, Clone)]
pub struct DubStabConfig {
    /// Lower edge of the stab band in Hz.
    pub band_low_hz: f32,
    /// Upper edge of the stab band in Hz.
    pub band_high_hz: f32,
    /// Lower edge of the kick band used for the coincidence mask.
    pub kick_band_low_hz: f32,
    /// Upper edge of the kick band used for the coincidence mask.
    pub kick_band_high_hz: f32,
    /// Symmetric mask half-width in milliseconds. Stab onsets within ±this
    /// many ms of any kick onset are dropped.
    pub kick_mask_window_ms: f32,
    /// Percentile threshold passed through to [`detect_band_onsets`].
    pub onset_threshold_percentile: f32,
}

impl Default for DubStabConfig {
    fn default() -> Self {
        Self {
            band_low_hz: 350.0,
            band_high_hz: 2000.0,
            kick_band_low_hz: 40.0,
            kick_band_high_hz: 120.0,
            kick_mask_window_ms: 80.0,
            onset_threshold_percentile: 0.85,
        }
    }
}

/// Beat-relative offset histogram with global and per-bar views.
///
/// The fixed-size arrays encode `HISTOGRAM_BINS` at the type level so
/// downstream template scoring (Stage 3+) cannot accidentally mismatch
/// lengths.
#[derive(Debug, Clone)]
pub struct OffsetHistogram {
    /// Global histogram aggregated across the whole track.
    pub global: [f32; HISTOGRAM_BINS],
    /// One sub-histogram per bar in the supplied beat grid.
    pub per_bar: Vec<[f32; HISTOGRAM_BINS]>,
}

/// Stage 1 — detect stab-band onsets and remove those that fall within the
/// configured ±window of any kick-band onset.
///
/// Reuses the existing [`detect_band_onsets`] primitive twice: once for the
/// stab band, once for the kick band. Returns frame indices of stab-band
/// onsets that survive the mask.
pub fn detect_kick_disjoint_stab_onsets(
    spec: &[Vec<f32>],
    sample_rate: u32,
    frame_size: usize,
    hop_size: usize,
    config: &DubStabConfig,
) -> Result<Vec<usize>, AnalysisError> {
    if !config.kick_mask_window_ms.is_finite() || config.kick_mask_window_ms < 0.0 {
        return Err(AnalysisError::InvalidInput(format!(
            "kick_mask_window_ms must be finite and ≥ 0, got {}",
            config.kick_mask_window_ms
        )));
    }
    if hop_size == 0 {
        return Err(AnalysisError::InvalidInput(
            "hop_size must be > 0".to_string(),
        ));
    }

    let stab = detect_band_onsets(
        spec,
        sample_rate,
        frame_size,
        (config.band_low_hz, config.band_high_hz),
        config.onset_threshold_percentile,
    )?;
    if stab.is_empty() {
        log::debug!("dub_stab Stage 1: no stab-band onsets detected; returning empty");
        return Ok(Vec::new());
    }

    let kicks = detect_band_onsets(
        spec,
        sample_rate,
        frame_size,
        (config.kick_band_low_hz, config.kick_band_high_hz),
        config.onset_threshold_percentile,
    )?;
    if kicks.is_empty() {
        log::debug!(
            "dub_stab Stage 1: {} stab onsets, no kick onsets — mask is a no-op",
            stab.len()
        );
        return Ok(stab);
    }

    let frames_per_second = sample_rate as f32 / hop_size as f32;
    let mask_frames = (config.kick_mask_window_ms / 1000.0 * frames_per_second).round() as usize;

    let kept = mask_kick_coincident(&stab, &kicks, mask_frames);

    if kept.is_empty() {
        log::debug!(
            "dub_stab Stage 1: all {} stab onsets masked by ±{} ms kick coincidence (mask_frames={})",
            stab.len(),
            config.kick_mask_window_ms,
            mask_frames
        );
    } else {
        log::debug!(
            "dub_stab Stage 1: {} of {} stab onsets survived ±{} ms kick mask (mask_frames={})",
            kept.len(),
            stab.len(),
            config.kick_mask_window_ms,
            mask_frames
        );
    }
    Ok(kept)
}

/// Drop stab onsets that fall within `mask_frames` of any kick onset.
///
/// Both inputs must be sorted ascending. The mask is symmetric and inclusive
/// at both edges: a stab at frame `s` is dropped iff some kick `k` exists
/// with `|s − k| ≤ mask_frames`.
pub fn mask_kick_coincident(
    stab_onsets: &[usize],
    kick_onsets_sorted: &[usize],
    mask_frames: usize,
) -> Vec<usize> {
    if kick_onsets_sorted.is_empty() {
        return stab_onsets.to_vec();
    }
    stab_onsets
        .iter()
        .copied()
        .filter(|&s| !kick_within(s, kick_onsets_sorted, mask_frames))
        .collect()
}

fn kick_within(stab_frame: usize, kicks_sorted: &[usize], window: usize) -> bool {
    let lo = stab_frame.saturating_sub(window);
    let hi = stab_frame.saturating_add(window);
    let idx = kicks_sorted.partition_point(|&k| k < lo);
    matches!(kicks_sorted.get(idx), Some(&k) if k <= hi)
}

/// Stage 2 — compute the beat-relative offset histogram for a list of onset
/// frames against the supplied beat grid.
///
/// Each onset is converted to seconds (matching the beat grid's hop-time
/// anchor — see the STFT centring section of `kick-bleed-investigation.md`),
/// placed into the `[0, 1)` interval relative to its surrounding beat period,
/// and accumulated into a 32-bin histogram with circular-Gaussian soft
/// binning.
///
/// Onsets before the first beat or after the last beat are discarded — no
/// beat period can be defined there.
///
/// `per_bar` is sized to the supplied `beat_grid.bars`. Onsets before
/// `bars[0]` are excluded from the per-bar accounting (their contribution
/// still lands in `global`). Onsets after the last bar boundary are
/// attributed to the **final bar**: `bars` carries no upper edge for the
/// last bar, so the function cannot symmetrically drop them.
///
/// # Errors
///
/// Returns `InvalidInput` if `beat_grid.beats` or `beat_grid.bars` contain
/// any non-finite value, or if either is not strictly ascending. A
/// non-monotonic / NaN-tainted grid is an upstream bug — silently absorbing
/// it would produce a NaN-corrupted or near-empty histogram with no
/// diagnostic.
pub fn beat_relative_offset_histogram(
    onset_frames: &[usize],
    hop_size: usize,
    sample_rate: u32,
    beat_grid: &BeatGrid,
) -> Result<OffsetHistogram, AnalysisError> {
    if hop_size == 0 || sample_rate == 0 {
        return Err(AnalysisError::InvalidInput(
            "hop_size and sample_rate must be > 0".to_string(),
        ));
    }
    validate_beat_grid(beat_grid)?;

    let mut global = [0.0_f32; HISTOGRAM_BINS];
    let mut per_bar: Vec<[f32; HISTOGRAM_BINS]> = vec![[0.0; HISTOGRAM_BINS]; beat_grid.bars.len()];

    if beat_grid.beats.len() < 2 {
        return Ok(OffsetHistogram { global, per_bar });
    }

    let two_sigma_sq = 2.0 * SOFT_BIN_SIGMA * SOFT_BIN_SIGMA;
    let beats = &beat_grid.beats;
    let bars = &beat_grid.bars;
    let sr = sample_rate as f32;

    for &frame in onset_frames {
        let onset_time = frame as f32 * hop_size as f32 / sr;

        // beats validated finite + strictly ascending: partial_cmp is total here.
        let beat_idx = match beats
            .binary_search_by(|b| b.partial_cmp(&onset_time).expect("validated finite"))
        {
            Ok(i) => i,
            Err(0) => continue, // before first beat
            Err(i) => i - 1,
        };
        if beat_idx + 1 >= beats.len() {
            continue; // after last beat — no period defined
        }
        let beat_period = beats[beat_idx + 1] - beats[beat_idx];
        debug_assert!(
            beat_period > 0.0,
            "beats validated strictly ascending; period must be positive"
        );

        let raw_offset = (onset_time - beats[beat_idx]) / beat_period;
        let offset = raw_offset.clamp(0.0, 1.0 - f32::EPSILON);

        let bar_idx = locate_bar(bars, onset_time);

        for bin in 0..HISTOGRAM_BINS {
            let centre = bin as f32 / HISTOGRAM_BINS as f32;
            let mut d = (centre - offset).abs();
            if d > 0.5 {
                d = 1.0 - d;
            }
            let w = (-(d * d) / two_sigma_sq).exp();
            global[bin] += w;
            if let Some(bar_hist) = bar_idx.and_then(|b| per_bar.get_mut(b)) {
                bar_hist[bin] += w;
            }
        }
    }

    Ok(OffsetHistogram { global, per_bar })
}

/// Validate that `beat_grid.beats` and `.bars` are finite and strictly
/// ascending. The downstream histogram code assumes both invariants: a
/// non-finite element makes the binary-search predicate non-total (silently
/// mis-bucketing onsets, or with NaN propagating into every histogram bin),
/// and a duplicate or non-monotonic element produces a wrong or zero beat
/// period.
fn validate_beat_grid(beat_grid: &BeatGrid) -> Result<(), AnalysisError> {
    fn check(name: &str, xs: &[f32]) -> Result<(), AnalysisError> {
        if let Some(idx) = xs.iter().position(|x| !x.is_finite()) {
            return Err(AnalysisError::InvalidInput(format!(
                "beat_grid.{name} must be finite, got {} at index {idx}",
                xs[idx]
            )));
        }
        if let Some(idx) = xs.windows(2).position(|w| w[0] >= w[1]) {
            return Err(AnalysisError::InvalidInput(format!(
                "beat_grid.{name} must be strictly ascending, got {name}[{idx}]={} >= {name}[{}]={}",
                xs[idx],
                idx + 1,
                xs[idx + 1]
            )));
        }
        Ok(())
    }
    check("beats", &beat_grid.beats)?;
    check("bars", &beat_grid.bars)?;
    Ok(())
}

fn locate_bar(bars: &[f32], onset_time: f32) -> Option<usize> {
    if bars.is_empty() || onset_time < bars[0] {
        return None;
    }
    let i = bars.partition_point(|&b| b <= onset_time);
    if i == 0 {
        None
    } else {
        Some(i - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;
    const FRAME: usize = 2048;
    const HOP: usize = 512;
    const N_BINS: usize = FRAME / 2 + 1;

    /// Build a synthetic 4/4 beat grid covering `duration_s` at `bpm`.
    fn beat_grid_4on4(bpm: f32, duration_s: f32) -> BeatGrid {
        let beat_period = 60.0 / bpm;
        let mut beats = Vec::new();
        let mut t = 0.0_f32;
        while t < duration_s {
            beats.push(t);
            t += beat_period;
        }
        let bars: Vec<f32> = beats.iter().step_by(4).copied().collect();
        BeatGrid {
            downbeats: bars.clone(),
            beats,
            bars,
        }
    }

    fn time_to_frame(t_s: f32) -> usize {
        (t_s * SR as f32 / HOP as f32).round() as usize
    }

    // ─── Stage 1 ────────────────────────────────────────────────────────────

    #[test]
    fn config_default_matches_investigation_findings() {
        let c = DubStabConfig::default();
        assert_eq!(c.band_low_hz, 350.0);
        assert_eq!(c.band_high_hz, 2000.0);
        assert_eq!(c.kick_band_low_hz, 40.0);
        assert_eq!(c.kick_band_high_hz, 120.0);
        assert_eq!(c.kick_mask_window_ms, 80.0);
        assert_eq!(c.onset_threshold_percentile, 0.85);
    }

    #[test]
    fn empty_spectrogram_returns_no_onsets() {
        let onsets =
            detect_kick_disjoint_stab_onsets(&[], SR, FRAME, HOP, &DubStabConfig::default())
                .unwrap();
        assert!(onsets.is_empty());
    }

    #[test]
    fn invalid_kick_mask_window_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        let neg = DubStabConfig {
            kick_mask_window_ms: -1.0,
            ..DubStabConfig::default()
        };
        assert!(detect_kick_disjoint_stab_onsets(&spec, SR, FRAME, HOP, &neg).is_err());
        let nan = DubStabConfig {
            kick_mask_window_ms: f32::NAN,
            ..DubStabConfig::default()
        };
        assert!(detect_kick_disjoint_stab_onsets(&spec, SR, FRAME, HOP, &nan).is_err());
    }

    #[test]
    fn zero_hop_size_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        let result =
            detect_kick_disjoint_stab_onsets(&spec, SR, FRAME, 0, &DubStabConfig::default());
        assert!(result.is_err());
    }

    #[test]
    fn mask_kick_coincident_drops_only_within_symmetric_window() {
        // Hand-crafted frame indices: avoids the synth-audio coupling where
        // sharp stab transients bleed into the kick band and trigger
        // false-kicks at stab times.
        let kicks = [10_usize, 50, 100];
        let stabs = [5_usize, 9, 15, 16, 30, 45, 55, 95, 105, 120];
        let mask_frames = 5;

        let kept = mask_kick_coincident(&stabs, &kicks, mask_frames);

        // Within ±5 of a kick: 5 (≤10), 9 (≤10), 15 (≥10), 45 (≤50), 55 (≥50),
        //                      95 (≤100), 105 (≥100). Survivors: 16, 30, 120.
        assert_eq!(kept, vec![16, 30, 120]);
    }

    #[test]
    fn mask_kick_coincident_with_no_kicks_returns_input_unchanged() {
        let stabs = [5_usize, 10, 20];
        let kept = mask_kick_coincident(&stabs, &[], 5);
        assert_eq!(kept, stabs);
    }

    #[test]
    fn pipeline_masks_kicks_using_hand_crafted_spectrogram() {
        // Build a spectrogram with kick-band energy at frames 10, 30 and
        // stab-band energy at frames 11, 20, 31, 50. With ±80 ms mask at SR
        // 44_100 / HOP 512 (~7 frames), stabs at 11 (∈ [3, 17]) and 31
        // (∈ [23, 37]) are masked; 20 and 50 survive.
        let n_frames = 60;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];

        // Kick band: bins 2–6 (~43–129 Hz at 44_100/2048).
        for &f in &[10_usize, 30] {
            for cell in spec[f].iter_mut().take(7).skip(2) {
                *cell = 1.0;
            }
        }
        // Stab band: bins 20–80 (~430–1722 Hz).
        for &f in &[11_usize, 20, 31, 50] {
            for cell in spec[f].iter_mut().take(80).skip(20) {
                *cell = 1.0;
            }
        }

        let config = DubStabConfig {
            onset_threshold_percentile: 0.5,
            ..DubStabConfig::default()
        };
        let onsets = detect_kick_disjoint_stab_onsets(&spec, SR, FRAME, HOP, &config).unwrap();

        let mask_frames =
            (config.kick_mask_window_ms / 1000.0 * SR as f32 / HOP as f32).round() as usize;
        let kicks = [10_usize, 30];
        for &k in &kicks {
            for &o in &onsets {
                assert!(
                    o.abs_diff(k) > mask_frames,
                    "stab at frame {o} survived ±{mask_frames}-frame mask around kick {k}"
                );
            }
        }
        // 20 and 50 are well outside the mask; at least one should survive.
        assert!(
            onsets.iter().any(|&f| (18..=22).contains(&f))
                || onsets.iter().any(|&f| (48..=52).contains(&f)),
            "expected an off-beat stab onset to survive, got {onsets:?}"
        );
    }

    #[test]
    fn no_kicks_passes_through_all_stab_onsets() {
        // Build a synthetic spectrogram with stab-band onsets but no kick-band
        // energy. Kick-mask should be a no-op.
        let n_frames = 20;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];
        // Bins 50–60 cover ~1075–1290 Hz (mid-band stabs).
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(60).skip(50) {
                *cell = 1.0;
            }
        }

        let onsets =
            detect_kick_disjoint_stab_onsets(&spec, SR, FRAME, HOP, &DubStabConfig::default())
                .unwrap();
        assert!(
            onsets.iter().any(|&f| (9..=12).contains(&f)),
            "expected stab onset to pass through with no kicks present, got {onsets:?}"
        );
    }

    #[test]
    fn kick_within_helper_handles_window_edges() {
        let kicks = [10_usize, 50, 100];
        // Inside window
        assert!(kick_within(15, &kicks, 5));
        // At exact window edge — inclusive
        assert!(kick_within(15, &kicks, 5));
        assert!(kick_within(105, &kicks, 5));
        // Just outside
        assert!(!kick_within(16, &kicks, 5));
        assert!(!kick_within(106, &kicks, 5));
        // Empty kicks
        assert!(!kick_within(15, &[], 5));
        // Saturating-sub edge near 0
        assert!(kick_within(2, &[0, 8], 5));
    }

    // ─── Stage 2 ────────────────────────────────────────────────────────────

    #[test]
    fn empty_onsets_yield_zero_histogram() {
        let beat_grid = beat_grid_4on4(120.0, 8.0);
        let hist = beat_relative_offset_histogram(&[], HOP, SR, &beat_grid).unwrap();
        assert_eq!(hist.global.len(), HISTOGRAM_BINS);
        assert!(hist.global.iter().all(|&v| v == 0.0));
        assert_eq!(hist.per_bar.len(), beat_grid.bars.len());
        assert!(hist.per_bar.iter().all(|b| b.iter().all(|&v| v == 0.0)));
    }

    #[test]
    fn fewer_than_two_beats_yields_zero_histogram() {
        let grid = BeatGrid {
            downbeats: vec![],
            beats: vec![1.0],
            bars: vec![],
        };
        let hist = beat_relative_offset_histogram(&[100], HOP, SR, &grid).unwrap();
        assert!(hist.global.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn off_beat_eighths_peak_near_bin_16() {
        // 120 BPM, beat_period = 0.5 s. Onsets at 0.25, 0.75, 1.25, ... — each
        // exactly halfway between beats, so the offset is 0.5 → bin 16
        // in a 32-bin histogram.
        let bpm = 120.0;
        let beat_period = 60.0 / bpm;
        let duration = 8.0_f32;
        let beat_grid = beat_grid_4on4(bpm, duration);

        let onset_frames: Vec<usize> = (0..16)
            .map(|i| beat_period * 0.5 + beat_period * i as f32)
            .filter(|&t| t < duration)
            .map(time_to_frame)
            .collect();

        let hist = beat_relative_offset_histogram(&onset_frames, HOP, SR, &beat_grid).unwrap();
        let argmax = hist
            .global
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert!(
            (15..=17).contains(&argmax),
            "off-beat 8ths should peak near bin 16, got argmax={argmax}, hist={:?}",
            hist.global
        );
    }

    #[test]
    fn on_beat_quarters_peak_at_bin_zero() {
        // Onsets exactly on every beat — offset 0.0, peak at bin 0 (with small
        // wrap-around contribution from bin 31).
        let bpm = 120.0;
        let beat_grid = beat_grid_4on4(bpm, 8.0);

        let onset_frames: Vec<usize> = beat_grid
            .beats
            .iter()
            .skip(1) // skip frame 0 — peak-picker ignores it; keep the test consistent with stage-1 use
            .map(|&t| time_to_frame(t))
            .collect();

        let hist = beat_relative_offset_histogram(&onset_frames, HOP, SR, &beat_grid).unwrap();
        assert!(
            hist.global[0] > hist.global[16],
            "on-beat onsets should give bin 0 > bin 16: bin0={} bin16={}",
            hist.global[0],
            hist.global[16]
        );
        // bin 31 picks up significant wrap-around from offsets just below 1.0
        // (rounding noise from time_to_frame). Both 0 and 31 should dominate
        // the rest.
        assert!(hist.global[0] > hist.global[8]);
    }

    #[test]
    fn per_bar_histograms_track_temporal_density() {
        // Off-beat 8th onsets only in the first two bars of a 16 s window.
        // Later bars must show effectively zero density.
        let bpm = 120.0;
        let beat_period = 60.0 / bpm;
        let duration = 16.0_f32;
        let beat_grid = beat_grid_4on4(bpm, duration);

        let onset_frames: Vec<usize> = (0..8)
            .map(|i| beat_period * 0.5 + beat_period * i as f32)
            .map(time_to_frame)
            .collect();

        let hist = beat_relative_offset_histogram(&onset_frames, HOP, SR, &beat_grid).unwrap();

        assert!(
            hist.per_bar.len() >= 4,
            "expected ≥ 4 bars in 16 s @ 120 BPM, got {}",
            hist.per_bar.len()
        );
        let bar_sum = |i: usize| hist.per_bar.get(i).map_or(0.0, |b| b.iter().sum::<f32>());
        let (b0, b1, b2, b3) = (bar_sum(0), bar_sum(1), bar_sum(2), bar_sum(3));

        assert!(b0 > 0.0, "bar 0 should have onsets");
        assert!(b1 > 0.0, "bar 1 should have onsets");
        assert!(
            b2 < b0 / 10.0,
            "bar 2 should be effectively empty: bar2={b2}, bar0={b0}"
        );
        assert!(
            b3 < b0 / 10.0,
            "bar 3 should be effectively empty: bar3={b3}, bar0={b0}"
        );
    }

    #[test]
    fn onset_before_first_beat_is_discarded() {
        let bpm = 120.0;
        let beat_period: f32 = 60.0 / bpm;
        let beats: Vec<f32> = (0..10).map(|i| 1.0 + i as f32 * beat_period).collect();
        let bars: Vec<f32> = beats.iter().step_by(4).copied().collect();
        let grid = BeatGrid {
            downbeats: bars.clone(),
            beats,
            bars,
        };
        // Onset at 0.5 s — before first beat at 1.0 s.
        let frame = time_to_frame(0.5);
        let hist = beat_relative_offset_histogram(&[frame], HOP, SR, &grid).unwrap();
        assert!(
            hist.global.iter().all(|&v| v == 0.0),
            "pre-grid onset must be discarded, got {:?}",
            hist.global
        );
    }

    #[test]
    fn onset_after_last_beat_is_discarded() {
        let bpm = 120.0;
        let beat_period: f32 = 60.0 / bpm;
        let beats: Vec<f32> = (0..4).map(|i| i as f32 * beat_period).collect();
        let grid = BeatGrid {
            downbeats: vec![0.0],
            beats,
            bars: vec![0.0],
        };
        // Last beat is at 1.5 s; onset at 5.0 s is well past.
        let frame = time_to_frame(5.0);
        let hist = beat_relative_offset_histogram(&[frame], HOP, SR, &grid).unwrap();
        assert!(hist.global.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn nan_beat_returns_invalid_input() {
        let grid = BeatGrid {
            downbeats: vec![0.0],
            beats: vec![0.0, f32::NAN, 1.0, 1.5],
            bars: vec![0.0],
        };
        let err = beat_relative_offset_histogram(&[100], HOP, SR, &grid);
        assert!(matches!(err, Err(AnalysisError::InvalidInput(_))));
    }

    #[test]
    fn nan_bar_returns_invalid_input() {
        let grid = BeatGrid {
            downbeats: vec![0.0],
            beats: vec![0.0, 0.5, 1.0, 1.5],
            bars: vec![0.0, f32::NAN],
        };
        let err = beat_relative_offset_histogram(&[100], HOP, SR, &grid);
        assert!(matches!(err, Err(AnalysisError::InvalidInput(_))));
    }

    #[test]
    fn duplicate_beat_returns_invalid_input() {
        let grid = BeatGrid {
            downbeats: vec![0.0],
            beats: vec![0.0, 1.0, 1.0, 2.0],
            bars: vec![0.0],
        };
        let err = beat_relative_offset_histogram(&[100], HOP, SR, &grid);
        assert!(matches!(err, Err(AnalysisError::InvalidInput(_))));
    }

    #[test]
    fn non_monotonic_beats_return_invalid_input() {
        let grid = BeatGrid {
            downbeats: vec![0.0],
            beats: vec![0.0, 1.0, 0.5, 1.5],
            bars: vec![0.0],
        };
        let err = beat_relative_offset_histogram(&[100], HOP, SR, &grid);
        assert!(matches!(err, Err(AnalysisError::InvalidInput(_))));
    }

    #[test]
    fn wrap_around_near_offset_one_lights_both_bin_zero_and_bin_31() {
        // Place an onset at offset ≈ 31.5 / 32 = 0.984 of a beat. Use 60 BPM
        // so frame quantisation is fine enough to land near the target.
        // With circular soft-binning, weight should land in both bin 31
        // (closest centre directly) and bin 0 (across the wrap), with both
        // dominating bin 16 (~half the beat away).
        //
        // Locks the `if d > 0.5 { d = 1.0 - d; }` wrap. A regression that
        // replaced the wrap with `d = 0.5` or removed it would put zero
        // weight in bin 0.
        let bpm = 60.0;
        let beat_period = 60.0_f32 / bpm;
        let beat_grid = beat_grid_4on4(bpm, 4.0);

        let onset_time = beat_period * (31.5 / HISTOGRAM_BINS as f32);
        let frame = time_to_frame(onset_time);
        let hist = beat_relative_offset_histogram(&[frame], HOP, SR, &beat_grid).unwrap();

        let h0 = hist.global[0];
        let h31 = hist.global[31];
        let h16 = hist.global[16];
        assert!(h31 > 0.3, "bin 31 (closest) should be substantial: {h31}");
        assert!(
            h0 > 0.3,
            "bin 0 should receive substantial wrap-around weight: {h0} (wrap broken?)"
        );
        assert!(
            h0 > 100.0 * h16,
            "bin 0 should swamp bin 16 via wrap: bin0={h0} bin16={h16}"
        );
    }

    #[test]
    fn soft_binning_spreads_across_neighbour_bins() {
        // A single onset placed exactly between two bins (offset 15.5 / 32 =
        // 0.484) should give nearly equal weight to bins 15 and 16, and very
        // little to bins 0, 8, 24.
        let bpm = 120.0;
        let beat_period = 60.0_f32 / bpm;
        let beat_grid = beat_grid_4on4(bpm, 4.0);

        let onset_time = beat_period * (15.5 / HISTOGRAM_BINS as f32);
        let frame = time_to_frame(onset_time);
        let hist = beat_relative_offset_histogram(&[frame], HOP, SR, &beat_grid).unwrap();

        let h15 = hist.global[15];
        let h16 = hist.global[16];
        let h0 = hist.global[0];
        let h8 = hist.global[8];
        // Adjacent bins receive substantial weight; their ratio depends on
        // STFT-frame rounding nudging the offset toward one bin centre. A
        // factor-of-3 cap rules out only the pathological "one bin gets
        // everything" case.
        let ratio = (h15 / h16).max(h16 / h15);
        assert!(
            ratio < 3.0,
            "neighbour bins should both receive substantial weight: bin15={h15} bin16={h16}"
        );
        assert!(h15 > 0.3, "bin 15 should be substantial: {h15}");
        assert!(h16 > 0.3, "bin 16 should be substantial: {h16}");
        // Far bins receive negligible weight.
        assert!(h0 < h15 / 100.0, "bin 0 should be ~zero: {h0} vs {h15}");
        assert!(h8 < h15 / 100.0, "bin 8 should be ~zero: {h8} vs {h15}");
    }
}
