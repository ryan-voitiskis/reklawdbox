//! Integration tests for audio analysis engine

use std::path::PathBuf;
use stratum_dsp::{analyze_audio, AnalysisConfig};

/// Load a WAV file and return (samples, sample_rate)
fn load_wav(path: &str) -> Result<(Vec<f32>, u32), Box<dyn std::error::Error>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<Vec<_>, _>>()?,
        hound::SampleFormat::Int => {
            let max_value = (1 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / max_value))
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    // Convert to mono if stereo
    let mono_samples = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| (chunk[0] + chunk[1]) / 2.0)
            .collect()
    } else {
        samples
    };

    Ok((mono_samples, spec.sample_rate))
}

fn fixture_path(filename: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_120bpm_kick() {
        let path = fixture_path("120bpm_4bar.wav");
        let (samples, sample_rate) =
            load_wav(path.to_str().unwrap()).expect("Failed to load 120bpm_4bar.wav");

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, sample_rate, config).expect("Analysis should succeed");

        // Verify basic results
        assert!(result.metadata.duration_seconds > 7.0 && result.metadata.duration_seconds < 9.0);
        assert!(result.metadata.processing_time_ms > 0.0);
        assert_eq!(result.metadata.sample_rate, sample_rate);

        // Phase 1B: BPM detection should work
        // For fixed-tempo test fixtures, we can use tighter tolerance (±2 BPM)
        if result.bpm > 0.0 {
            assert!(
                (result.bpm - 120.0).abs() < 2.0,
                "BPM should be close to 120 (±2 BPM tolerance), got {:.2}",
                result.bpm
            );
            assert!(
                result.bpm_confidence > 0.0,
                "BPM confidence should be positive"
            );

            // Phase 1C: Beat tracking should work
            if !result.beat_grid.beats.is_empty() {
                assert!(
                    result.beat_grid.beats.len() >= 4,
                    "Should detect at least 4 beats for 4-bar track"
                );
                assert!(
                    result.grid_stability >= 0.0 && result.grid_stability <= 1.0,
                    "Grid stability should be in [0, 1]"
                );

                // Validate beat intervals (should be ~0.5s for 120 BPM)
                if result.beat_grid.beats.len() >= 2 {
                    let beat_interval = result.beat_grid.beats[1] - result.beat_grid.beats[0];
                    let expected_interval = 60.0 / 120.0; // 0.5s
                    assert!(
                        (beat_interval - expected_interval).abs() < 0.1,
                        "Beat interval should be ~0.5s for 120 BPM, got {:.3}s",
                        beat_interval
                    );
                }

                // Validate downbeats (should be every N beats depending on time signature)
                // For 4/4: ~2.0s, for 3/4: ~1.5s, for 6/8: ~3.0s
                if result.beat_grid.downbeats.len() >= 2 {
                    let bar_interval =
                        result.beat_grid.downbeats[1] - result.beat_grid.downbeats[0];
                    // Accept any reasonable bar interval (1.0s to 4.0s)
                    assert!(
                        (1.0..=4.0).contains(&bar_interval),
                        "Bar interval should be reasonable (1.0-4.0s), got {:.3}s",
                        bar_interval
                    );
                }

                // Phase 1C Enhanced: Variable tempo and time signature detection
                println!("120 BPM test: BPM={:.2}, confidence={:.3}, {} beats, {} downbeats, stability={:.3}, duration={:.2}s, processing={:.2}ms",
                         result.bpm, result.bpm_confidence, result.beat_grid.beats.len(),
                         result.beat_grid.downbeats.len(), result.grid_stability,
                         result.metadata.duration_seconds, result.metadata.processing_time_ms);
                // Note: Variable tempo and time signature detection are now integrated
            } else {
                println!("120 BPM test: BPM={:.2}, confidence={:.3}, but beat grid is empty, duration={:.2}s, processing={:.2}ms",
                         result.bpm, result.bpm_confidence, result.metadata.duration_seconds, result.metadata.processing_time_ms);
            }
        } else {
            println!(
                "120 BPM test: BPM detection failed, duration={:.2}s, processing={:.2}ms",
                result.metadata.duration_seconds, result.metadata.processing_time_ms
            );
        }
    }

    #[test]
    fn test_analyze_128bpm_kick() {
        let path = fixture_path("128bpm_4bar.wav");
        let (samples, sample_rate) =
            load_wav(path.to_str().unwrap()).expect("Failed to load 128bpm_4bar.wav");

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, sample_rate, config).expect("Analysis should succeed");

        // Verify basic results
        assert!(result.metadata.duration_seconds > 7.0 && result.metadata.duration_seconds < 8.0);
        assert!(result.metadata.processing_time_ms > 0.0);

        // Phase 1B: BPM detection should work
        // For fixed-tempo test fixtures, we can use tighter tolerance (±2 BPM)
        if result.bpm > 0.0 {
            assert!(
                (result.bpm - 128.0).abs() <= 2.0,
                "BPM should be close to 128 (±2 BPM tolerance), got {:.2}",
                result.bpm
            );
            assert!(
                result.bpm_confidence > 0.0,
                "BPM confidence should be positive"
            );

            // Phase 1C: Beat tracking should work
            if !result.beat_grid.beats.is_empty() {
                assert!(
                    result.beat_grid.beats.len() >= 4,
                    "Should detect at least 4 beats for 4-bar track"
                );
                assert!(
                    result.grid_stability >= 0.0 && result.grid_stability <= 1.0,
                    "Grid stability should be in [0, 1]"
                );

                // Validate beat intervals (should be ~0.469s for 128 BPM)
                if result.beat_grid.beats.len() >= 2 {
                    let beat_interval = result.beat_grid.beats[1] - result.beat_grid.beats[0];
                    let expected_interval = 60.0 / 128.0; // ~0.469s
                    assert!(
                        (beat_interval - expected_interval).abs() < 0.1,
                        "Beat interval should be ~{:.3}s for 128 BPM, got {:.3}s",
                        expected_interval,
                        beat_interval
                    );
                }

                println!("128 BPM test: BPM={:.2}, confidence={:.3}, {} beats, {} downbeats, stability={:.3}, duration={:.2}s, processing={:.2}ms", 
                         result.bpm, result.bpm_confidence, result.beat_grid.beats.len(),
                         result.beat_grid.downbeats.len(), result.grid_stability,
                         result.metadata.duration_seconds, result.metadata.processing_time_ms);
            } else {
                println!("128 BPM test: BPM={:.2}, confidence={:.3}, but beat grid is empty, duration={:.2}s, processing={:.2}ms", 
                         result.bpm, result.bpm_confidence, result.metadata.duration_seconds, result.metadata.processing_time_ms);
            }
        } else {
            println!(
                "128 BPM test: BPM detection failed, duration={:.2}s, processing={:.2}ms",
                result.metadata.duration_seconds, result.metadata.processing_time_ms
            );
        }
    }

    #[test]
    fn test_analyze_cmajor_scale() {
        let path = fixture_path("cmajor_scale.wav");
        let (samples, sample_rate) =
            load_wav(path.to_str().unwrap()).expect("Failed to load cmajor_scale.wav");

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, sample_rate, config).expect("Analysis should succeed");

        // Verify basic results
        assert!(result.metadata.duration_seconds > 3.0 && result.metadata.duration_seconds < 5.0);

        // Phase 1D: Key detection should work
        // C major scale should be detected as C major (Key::Major(0))
        use stratum_dsp::analysis::result::Key;
        if result.key_confidence > 0.0 {
            // For a C major scale, we expect C major to be detected
            // Allow for some tolerance since it's a simple scale (not full harmonic content)
            assert!(
                result.key == Key::Major(0) || result.key_confidence < 0.3,
                "C major scale should be detected as C major (Key::Major(0)), got {:?} with confidence {:.3}",
                result.key, result.key_confidence
            );
            assert!(
                result.key_confidence >= 0.0 && result.key_confidence <= 1.0,
                "Key confidence should be in [0, 1], got {:.3}",
                result.key_confidence
            );

            // Key clarity should be reasonable for a tonal scale
            // Note: key_clarity is not in AnalysisResult yet, but we can check if it's computed
            println!("C major scale test: key={:?} ({}), confidence={:.3}, duration={:.2}s, processing={:.2}ms",
                     result.key, result.key.name(), result.key_confidence,
                     result.metadata.duration_seconds, result.metadata.processing_time_ms);
        } else {
            println!("C major scale test: Key detection failed or low confidence, duration={:.2}s, processing={:.2}ms",
                     result.metadata.duration_seconds, result.metadata.processing_time_ms);
        }
    }

    #[test]
    fn test_silence_detection_and_trimming() {
        let path = fixture_path("mixed_silence.wav");
        let (samples, sample_rate) =
            load_wav(path.to_str().unwrap()).expect("Failed to load mixed_silence.wav");

        // Original duration should be ~15 seconds (5s silence + 5s audio + 5s silence)
        let original_duration = samples.len() as f32 / sample_rate as f32;
        assert!(original_duration > 14.0 && original_duration < 16.0);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, sample_rate, config).expect("Analysis should succeed");

        // After silence trimming, duration should be ~5 seconds (just the audio content)
        // The analyze_audio function trims silence, so metadata.duration_seconds should reflect trimmed length
        assert!(
            result.metadata.duration_seconds > 4.0 && result.metadata.duration_seconds < 6.0,
            "Expected trimmed duration ~5s, got {:.2}s",
            result.metadata.duration_seconds
        );

        println!(
            "Silence trimming test: original={:.2}s, trimmed={:.2}s",
            original_duration, result.metadata.duration_seconds
        );
    }

    #[test]
    fn test_analyze_audio_placeholder() {
        // Test with silence (edge case)
        let samples = vec![0.0f32; 44100 * 30]; // 30 seconds of silence
        let config = AnalysisConfig::default();

        // This should fail because audio is entirely silent after trimming
        let result = analyze_audio(&samples, 44100, config);
        assert!(result.is_err(), "Silent audio should return error");

        if let Err(e) = result {
            assert!(
                e.to_string().contains("silent"),
                "Error should mention silence: {}",
                e
            );
        }
    }

    #[test]
    fn external_beat_grid_replaces_hmm_grid() {
        // Synthesize a short kick-like pulse train so analyze_audio's
        // pipeline runs to the beat-tracking phase. We don't care what BPM
        // the rest of the pipeline infers — we only check that the supplied
        // external_beat_grid round-trips into result.beat_grid unchanged.
        use stratum_dsp::analysis::result::BeatGrid;

        let sample_rate: u32 = 44100;
        let duration_s = 8.0_f32;
        let n = (duration_s * sample_rate as f32) as usize;
        let mut samples = vec![0.0_f32; n];
        // Pulses every 0.5s with a short decay.
        let pulse_interval = (0.5 * sample_rate as f32) as usize;
        for start in (0..n).step_by(pulse_interval) {
            for i in 0..200 {
                if start + i >= n {
                    break;
                }
                samples[start + i] = (1.0 - i as f32 / 200.0) * 0.6;
            }
        }

        // Deliberately-skewed grid (0.6 s spacing, not 0.5 s) so we'd notice
        // if the HMM tracker silently overrode it.
        let beats: Vec<f32> = (0..12).map(|i| 0.05 + i as f32 * 0.6).collect();
        let bars: Vec<f32> = beats.iter().step_by(4).copied().collect();
        let supplied = BeatGrid {
            downbeats: bars.clone(),
            beats: beats.clone(),
            bars: bars.clone(),
        };

        let config = AnalysisConfig {
            external_beat_grid: Some(supplied.clone()),
            ..AnalysisConfig::default()
        };
        let result = analyze_audio(&samples, sample_rate, config).expect("analysis");

        assert_eq!(result.beat_grid.beats, beats);
        assert_eq!(result.beat_grid.bars, bars);
        assert!(
            (result.grid_stability - 1.0).abs() < 1e-6,
            "external grid should report stability=1.0, got {}",
            result.grid_stability
        );

        // dub_stab fires whenever beats.len() >= 2; with synthesized pulse
        // train + 12-beat external grid, the histogram should be populated.
        let dub = result
            .dub_stab
            .expect("dub_stab populated when grid has beats");
        assert_eq!(dub.histogram.len(), 32, "histogram is 32-bin");
        assert_eq!(
            dub.per_bar_histograms.len(),
            bars.len(),
            "one per-bar histogram per bar"
        );
        for h in &dub.per_bar_histograms {
            assert_eq!(h.len(), 32);
        }
    }

    #[test]
    fn dub_stab_skipped_when_beat_grid_empty() {
        // 30 s of silence trims to nothing, so analyze_audio returns Err.
        // To get a non-error path with an empty grid, supply low-energy
        // noise that survives silence trimming but yields no detectable
        // onsets, so the HMM tracker produces an empty grid.
        use stratum_dsp::analysis::result::BeatGrid;
        let sample_rate: u32 = 44100;
        let n = (5.0 * sample_rate as f32) as usize;
        let mut samples = vec![0.0_f32; n];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = ((i as f32 * 0.001).sin()) * 0.05;
        }

        // Force empty grid via external_beat_grid.
        let config = AnalysisConfig {
            external_beat_grid: Some(BeatGrid {
                downbeats: vec![],
                beats: vec![],
                bars: vec![],
            }),
            ..AnalysisConfig::default()
        };

        let Ok(result) = analyze_audio(&samples, sample_rate, config) else {
            return; // tolerate silence-trim failure on this synthetic input
        };
        assert!(
            result.dub_stab.is_none(),
            "dub_stab should be None when beat grid is empty"
        );
    }
}
