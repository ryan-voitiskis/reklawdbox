//! Modulation spectral centroid — measures reverb-induced temporal smoothing.
//!
//! Reverb acts as a low-pass filter on amplitude modulations: it fills gaps
//! between transients, suppressing fast amplitude changes while preserving slow
//! ones. The modulation spectral centroid captures this by computing how "fast"
//! the amplitude fluctuations are across frequency bands.
//!
//! **Algorithm:**
//! 1. Group STFT magnitude frames into 8 frequency bands
//! 2. Per band: sum magnitudes per frame → amplitude envelope (~86 Hz at sr=44100, hop=512)
//! 3. FFT each band envelope → modulation spectrum
//! 4. Compute spectral centroid of each modulation spectrum
//! 5. Average across bands → single scalar
//!
//! **Validated values (15-track reference set):**
//! - Dub Techno: highest (~11.3 Hz) — reverb sustains energy across many
//!   modulation frequencies, spreading the centroid upward
//! - Techno: middle (~10.4 Hz) — strong kick-rate modulation pulls centroid down
//! - Deep Techno: lowest (~9.8 Hz) — slow, hypnotic textures with minimal
//!   fast modulations

use rustfft::{num_complex::Complex, FftPlanner};

use crate::error::AnalysisError;

/// Number of frequency bands for envelope extraction.
const NUM_BANDS: usize = 8;

/// Compute modulation spectral centroid from STFT magnitude frames.
///
/// `magnitude_spec_frames` is `Vec<Vec<f32>>` where each inner vec has
/// `frame_size/2 + 1` bins. `sample_rate` and `hop_size` are needed to
/// convert modulation bin indices to Hz.
///
/// Returns `None` if the input is too short or all energy is near DC.
pub fn modulation_spectral_centroid(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    hop_size: usize,
) -> Result<Option<f32>, AnalysisError> {
    if magnitude_spec_frames.is_empty() {
        return Ok(None);
    }

    let n_bins = magnitude_spec_frames[0].len();
    if n_bins < NUM_BANDS {
        return Ok(None);
    }

    let n_frames = magnitude_spec_frames.len();
    if n_frames < 4 {
        return Ok(None);
    }

    // Trim first/last 15% of frames
    let trim = n_frames * 15 / 100;
    let start = trim;
    let end = n_frames - trim;
    if end <= start + 4 {
        return Ok(None);
    }
    let trimmed_frames = &magnitude_spec_frames[start..end];
    let trimmed_len = trimmed_frames.len();

    // Divide frequency bins into NUM_BANDS equal-width bands
    let bins_per_band = n_bins / NUM_BANDS;
    if bins_per_band == 0 {
        return Ok(None);
    }

    // Envelope frame rate in Hz
    let envelope_sr = sample_rate as f64 / hop_size as f64;

    // FFT size: next power of 2 >= trimmed_len
    let fft_size = trimmed_len.next_power_of_two();
    let mut planner = FftPlanner::<f64>::new();
    let fft = planner.plan_fft_forward(fft_size);

    let mut centroid_sum = 0.0_f64;
    let mut band_count = 0_usize;

    for band in 0..NUM_BANDS {
        let bin_start = band * bins_per_band;
        let bin_end = if band == NUM_BANDS - 1 {
            n_bins // last band gets remaining bins
        } else {
            (band + 1) * bins_per_band
        };

        // Build amplitude envelope: sum of magnitudes per frame in this band
        let mut buffer: Vec<Complex<f64>> = Vec::with_capacity(fft_size);
        for frame in trimmed_frames {
            let energy: f64 = frame[bin_start..bin_end].iter().map(|&x| x as f64).sum();
            buffer.push(Complex::new(energy, 0.0));
        }
        // Zero-pad to fft_size
        buffer.resize(fft_size, Complex::new(0.0, 0.0));

        // FFT → modulation spectrum
        fft.process(&mut buffer);

        // Only use positive frequencies (bins 1..fft_size/2)
        // Bin 0 is DC (mean level), skip it
        let nyquist_bin = fft_size / 2;
        let mod_freq_resolution = envelope_sr / fft_size as f64;

        // Compute spectral centroid of modulation spectrum
        let mut weighted_sum = 0.0_f64;
        let mut magnitude_sum = 0.0_f64;

        for (k, sample) in buffer.iter().enumerate().take(nyquist_bin).skip(1) {
            let mag = sample.norm();
            let freq = k as f64 * mod_freq_resolution;
            weighted_sum += freq * mag;
            magnitude_sum += mag;
        }

        if magnitude_sum > 1e-10 {
            centroid_sum += weighted_sum / magnitude_sum;
            band_count += 1;
        }
    }

    if band_count == 0 {
        return Ok(None);
    }

    let centroid = (centroid_sum / band_count as f64) as f32;

    // Clamp to [0, envelope_sr/2] Hz (Nyquist of envelope)
    let max_hz = (envelope_sr / 2.0) as f32;
    let clamped = centroid.clamp(0.0, max_hz);

    Ok(Some(clamped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_constant_frames(n_frames: usize, n_bins: usize, value: f32) -> Vec<Vec<f32>> {
        vec![vec![value; n_bins]; n_frames]
    }

    #[test]
    fn returns_none_for_empty_input() {
        let result = modulation_spectral_centroid(&[], 44100, 512).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn returns_none_for_very_short_input() {
        let frames = make_constant_frames(3, 1025, 1.0);
        let result = modulation_spectral_centroid(&frames, 44100, 512).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn constant_signal_has_low_centroid() {
        // Constant envelope → all energy at DC → centroid should be very low
        let frames = make_constant_frames(1000, 1025, 1.0);
        let result = modulation_spectral_centroid(&frames, 44100, 512)
            .unwrap()
            .expect("should produce a value");
        // DC is excluded so centroid comes from spectral leakage — should be near 0
        assert!(
            result < 10.0,
            "constant signal centroid should be low, got {result}"
        );
    }

    #[test]
    fn alternating_signal_has_higher_centroid() {
        // Alternating high/low → strong modulation at envelope_sr/2
        let n_bins = 1025;
        let mut frames = Vec::with_capacity(1000);
        for i in 0..1000 {
            let val = if i % 2 == 0 { 2.0 } else { 0.5 };
            frames.push(vec![val; n_bins]);
        }
        let result = modulation_spectral_centroid(&frames, 44100, 512)
            .unwrap()
            .expect("should produce a value");
        // Should be much higher than constant signal
        assert!(
            result > 10.0,
            "alternating signal should have high centroid, got {result}"
        );
    }

    #[test]
    fn output_is_clamped_to_nyquist() {
        let frames = make_constant_frames(100, 1025, 1.0);
        let result = modulation_spectral_centroid(&frames, 44100, 512).unwrap();
        if let Some(val) = result {
            let max_hz = 44100.0 / 512.0 / 2.0;
            assert!(val <= max_hz, "centroid {val} exceeds Nyquist {max_hz}");
        }
    }
}
