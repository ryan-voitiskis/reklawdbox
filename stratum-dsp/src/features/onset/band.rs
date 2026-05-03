//! Band-restricted spectral flux onset detection.
//!
//! Mirrors [`super::spectral_flux::detect_spectral_flux_onsets`] but limits
//! both per-frame normalisation and the L2 flux computation to a configurable
//! frequency band. This lets callers detect onsets within a specific band
//! (e.g. 40–120 Hz for kicks, 350–2000 Hz for chord stabs) without bleed
//! from out-of-band energy.
//!
//! Reuses the shared STFT computed once in `lib.rs` — no extra FFT pass.

use crate::error::AnalysisError;

const EPSILON: f32 = 1e-10;

/// Detect onsets via spectral flux restricted to a frequency band.
///
/// Per-frame max normalisation is applied to band bins only, so a track with
/// loud out-of-band content (e.g. a heavy kick) doesn't suppress mid-band
/// stabs. The L2 half-wave-rectified flux is summed only over band bins.
///
/// # Arguments
///
/// * `fft_magnitudes` — FFT magnitude spectrogram (`n_frames × n_bins`).
/// * `sample_rate` — Audio sample rate in Hz (used to map band edges to bins).
/// * `frame_size` — FFT size used to produce `fft_magnitudes` (used for the
///   bin-centre frequency formula `freq[k] = k * sample_rate / frame_size`).
/// * `band_hz` — `(low, high)` band edges in Hz. `low` must be ≥ 0 and < `high`,
///   and `low` must be below Nyquist.
/// * `threshold_percentile` — percentile in `[0, 1]` of the band-flux
///   distribution used as the peak-pick threshold.
///
/// # Returns
///
/// Frame indices (0-based) where a band-restricted onset was detected,
/// sorted ascending.
pub fn detect_band_onsets(
    fft_magnitudes: &[Vec<f32>],
    sample_rate: u32,
    frame_size: usize,
    band_hz: (f32, f32),
    threshold_percentile: f32,
) -> Result<Vec<usize>, AnalysisError> {
    if fft_magnitudes.is_empty() {
        return Ok(Vec::new());
    }

    if !(0.0..=1.0).contains(&threshold_percentile) {
        return Err(AnalysisError::InvalidInput(format!(
            "Threshold percentile must be in [0, 1], got {}",
            threshold_percentile
        )));
    }

    let (low_hz, high_hz) = band_hz;
    if !(low_hz.is_finite() && high_hz.is_finite()) || low_hz < 0.0 || high_hz <= low_hz {
        return Err(AnalysisError::InvalidInput(format!(
            "Invalid band [{}, {}) Hz: require 0 ≤ low < high and both finite",
            low_hz, high_hz
        )));
    }

    let nyquist = sample_rate as f32 / 2.0;
    if low_hz >= nyquist {
        return Err(AnalysisError::InvalidInput(format!(
            "Band low {} Hz is at or above Nyquist {} Hz",
            low_hz, nyquist
        )));
    }

    let n_bins = fft_magnitudes[0].len();
    if n_bins == 0 {
        return Err(AnalysisError::InvalidInput(
            "Empty magnitude frames".to_string(),
        ));
    }
    for (i, frame) in fft_magnitudes.iter().enumerate() {
        if frame.len() != n_bins {
            return Err(AnalysisError::InvalidInput(format!(
                "Inconsistent frame lengths: frame 0 has {} bins, frame {} has {} bins",
                n_bins,
                i,
                frame.len()
            )));
        }
    }

    if fft_magnitudes.len() < 2 {
        return Ok(Vec::new());
    }

    // Map band edges to bin indices using freq[k] = k * sample_rate / frame_size.
    let bins_per_hz = frame_size as f32 / sample_rate as f32;
    let bin_low = (low_hz * bins_per_hz).ceil() as isize;
    let bin_high = (high_hz * bins_per_hz).floor() as isize;
    if bin_low < 0 || bin_high < bin_low || (bin_low as usize) >= n_bins {
        // Band is too narrow to contain any bin centre, or sits beyond the
        // available bins. Not an error — caller may sweep band edges.
        return Ok(Vec::new());
    }
    let bin_low = bin_low as usize;
    let bin_high = (bin_high as usize).min(n_bins - 1);

    log::debug!(
        "Band-restricted onset detection: bins {}..={} ({:.1}-{:.1} Hz), {} frames, percentile={:.2}",
        bin_low,
        bin_high,
        low_hz,
        high_hz,
        fft_magnitudes.len(),
        threshold_percentile
    );

    // Per-frame normalisation over band bins only — so a track with loud
    // out-of-band energy doesn't suppress in-band onsets.
    let mut normalized: Vec<Vec<f32>> = Vec::with_capacity(fft_magnitudes.len());
    for frame in fft_magnitudes {
        let band = &frame[bin_low..=bin_high];
        let max_mag = band.iter().copied().fold(0.0_f32, f32::max);
        if max_mag > EPSILON {
            normalized.push(band.iter().map(|&x| x / max_mag).collect());
        } else {
            normalized.push(vec![0.0; band.len()]);
        }
    }

    // L2 of half-wave-rectified diffs across the band.
    let mut spectral_flux = Vec::with_capacity(fft_magnitudes.len() - 1);
    for i in 1..normalized.len() {
        let prev = &normalized[i - 1];
        let curr = &normalized[i];
        let sum_sq: f32 = prev
            .iter()
            .zip(curr.iter())
            .map(|(&p, &c)| (c - p).max(0.0))
            .map(|d| d * d)
            .sum();
        spectral_flux.push(sum_sq.sqrt());
    }
    if spectral_flux.is_empty() {
        return Ok(Vec::new());
    }

    // Percentile threshold over the band-flux distribution.
    let mut sorted = spectral_flux.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let threshold_idx = ((sorted.len() as f32) * threshold_percentile) as usize;
    let threshold_idx = threshold_idx.min(sorted.len() - 1);
    let threshold = sorted[threshold_idx];

    // Peak-pick local maxima above threshold.
    let mut onsets = Vec::new();
    if spectral_flux.len() >= 3 {
        for i in 1..(spectral_flux.len() - 1) {
            let f = spectral_flux[i];
            if f > threshold && f > spectral_flux[i - 1] && f >= spectral_flux[i + 1] {
                onsets.push(i + 1);
            }
        }
    }
    if spectral_flux.len() > 1
        && spectral_flux[0] > threshold
        && spectral_flux[0] >= spectral_flux[1]
    {
        onsets.push(1);
    }
    let last = spectral_flux.len() - 1;
    if spectral_flux.len() > 1
        && spectral_flux[last] > threshold
        && spectral_flux[last] > spectral_flux[last - 1]
    {
        onsets.push(spectral_flux.len());
    }

    onsets.sort_unstable();
    onsets.dedup();

    log::debug!("Band-restricted: detected {} onsets", onsets.len());
    Ok(onsets)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44_100;
    const FRAME: usize = 2048;
    const N_BINS: usize = FRAME / 2 + 1; // 1025

    #[test]
    fn empty_spectrogram_returns_empty() {
        let result = detect_band_onsets(&[], SR, FRAME, (200.0, 2000.0), 0.8).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn invalid_threshold_percentile_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        assert!(detect_band_onsets(&spec, SR, FRAME, (200.0, 2000.0), -0.1).is_err());
        assert!(detect_band_onsets(&spec, SR, FRAME, (200.0, 2000.0), 1.5).is_err());
    }

    #[test]
    fn invalid_band_low_ge_high_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        assert!(detect_band_onsets(&spec, SR, FRAME, (2000.0, 200.0), 0.5).is_err());
        assert!(detect_band_onsets(&spec, SR, FRAME, (1000.0, 1000.0), 0.5).is_err());
    }

    #[test]
    fn negative_band_frequency_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        assert!(detect_band_onsets(&spec, SR, FRAME, (-100.0, 1000.0), 0.5).is_err());
    }

    #[test]
    fn band_above_nyquist_returns_err() {
        let spec = vec![vec![0.0_f32; N_BINS]; 10];
        // Nyquist for 44.1 kHz is 22.05 kHz; (30k, 40k) is fully above it.
        assert!(detect_band_onsets(&spec, SR, FRAME, (30_000.0, 40_000.0), 0.5).is_err());
    }

    #[test]
    fn inconsistent_frame_lengths_returns_err() {
        let mut spec = vec![vec![0.0_f32; N_BINS]; 10];
        spec[5] = vec![0.0_f32; 512];
        assert!(detect_band_onsets(&spec, SR, FRAME, (200.0, 2000.0), 0.5).is_err());
    }

    #[test]
    fn single_frame_returns_empty() {
        let spec = vec![vec![0.0_f32; N_BINS]];
        let onsets = detect_band_onsets(&spec, SR, FRAME, (200.0, 2000.0), 0.5).unwrap();
        assert!(onsets.is_empty());
    }

    #[test]
    fn detects_onset_introduced_in_target_band() {
        // 44100/2048 ≈ 21.5 Hz/bin. Band 350–2000 Hz → bins ~17–93.
        // Baseline frames have content at low bins only (out-of-band).
        // From frame 10 onward, content appears in bins 50–60 (in-band).
        // Detector queries the in-band region and should fire near frame 10.
        let n_frames = 20;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];
        for frame in spec.iter_mut() {
            for cell in frame.iter_mut().take(10) {
                *cell = 1.0;
            }
        }
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(60).skip(50) {
                *cell = 1.0;
            }
        }

        let onsets = detect_band_onsets(&spec, SR, FRAME, (350.0, 2000.0), 0.5).unwrap();
        assert!(!onsets.is_empty(), "expected at least one onset");
        assert!(
            onsets.iter().any(|&f| (9..=12).contains(&f)),
            "onset should be near frame 10, got {:?}",
            onsets,
        );
    }

    #[test]
    fn ignores_onset_outside_target_band() {
        // Onset is in bins 2–5 (low band, ~40–110 Hz). Querying 350–2000 Hz
        // should see nothing — those bins are not in the queried band.
        let n_frames = 20;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(6).skip(2) {
                *cell = 1.0;
            }
        }

        let onsets = detect_band_onsets(&spec, SR, FRAME, (350.0, 2000.0), 0.5).unwrap();
        assert!(
            onsets.is_empty(),
            "low-band onset should not trigger high-band detection: {:?}",
            onsets,
        );
    }

    #[test]
    fn distinguishes_kick_band_from_stab_band() {
        // Onset is in bins 2–5 only (kick band, ~40–110 Hz).
        // Querying the kick band fires; querying the stab band stays silent.
        let n_frames = 20;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(6).skip(2) {
                *cell = 1.0;
            }
        }

        let kick_onsets = detect_band_onsets(&spec, SR, FRAME, (40.0, 120.0), 0.5).unwrap();
        let stab_onsets = detect_band_onsets(&spec, SR, FRAME, (350.0, 2000.0), 0.5).unwrap();

        assert!(
            !kick_onsets.is_empty(),
            "kick band should fire on kick onset"
        );
        assert!(
            stab_onsets.is_empty(),
            "stab band should ignore kick-only onset: {:?}",
            stab_onsets,
        );
    }

    #[test]
    fn full_spectrum_band_matches_unbanded_detector() {
        use super::super::spectral_flux::detect_spectral_flux_onsets;

        // With band == full spectrum, the band-restricted variant reduces
        // exactly to the existing detector (same per-frame normaliser, same
        // flux summation, same peak-pick). Sanity check the equivalence.
        let n_frames = 30;
        let mut spec = vec![vec![0.01_f32; N_BINS]; n_frames];
        for frame in &mut spec[10..15] {
            for cell in frame.iter_mut().take(300).skip(100) {
                *cell = 1.0;
            }
        }
        for frame in &mut spec[20..25] {
            for cell in frame.iter_mut().take(800).skip(600) {
                *cell = 1.0;
            }
        }

        let nyquist = SR as f32 / 2.0;
        let banded = detect_band_onsets(&spec, SR, FRAME, (0.0, nyquist), 0.5).unwrap();
        let unbanded = detect_spectral_flux_onsets(&spec, 0.5).unwrap();
        assert_eq!(banded, unbanded);
    }

    #[test]
    fn rejects_out_of_band_events_that_full_spectrum_detector_picks_up() {
        // Per-band normalisation must do two things: surface a quiet in-band
        // onset, AND ignore loud out-of-band events. We exercise the second
        // (and harder) claim by comparing against the existing full-spectrum
        // detector on the same magnitudes.
        //
        // Data: impulsive sub-bass kicks (bins 0..=4 jump 0→10→0) at frames
        // 3, 7, 13, plus a single in-band step (bins 50..=59 from 0 to 1.0)
        // at frame 10. The chord-stab band (350–2000 Hz) covers bins ~17–92
        // so excludes the kicks entirely.
        use super::super::spectral_flux::detect_spectral_flux_onsets;

        let n_frames = 20;
        let mut spec = vec![vec![0.0_f32; N_BINS]; n_frames];
        for &kick_frame in &[3usize, 7, 13] {
            for cell in spec[kick_frame].iter_mut().take(5) {
                *cell = 10.0;
            }
        }
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(60).skip(50) {
                *cell = 1.0;
            }
        }

        let banded = detect_band_onsets(&spec, SR, FRAME, (350.0, 2000.0), 0.5).unwrap();
        let unbanded = detect_spectral_flux_onsets(&spec, 0.5).unwrap();

        // Banded detector finds the in-band onset (frame 10, ±1 for picker).
        assert!(
            banded.iter().any(|&f| (9..=11).contains(&f)),
            "banded detector missed the in-band onset: {:?}",
            banded
        );
        // ...and ignores the kicks entirely.
        for &kick_frame in &[3usize, 7, 13] {
            assert!(
                !banded.iter().any(|&f| f.abs_diff(kick_frame) <= 1),
                "banded detector should ignore kick at frame {}: {:?}",
                kick_frame,
                banded
            );
        }
        // Sanity: the full-spectrum detector *does* fire on the kicks — proves
        // the test setup actually exercises out-of-band rejection rather than
        // just running on quiet data.
        for &kick_frame in &[3usize, 7, 13] {
            assert!(
                unbanded.iter().any(|&f| f.abs_diff(kick_frame) <= 1),
                "test setup broken: full-spectrum should detect kick at frame {}: {:?}",
                kick_frame,
                unbanded
            );
        }
    }

    #[test]
    fn detects_synthesised_kick_pattern_via_real_stft() {
        // Synthesise time-domain kicks, run compute_stft, then detect_band_onsets.
        // This exercises real FFT output rather than hand-built magnitudes —
        // the production path the chord-stab and kick-pattern detectors will use.
        use crate::features::chroma::extractor::compute_stft;

        let sr: u32 = 44_100;
        let frame_size = 2048;
        let hop_size = 512;
        let duration_secs = 3.0;
        let n_samples = (sr as f32 * duration_secs) as usize;
        let mut samples = vec![0.0_f32; n_samples];

        let kick_times_s = [0.5_f32, 1.0, 1.5, 2.0];
        for &kick_t in &kick_times_s {
            let start = (kick_t * sr as f32) as usize;
            for i in 0..2200 {
                let t = i as f32 / sr as f32;
                let env = (-t * 60.0).exp();
                let s = (2.0 * std::f32::consts::PI * 60.0 * t).sin() * env * 0.8
                    + (2.0 * std::f32::consts::PI * 180.0 * t).sin() * env * 0.3;
                if start + i < samples.len() {
                    samples[start + i] += s;
                }
            }
        }

        let spec = compute_stft(&samples, frame_size, hop_size).unwrap();
        let onsets = detect_band_onsets(&spec, sr, frame_size, (40.0, 200.0), 0.7).unwrap();
        assert!(!onsets.is_empty(), "expected kick onsets in 40-200 Hz band");

        let onset_times: Vec<f32> = onsets
            .iter()
            .map(|&f| f as f32 * hop_size as f32 / sr as f32)
            .collect();
        for &kick_t in &kick_times_s {
            let hit = onset_times.iter().any(|&t| (t - kick_t).abs() < 0.1);
            assert!(
                hit,
                "no onset within 100 ms of {} s; got {:?}",
                kick_t, onset_times
            );
        }

        // Sharp kick attacks leak across the spectrum, so the stab band fires
        // too — but every stab-band onset must land near a kick time. This
        // documents *why* the chord-stab detector needs kick-coincidence
        // masking: band restriction alone cannot reject transient bleed.
        let stab_band = detect_band_onsets(&spec, sr, frame_size, (350.0, 2000.0), 0.95).unwrap();
        let stab_times: Vec<f32> = stab_band
            .iter()
            .map(|&f| f as f32 * hop_size as f32 / sr as f32)
            .collect();
        for &t in &stab_times {
            let near_kick = kick_times_s.iter().any(|&kt| (t - kt).abs() < 0.1);
            assert!(
                near_kick,
                "stab-band onset at {} s did not coincide with any kick: {:?}",
                t, stab_times
            );
        }
    }

    #[test]
    fn detects_stabs_in_stab_band_alongside_kicks() {
        // The complement of the kick-pattern test: a real in-band event must
        // survive when out-of-band kicks are also present. The chord-stab
        // detector's downstream kick-coincidence mask must not eliminate
        // genuine on-kick stabs, so one stab is placed at a kick time.
        use crate::features::chroma::extractor::compute_stft;

        let sr: u32 = 44_100;
        let frame_size = 2048;
        let hop_size = 512;
        let duration_secs = 3.0;
        let n_samples = (sr as f32 * duration_secs) as usize;
        let mut samples = vec![0.0_f32; n_samples];

        // Kicks at every half-second.
        let kick_times_s = [0.5_f32, 1.0, 1.5, 2.0];
        for &kick_t in &kick_times_s {
            let start = (kick_t * sr as f32) as usize;
            for i in 0..2200 {
                let t = i as f32 / sr as f32;
                let env = (-t * 60.0).exp();
                let s = (2.0 * std::f32::consts::PI * 60.0 * t).sin() * env * 0.8
                    + (2.0 * std::f32::consts::PI * 180.0 * t).sin() * env * 0.3;
                if start + i < samples.len() {
                    samples[start + i] += s;
                }
            }
        }

        // Stabs: 1 kHz tone bursts at 1.25 s (between kicks) and 2.0 s (on a kick).
        let stab_times_s = [1.25_f32, 2.0];
        for &stab_t in &stab_times_s {
            let start = (stab_t * sr as f32) as usize;
            for i in 0..4400 {
                let t = i as f32 / sr as f32;
                let env = (-t * 30.0).exp();
                let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * env * 0.6;
                if start + i < samples.len() {
                    samples[start + i] += s;
                }
            }
        }

        let spec = compute_stft(&samples, frame_size, hop_size).unwrap();
        let onsets = detect_band_onsets(&spec, sr, frame_size, (350.0, 2000.0), 0.7).unwrap();
        let onset_times: Vec<f32> = onsets
            .iter()
            .map(|&f| f as f32 * hop_size as f32 / sr as f32)
            .collect();

        for &stab_t in &stab_times_s {
            let hit = onset_times.iter().any(|&t| (t - stab_t).abs() < 0.1);
            assert!(
                hit,
                "stab-band onset missing within 100 ms of {} s; got {:?}",
                stab_t, onset_times
            );
        }
    }

    #[test]
    fn band_too_narrow_for_any_bin_returns_empty() {
        // A band thinner than the bin width (~21.5 Hz at sr=44.1k, frame=2048)
        // may or may not contain a bin centre. With (1000.0, 1001.0) Hz the
        // band is 1 Hz wide and almost certainly skips every bin centre — the
        // detector should return Ok(empty), not error.
        let n_frames = 20;
        let mut spec = vec![vec![0.01_f32; N_BINS]; n_frames];
        for frame in &mut spec[10..] {
            for cell in frame.iter_mut().take(50).skip(40) {
                *cell = 1.0;
            }
        }

        let onsets = detect_band_onsets(&spec, SR, FRAME, (1000.0, 1001.0), 0.5).unwrap();
        assert!(onsets.is_empty());
    }
}
