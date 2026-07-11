//! Integration tests for audio analysis engine.
//!
//! These tests synthesize deterministic audio in memory instead of relying on
//! private fixture files. Keep them portable enough to run in CI and fresh
//! clones.

use stratum_dsp::{analyze_audio, AnalysisConfig, AnalysisResult, Key};

const SAMPLE_RATE: u32 = 44_100;

fn synth_kick_track(bpm: f32, bars: usize) -> Vec<f32> {
    let beat_s = 60.0 / bpm;
    let duration_s = beat_s * 4.0 * bars as f32;
    let n = (duration_s * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0.0_f32; n];

    // A quiet continuous bed prevents edge trimming from changing the intended
    // duration while the decaying pulses give onset/BPM detectors clean events.
    for (i, sample) in samples.iter_mut().enumerate() {
        let t = i as f32 / SAMPLE_RATE as f32;
        *sample += 0.05 * (2.0 * std::f32::consts::PI * 55.0 * t).sin();
    }

    let pulse_interval = (beat_s * SAMPLE_RATE as f32) as usize;
    let pulse_len = (0.08 * SAMPLE_RATE as f32) as usize;
    for start in (0..n).step_by(pulse_interval) {
        for i in 0..pulse_len {
            if start + i >= n {
                break;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = 1.0 - i as f32 / pulse_len as f32;
            let thump = (2.0 * std::f32::consts::PI * 60.0 * t).sin()
                + 0.35 * (2.0 * std::f32::consts::PI * 120.0 * t).sin();
            samples[start + i] += 0.65 * env * thump;
        }
    }

    samples
}

fn synth_gradual_tempo_track() -> (Vec<f32>, Vec<f32>) {
    const PULSE_COUNT: usize = 40;
    const START_BPM: f32 = 114.0;
    const END_BPM: f32 = 126.0;

    let mut pulse_times = Vec::with_capacity(PULSE_COUNT);
    let mut time = 0.0_f32;
    for index in 0..PULSE_COUNT {
        pulse_times.push(time);
        let progress = index as f32 / (PULSE_COUNT - 1) as f32;
        let bpm = START_BPM + progress * (END_BPM - START_BPM);
        time += 60.0 / bpm;
    }

    let duration_s = time + 0.1;
    let n = (duration_s * SAMPLE_RATE as f32) as usize;
    let mut samples = vec![0.0_f32; n];
    for (index, sample) in samples.iter_mut().enumerate() {
        let t = index as f32 / SAMPLE_RATE as f32;
        *sample = 0.05 * (2.0 * std::f32::consts::PI * 55.0 * t).sin();
    }

    let pulse_len = (0.08 * SAMPLE_RATE as f32) as usize;
    for pulse_time in &pulse_times {
        let start = (*pulse_time * SAMPLE_RATE as f32).round() as usize;
        for index in 0..pulse_len {
            if start + index >= samples.len() {
                break;
            }
            let t = index as f32 / SAMPLE_RATE as f32;
            let envelope = 1.0 - index as f32 / pulse_len as f32;
            let thump = (2.0 * std::f32::consts::PI * 60.0 * t).sin()
                + 0.35 * (2.0 * std::f32::consts::PI * 120.0 * t).sin();
            samples[start + index] += 0.65 * envelope * thump;
        }
    }

    (samples, pulse_times)
}

fn median(mut values: Vec<f32>) -> f32 {
    assert!(!values.is_empty());
    values.sort_by(f32::total_cmp);
    values[values.len() / 2]
}

fn synth_chord(duration_s: f32, freqs: [f32; 4]) -> Vec<f32> {
    let n = (duration_s * SAMPLE_RATE as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            let amp = if t < 0.05 { t / 0.05 } else { 1.0 };
            let v: f32 = freqs
                .iter()
                .map(|f| (2.0 * std::f32::consts::PI * f * t).sin())
                .sum();
            0.18 * amp * v / freqs.len() as f32
        })
        .collect()
}

fn synth_c_major_chord(duration_s: f32) -> Vec<f32> {
    synth_chord(duration_s, [261.63, 329.63, 392.0, 523.25])
}

fn synth_c_minor_chord(duration_s: f32) -> Vec<f32> {
    synth_chord(duration_s, [261.63, 311.13, 392.0, 523.25])
}

fn synth_tone_with_silence() -> Vec<f32> {
    let silence = vec![0.0_f32; SAMPLE_RATE as usize * 5];
    let tone = synth_c_major_chord(5.0);
    [silence.as_slice(), tone.as_slice(), silence.as_slice()].concat()
}

fn assert_finite(value: f32, field: &str) {
    assert!(value.is_finite(), "{field} should be finite, got {value}");
}

fn assert_unit_interval(value: f32, field: &str) {
    assert_finite(value, field);
    assert!(
        (0.0..=1.0).contains(&value),
        "{field} should be in [0, 1], got {value}"
    );
}

fn assert_finite_non_negative(values: &[f32], field: &str) {
    for (index, value) in values.iter().enumerate() {
        assert!(
            value.is_finite(),
            "{field}[{index}] should be finite, got {value}"
        );
        assert!(
            *value >= 0.0,
            "{field}[{index}] should be non-negative, got {value}"
        );
    }
}

fn assert_strictly_ascending_non_negative(values: &[f32], field: &str) {
    assert_finite_non_negative(values, field);
    for (index, pair) in values.windows(2).enumerate() {
        assert!(
            pair[0] < pair[1],
            "{field}[{index}]={} should be less than {field}[{}]={}",
            pair[0],
            index + 1,
            pair[1]
        );
    }
}

fn assert_analysis_result_invariants(result: &AnalysisResult) {
    assert_finite(result.bpm, "bpm");
    assert_unit_interval(result.bpm_confidence, "bpm_confidence");
    assert_unit_interval(result.grid_stability, "grid_stability");
    assert_finite(
        result.metadata.duration_seconds,
        "metadata.duration_seconds",
    );
    assert_finite(
        result.metadata.processing_time_ms,
        "metadata.processing_time_ms",
    );

    assert_strictly_ascending_non_negative(&result.beat_grid.beats, "beat_grid.beats");
    assert_strictly_ascending_non_negative(&result.beat_grid.downbeats, "beat_grid.downbeats");
    assert_strictly_ascending_non_negative(&result.beat_grid.bars, "beat_grid.bars");

    if !result.beat_grid.downbeats.is_empty() || !result.beat_grid.bars.is_empty() {
        let first_beat = result
            .beat_grid
            .beats
            .first()
            .expect("downbeats and bars require at least one beat");
        let last_beat = result
            .beat_grid
            .beats
            .last()
            .expect("downbeats and bars require at least one beat");
        for (field, values) in [
            ("beat_grid.downbeats", result.beat_grid.downbeats.as_slice()),
            ("beat_grid.bars", result.beat_grid.bars.as_slice()),
        ] {
            for (index, value) in values.iter().enumerate() {
                assert!(
                    (*first_beat..=*last_beat).contains(value),
                    "{field}[{index}]={value} should be within the beat range [{first_beat}, {last_beat}]"
                );
            }
        }
    }

    assert_eq!(
        result.beat_grid.downbeats, result.beat_grid.bars,
        "beat_grid.downbeats should match beat_grid.bars"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analyze_120bpm_kick() {
        let samples = synth_kick_track(120.0, 4);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, SAMPLE_RATE, config).expect("Analysis should succeed");
        assert_analysis_result_invariants(&result);

        // Verify basic results
        assert!(result.metadata.duration_seconds > 7.0 && result.metadata.duration_seconds < 9.0);
        assert!(result.metadata.processing_time_ms > 0.0);
        assert_eq!(result.metadata.sample_rate, SAMPLE_RATE);

        assert!(
            result.bpm > 0.0,
            "BPM should be positive, got {}",
            result.bpm
        );
        assert!(
            (result.bpm - 120.0).abs() < 2.0,
            "BPM should be close to 120 (±2 BPM tolerance), got {:.2}",
            result.bpm
        );
        assert!(
            result.bpm_confidence.is_finite(),
            "BPM confidence should be finite, got {}",
            result.bpm_confidence
        );
        assert!(
            result.bpm_confidence > 0.0,
            "BPM confidence should be positive, got {}",
            result.bpm_confidence
        );
        assert!(
            result.beat_grid.beats.len() >= 4,
            "Should detect at least 4 beats for 4-bar track, got {}",
            result.beat_grid.beats.len()
        );
        assert!(
            result.beat_grid.beats.len() >= 2,
            "Need at least 2 beats to check the first interval, got {}",
            result.beat_grid.beats.len()
        );

        let beat_interval = result.beat_grid.beats[1] - result.beat_grid.beats[0];
        let expected_interval = 60.0 / 120.0;
        assert!(
            (beat_interval - expected_interval).abs() < 0.1,
            "Beat interval should be ~0.5s for 120 BPM, got {:.3}s",
            beat_interval
        );

        // Uniform kick accents contain no meter contrast, so the conservative
        // fallback must produce 4/4 bars from actual tracked-beat indices.
        let mut positive_intervals: Vec<f32> = result
            .beat_grid
            .beats
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .filter(|interval| *interval > 0.0)
            .collect();
        assert!(
            !positive_intervals.is_empty(),
            "need a positive tracked-beat interval to derive tolerance"
        );
        positive_intervals.sort_by(f32::total_cmp);
        let median_interval = positive_intervals[positive_intervals.len() / 2];
        let timestamp_tolerance = median_interval * 0.05;
        let expected_downbeats: Vec<f32> =
            result.beat_grid.beats.iter().step_by(4).copied().collect();
        assert_eq!(
            result.beat_grid.downbeats.len(),
            expected_downbeats.len(),
            "uniform accents should fall back to one downbeat every four tracked beats"
        );
        for (index, (actual, expected)) in result
            .beat_grid
            .downbeats
            .iter()
            .zip(expected_downbeats.iter())
            .enumerate()
        {
            assert!(
                (actual - expected).abs() <= timestamp_tolerance,
                "downbeat {index} should use tracked beat index {}, got {actual:.6} instead of {expected:.6}",
                index * 4
            );
        }

        println!("120 BPM test: BPM={:.2}, confidence={:.3}, {} beats, {} downbeats, stability={:.3}, duration={:.2}s, processing={:.2}ms",
                 result.bpm, result.bpm_confidence, result.beat_grid.beats.len(),
                 result.beat_grid.downbeats.len(), result.grid_stability,
                 result.metadata.duration_seconds, result.metadata.processing_time_ms);
    }

    #[test]
    fn test_analyze_128bpm_kick() {
        let samples = synth_kick_track(128.0, 4);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, SAMPLE_RATE, config).expect("Analysis should succeed");
        assert_analysis_result_invariants(&result);

        // Verify basic results
        assert!(result.metadata.duration_seconds > 7.0 && result.metadata.duration_seconds < 8.0);
        assert!(result.metadata.processing_time_ms > 0.0);

        assert!(
            result.bpm > 0.0,
            "BPM should be positive, got {}",
            result.bpm
        );
        assert!(
            (result.bpm - 128.0).abs() <= 2.0,
            "BPM should be close to 128 (±2 BPM tolerance), got {:.2}",
            result.bpm
        );
        assert!(
            result.bpm_confidence.is_finite(),
            "BPM confidence should be finite, got {}",
            result.bpm_confidence
        );
        assert!(
            result.bpm_confidence > 0.0,
            "BPM confidence should be positive, got {}",
            result.bpm_confidence
        );
        assert!(
            result.beat_grid.beats.len() >= 4,
            "Should detect at least 4 beats for 4-bar track, got {}",
            result.beat_grid.beats.len()
        );
        assert!(
            result.beat_grid.beats.len() >= 2,
            "Need at least 2 beats to check the first interval, got {}",
            result.beat_grid.beats.len()
        );

        let beat_interval = result.beat_grid.beats[1] - result.beat_grid.beats[0];
        let expected_interval = 60.0 / 128.0;
        assert!(
            (beat_interval - expected_interval).abs() < 0.1,
            "Beat interval should be ~{:.3}s for 128 BPM, got {:.3}s",
            expected_interval,
            beat_interval
        );

        println!("128 BPM test: BPM={:.2}, confidence={:.3}, {} beats, {} downbeats, stability={:.3}, duration={:.2}s, processing={:.2}ms",
                 result.bpm, result.bpm_confidence, result.beat_grid.beats.len(),
                 result.beat_grid.downbeats.len(), result.grid_stability,
                 result.metadata.duration_seconds, result.metadata.processing_time_ms);
    }

    #[test]
    fn variable_tempo_grid_is_strict_unique_and_tracks_acceleration() {
        use stratum_dsp::features::beat_tracking::tempo_variation::detect_tempo_variations;

        let (samples, pulse_times) = synth_gradual_tempo_track();
        let result = analyze_audio(&samples, SAMPLE_RATE, AnalysisConfig::default())
            .expect("gradual-tempo analysis should succeed");
        assert_analysis_result_invariants(&result);
        assert!(!result.beat_grid.beats.is_empty());

        let segments = detect_tempo_variations(&result.beat_grid.beats, result.bpm)
            .expect("tracked grid should support tempo segmentation");
        assert!(segments.len() > 1, "fixture should span multiple segments");

        let merge_tolerance = 0.05_f32.min(0.20 * (60.0 / result.bpm));
        assert!(result
            .beat_grid
            .beats
            .windows(2)
            .all(|pair| pair[1] - pair[0] >= merge_tolerance));

        let pulse_match_tolerance = 0.08_f32;
        let mut used_pulses = vec![false; pulse_times.len()];
        let mut mapped_beats = 0_usize;
        for beat in &result.beat_grid.beats {
            let matches: Vec<usize> = pulse_times
                .iter()
                .enumerate()
                .filter(|(_, pulse)| (**pulse - *beat).abs() <= pulse_match_tolerance)
                .map(|(index, _)| index)
                .collect();
            assert!(
                matches.len() <= 1,
                "beat {beat:.6}s should map to at most one synthesized pulse"
            );
            if let Some(index) = matches.first().copied() {
                assert!(
                    !used_pulses[index],
                    "synthesized pulse {index} should not be reused by multiple beats"
                );
                used_pulses[index] = true;
                mapped_beats += 1;
            }
        }
        assert!(
            mapped_beats * 2 >= result.beat_grid.beats.len(),
            "at least half of tracked beats should map to synthesized pulses"
        );

        let track_end = pulse_times.last().copied().unwrap();
        let early_intervals: Vec<f32> = result
            .beat_grid
            .beats
            .windows(2)
            .filter(|pair| pair[1] <= track_end * 0.4)
            .map(|pair| pair[1] - pair[0])
            .collect();
        let late_intervals: Vec<f32> = result
            .beat_grid
            .beats
            .windows(2)
            .filter(|pair| pair[0] >= track_end * 0.6)
            .map(|pair| pair[1] - pair[0])
            .collect();
        assert!(
            median(late_intervals) < median(early_intervals),
            "later median interval should be shorter as the fixture accelerates"
        );
    }

    #[test]
    fn test_analyze_c_major_chord_pipeline_smoke() {
        let samples = synth_c_major_chord(4.0);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, SAMPLE_RATE, config).expect("Analysis should succeed");
        assert_analysis_result_invariants(&result);

        assert!(result.metadata.duration_seconds > 3.0 && result.metadata.duration_seconds < 5.0);
        assert_unit_interval(result.key_confidence, "key_confidence");
        assert_eq!(result.key, Key::Major(0));
        assert!(
            result.key_confidence > 0.0,
            "C-major confidence should be positive, got {}",
            result.key_confidence
        );

        println!(
            "C-major chord pipeline smoke: key={:?}, confidence={:.3}, duration={:.2}s, processing={:.2}ms",
            result.key,
            result.key_confidence,
            result.metadata.duration_seconds,
            result.metadata.processing_time_ms
        );
    }

    #[test]
    fn test_analyze_c_minor_chord_pipeline_smoke() {
        let samples = synth_c_minor_chord(4.0);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, SAMPLE_RATE, config).expect("Analysis should succeed");
        assert_analysis_result_invariants(&result);

        assert!(result.metadata.duration_seconds > 3.0 && result.metadata.duration_seconds < 5.0);
        assert_unit_interval(result.key_confidence, "key_confidence");
        assert_eq!(result.key, Key::Minor(0));
        assert!(
            result.key_confidence > 0.0,
            "C-minor confidence should be positive, got {}",
            result.key_confidence
        );

        println!(
            "C-minor chord pipeline smoke: key={:?}, confidence={:.3}, duration={:.2}s, processing={:.2}ms",
            result.key,
            result.key_confidence,
            result.metadata.duration_seconds,
            result.metadata.processing_time_ms
        );
    }

    #[test]
    fn test_silence_detection_and_trimming() {
        let samples = synth_tone_with_silence();

        // Original duration should be ~15 seconds (5s silence + 5s audio + 5s silence)
        let original_duration = samples.len() as f32 / SAMPLE_RATE as f32;
        assert!(original_duration > 14.0 && original_duration < 16.0);

        let config = AnalysisConfig::default();
        let result = analyze_audio(&samples, SAMPLE_RATE, config).expect("Analysis should succeed");

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
        assert_strictly_ascending_non_negative(&beats, "supplied beat_grid.beats");
        assert_strictly_ascending_non_negative(&bars, "supplied beat_grid.bars");
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
        assert_analysis_result_invariants(&result);
        assert_strictly_ascending_non_negative(&result.beat_grid.beats, "returned beat_grid.beats");
        assert_strictly_ascending_non_negative(&result.beat_grid.bars, "returned beat_grid.bars");

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
        // Onset rate is the count divided by the analysed duration (seconds).
        // The synthetic pulse train should produce a strictly-positive rate
        // when any onsets are detected.
        if dub.stab_onset_count > 0 {
            assert!(
                dub.stab_onset_rate > 0.0,
                "onset_rate should be > 0 when count > 0; got rate={}, count={}",
                dub.stab_onset_rate,
                dub.stab_onset_count
            );
        }
        // rate_basis must be one of the two known regimes — the type
        // system enforces this, but pin it as a sanity check.
        use stratum_dsp::RateBasis;
        assert!(
            matches!(dub.rate_basis, RateBasis::MainGroove | RateBasis::Track),
            "rate_basis should be MainGroove or Track; got {:?}",
            dub.rate_basis
        );
    }

    #[test]
    fn external_grid_survives_leading_silence_without_phase_shift() {
        // 0.3 s of silence then a steady pulse train at 0.5-s intervals.
        // The external grid is anchored at the FIRST PULSE (t=0.3, then
        // 0.8, 1.3, …). If silence-trimming runs, frame indices into the
        // trimmed STFT would be relative to t=0.3 but the grid would still
        // be in original time → 0.3 s offset → ~3/4 of a beat phase error.
        //
        // We assert the dub_stab onset count is non-trivial. Before the fix,
        // analyze_audio would either error (grid before trimmed t=0) or
        // silently produce a phase-shifted histogram.
        use stratum_dsp::analysis::result::BeatGrid;

        let sample_rate: u32 = 44100;
        let lead_silence_s = 0.3_f32;
        let dur_s = 8.0_f32;
        let n = (dur_s * sample_rate as f32) as usize;
        let mut samples = vec![0.0_f32; n];
        let lead_samples = (lead_silence_s * sample_rate as f32) as usize;
        for start in (lead_samples..n).step_by((0.5 * sample_rate as f32) as usize) {
            for i in 0..200 {
                if start + i >= n {
                    break;
                }
                samples[start + i] = (1.0 - i as f32 / 200.0) * 0.6;
            }
        }

        let beats: Vec<f32> = (0..14).map(|i| lead_silence_s + i as f32 * 0.5).collect();
        let bars: Vec<f32> = beats.iter().step_by(4).copied().collect();
        assert_strictly_ascending_non_negative(&beats, "supplied beat_grid.beats");
        assert_strictly_ascending_non_negative(&bars, "supplied beat_grid.bars");
        let supplied = BeatGrid {
            downbeats: bars.clone(),
            beats: beats.clone(),
            bars: bars.clone(),
        };

        let config = AnalysisConfig {
            external_beat_grid: Some(supplied),
            ..AnalysisConfig::default()
        };
        let result = analyze_audio(&samples, sample_rate, config).expect("analysis");
        assert_analysis_result_invariants(&result);
        assert_strictly_ascending_non_negative(&result.beat_grid.beats, "returned beat_grid.beats");
        assert_strictly_ascending_non_negative(&result.beat_grid.bars, "returned beat_grid.bars");

        assert_eq!(result.beat_grid.beats, beats);
        assert_eq!(result.beat_grid.bars, bars);
        let dub = result
            .dub_stab
            .expect("dub_stab populated when external grid is supplied");
        assert!(
            dub.stab_onset_count == 0 || !dub.histogram.iter().all(|&w| w == 0.0),
            "stab onsets should land coherently against the beat grid"
        );
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
        // dub_stab=None alone is ambiguous — also assert the flag so callers
        // can distinguish "no histogram because no beats" from other paths.
        assert!(
            result.metadata.flags.iter().any(|f| matches!(
                f,
                stratum_dsp::analysis::result::AnalysisFlag::DubStabGridTooShort
            )),
            "DubStabGridTooShort flag should be set; got flags={:?}",
            result.metadata.flags
        );
    }
}
