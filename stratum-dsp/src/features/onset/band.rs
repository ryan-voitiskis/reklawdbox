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
