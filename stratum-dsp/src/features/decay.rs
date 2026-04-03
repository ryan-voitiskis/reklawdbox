//! Post-transient spectral decay rate — measures reverb tail length.
//!
//! For each detected onset, extracts the energy envelope in two frequency bands
//! (1-4 kHz mid-range, 4-8 kHz hi-hat/cymbal) and fits an exponential decay
//! in the log domain. Reports median decay time constant (tau), IQR, fit
//! quality (R²), and usable window count per band.

use crate::error::AnalysisError;

/// Decay analysis results for a single frequency band.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BandDecay {
    /// Median decay time constant in milliseconds.
    pub tau_median: f32,
    /// Interquartile range of tau values (ms).
    pub tau_iqr: f32,
    /// Median R² of the log-linear fit (0-1, higher = cleaner exponential).
    pub fit_r2_median: f32,
    /// Number of usable transient windows (≥ 150ms).
    pub usable_count: u32,
}

/// Full decay analysis output (two bands).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DecayResult {
    /// 1-4 kHz band (chord/snare reverb).
    pub mid: Option<BandDecay>,
    /// 4-8 kHz band (hi-hat/cymbal reverb).
    pub high: Option<BandDecay>,
}

/// Minimum window length in frames to be considered usable.
const MIN_WINDOW_FRAMES: usize = 13; // ~150ms at sr=44100, hop=512 (~86 Hz frame rate)

/// Guard gap in frames subtracted from end of each window (20ms ≈ 2 frames).
const GUARD_FRAMES: usize = 2;

/// Maximum tau in ms (clamp).
const MAX_TAU_MS: f32 = 5000.0;

/// Compute post-transient decay rates from STFT magnitude frames and onset positions.
///
/// `onset_frames` should be sorted frame indices of detected onsets.
/// `sample_rate` and `hop_size` are needed to convert frame indices to time.
pub fn post_transient_decay(
    magnitude_spec_frames: &[Vec<f32>],
    onset_frames: &[usize],
    sample_rate: u32,
    hop_size: usize,
) -> Result<DecayResult, AnalysisError> {
    if magnitude_spec_frames.is_empty() || onset_frames.len() < 2 {
        return Ok(DecayResult {
            mid: None,
            high: None,
        });
    }

    let n_bins = magnitude_spec_frames[0].len();
    let n_frames = magnitude_spec_frames.len();

    // Trim first/last 15%
    let trim = n_frames * 15 / 100;
    let frame_start = trim;
    let frame_end = n_frames - trim;
    if frame_end <= frame_start + 4 {
        return Ok(DecayResult {
            mid: None,
            high: None,
        });
    }

    // Compute frequency per bin
    let bin_hz = sample_rate as f64 / ((n_bins - 1) * 2) as f64;

    // Band ranges in bins
    let mid_lo = (1000.0 / bin_hz).round() as usize;
    let mid_hi = (4000.0 / bin_hz).round() as usize;
    let high_lo = (4000.0 / bin_hz).round() as usize;
    let high_hi = (8000.0 / bin_hz).round() as usize;

    let mid_lo = mid_lo.min(n_bins);
    let mid_hi = mid_hi.min(n_bins);
    let high_lo = high_lo.min(n_bins);
    let high_hi = high_hi.min(n_bins);

    if mid_lo >= mid_hi || high_lo >= high_hi {
        return Ok(DecayResult {
            mid: None,
            high: None,
        });
    }

    // Filter onsets to trimmed region
    let trimmed_onsets: Vec<usize> = onset_frames
        .iter()
        .copied()
        .filter(|&f| f >= frame_start && f < frame_end)
        .collect();

    if trimmed_onsets.len() < 2 {
        return Ok(DecayResult {
            mid: None,
            high: None,
        });
    }

    let ms_per_frame = (hop_size as f64 / sample_rate as f64) * 1000.0;

    // For each consecutive onset pair, extract decay windows
    let mut mid_taus = Vec::new();
    let mut mid_r2s = Vec::new();
    let mut high_taus = Vec::new();
    let mut high_r2s = Vec::new();

    for i in 0..trimmed_onsets.len() - 1 {
        let onset = trimmed_onsets[i];
        let next_onset = trimmed_onsets[i + 1];

        // Window: from onset to next_onset - guard
        let window_end = if next_onset > GUARD_FRAMES {
            next_onset - GUARD_FRAMES
        } else {
            continue;
        };

        if window_end <= onset {
            continue;
        }

        let window_len = window_end - onset;
        if window_len < MIN_WINDOW_FRAMES {
            continue;
        }

        // Clamp to valid frame range
        let w_start = onset.max(frame_start);
        let w_end = window_end.min(frame_end);
        if w_end <= w_start + 2 {
            continue;
        }

        // Extract band energy per frame in window
        let mid_env: Vec<f64> = (w_start..w_end)
            .map(|f| {
                magnitude_spec_frames[f][mid_lo..mid_hi]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
            })
            .collect();

        let high_env: Vec<f64> = (w_start..w_end)
            .map(|f| {
                magnitude_spec_frames[f][high_lo..high_hi]
                    .iter()
                    .map(|&x| (x as f64) * (x as f64))
                    .sum::<f64>()
            })
            .collect();

        // Fit decay in each band
        if let Some((tau, r2)) = fit_decay(&mid_env, ms_per_frame) {
            mid_taus.push(tau);
            mid_r2s.push(r2);
        }
        if let Some((tau, r2)) = fit_decay(&high_env, ms_per_frame) {
            high_taus.push(tau);
            high_r2s.push(r2);
        }
    }

    Ok(DecayResult {
        mid: make_band_decay(&mut mid_taus, &mut mid_r2s),
        high: make_band_decay(&mut high_taus, &mut high_r2s),
    })
}

/// Fit exponential decay to energy envelope in log domain.
/// Returns (tau_ms, r_squared) or None if fit is not possible.
fn fit_decay(envelope: &[f64], ms_per_frame: f64) -> Option<(f32, f32)> {
    if envelope.len() < 3 {
        return None;
    }

    // Convert to log domain, skipping near-zero values
    let floor = 1e-10_f64;
    let log_env: Vec<(f64, f64)> = envelope
        .iter()
        .enumerate()
        .filter(|(_, &e)| e > floor)
        .map(|(i, &e)| (i as f64 * ms_per_frame, e.ln()))
        .collect();

    if log_env.len() < 3 {
        return None;
    }

    // Linear regression: log(E) = slope * t + intercept
    // slope = -1/tau
    let n = log_env.len() as f64;
    let sum_x: f64 = log_env.iter().map(|(x, _)| x).sum();
    let sum_y: f64 = log_env.iter().map(|(_, y)| y).sum();
    let sum_xy: f64 = log_env.iter().map(|(x, y)| x * y).sum();
    let sum_xx: f64 = log_env.iter().map(|(x, _)| x * x).sum();

    let denom = n * sum_xx - sum_x * sum_x;
    if denom.abs() < 1e-20 {
        return None;
    }

    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;

    // tau = -1/slope (only valid if slope < 0, i.e., decaying)
    if slope >= 0.0 {
        // Not a decay — energy is increasing or flat
        return None;
    }
    let tau = (-1.0 / slope) as f32;
    let tau = tau.clamp(0.0, MAX_TAU_MS);

    // Compute R²
    let mean_y = sum_y / n;
    let ss_tot: f64 = log_env.iter().map(|(_, y)| (y - mean_y).powi(2)).sum();
    let ss_res: f64 = log_env
        .iter()
        .map(|(x, y)| {
            let predicted = slope * x + intercept;
            (y - predicted).powi(2)
        })
        .sum();

    let r2 = if ss_tot > 1e-20 {
        (1.0 - ss_res / ss_tot).max(0.0) as f32
    } else {
        0.0
    };

    Some((tau, r2))
}

fn make_band_decay(taus: &mut [f32], r2s: &mut [f32]) -> Option<BandDecay> {
    if taus.is_empty() {
        return None;
    }

    taus.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    r2s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let n = taus.len();
    let tau_median = if n.is_multiple_of(2) {
        taus[n / 2 - 1].midpoint(taus[n / 2])
    } else {
        taus[n / 2]
    };

    let q1_idx = n / 4;
    let q3_idx = (3 * n) / 4;
    let tau_iqr = taus[q3_idx.min(n - 1)] - taus[q1_idx.min(n - 1)];

    let r2_median = if n.is_multiple_of(2) {
        r2s[n / 2 - 1].midpoint(r2s[n / 2])
    } else {
        r2s[n / 2]
    };

    Some(BandDecay {
        tau_median,
        tau_iqr,
        fit_r2_median: r2_median,
        usable_count: n as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_none_for_empty_input() {
        let result = post_transient_decay(&[], &[], 44100, 512).unwrap();
        assert!(result.mid.is_none());
        assert!(result.high.is_none());
    }

    #[test]
    fn returns_none_for_insufficient_onsets() {
        let frames = vec![vec![1.0_f32; 1025]; 100];
        let result = post_transient_decay(&frames, &[50], 44100, 512).unwrap();
        assert!(result.mid.is_none());
    }

    #[test]
    fn fit_decay_detects_exponential() {
        // Simulate exponential decay: E(t) = e^(-t/200)
        let ms_per_frame = 11.6; // ~hop 512, sr 44100
        let envelope: Vec<f64> = (0..50)
            .map(|i| (-i as f64 * ms_per_frame / 200.0).exp())
            .collect();

        let (tau, r2) = fit_decay(&envelope, ms_per_frame).expect("should fit");
        assert!((tau - 200.0).abs() < 5.0, "tau should be ~200ms, got {tau}");
        assert!(
            r2 > 0.99,
            "R² should be near 1.0 for clean exponential, got {r2}"
        );
    }

    #[test]
    fn fit_decay_rejects_increasing_signal() {
        let ms_per_frame = 11.6;
        let envelope: Vec<f64> = (0..20).map(|i| (i as f64 * 0.1).exp()).collect();
        assert!(fit_decay(&envelope, ms_per_frame).is_none());
    }
}
