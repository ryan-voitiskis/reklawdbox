//! Shared test helpers for stratum-dsp integration tests.
//!
//! Lazily synthesizes the WAV fixtures referenced by `integration_tests.rs`
//! into a workspace-level cache directory (`target/test-fixtures/`). Generation
//! is deterministic and idempotent: a cached fixture is reused on subsequent
//! test runs, and concurrent generation is serialized by a process-local mutex.
//!
//! Rationale: the original test suite shipped four WAV fixtures under
//! `stratum-dsp/tests/fixtures/`, but `.gitignore` excludes `*.wav` etc. in
//! that directory, so the files are never committed and `cargo test
//! -p stratum-dsp` fails for any contributor on a clean clone. Generating them
//! from `hound` keeps the repo bloat-free while preserving test reproducibility.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static FIXTURE_LOCK: Mutex<()> = Mutex::new(());

pub fn fixture_path(name: &str) -> PathBuf {
    let cache_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("target")
        .join("test-fixtures");
    let path = cache_dir.join(name);

    if path.exists() {
        return path;
    }

    // Serialize generation so two tests racing on the same fixture don't both
    // half-write the file. Re-check existence after acquiring the lock in case
    // a sibling thread already produced it.
    let _guard = FIXTURE_LOCK
        .lock()
        .expect("fixture lock poisoned by a panic in a prior generator");
    if path.exists() {
        return path;
    }

    std::fs::create_dir_all(&cache_dir).expect("create test-fixtures cache dir");

    match name {
        "120bpm_4bar.wav" => synth_kick_track(&path, 120.0, 8.0),
        "128bpm_4bar.wav" => synth_kick_track(&path, 128.0, 7.5),
        "cmajor_scale.wav" => synth_cmajor_scale(&path, 0.5),
        "mixed_silence.wav" => synth_mixed_silence(&path, 5.0, 5.0, 5.0),
        other => panic!("unknown fixture name: {other}"),
    }

    path
}

const SAMPLE_RATE: u32 = 44100;

fn wav_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

fn write_i16(writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>, sample: f32) {
    let v = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
    writer.write_sample(v).expect("write sample");
}

/// Synthesize a steady kick-drum track: a 60 Hz sine wave with 40 ms
/// exponential decay, re-triggered every (60 / bpm) seconds. Produces strong
/// onsets that the BPM/beat-tracking pipeline can lock onto.
fn synth_kick_track(path: &Path, bpm: f32, duration_s: f32) {
    let mut writer = hound::WavWriter::create(path, wav_spec()).expect("create wav writer");
    let total_samples = (duration_s * SAMPLE_RATE as f32) as usize;
    let beat_interval = (60.0 / bpm * SAMPLE_RATE as f32) as usize;
    let decay_samples = (SAMPLE_RATE as f32 * 0.040) as usize;

    let mut buf = vec![0.0f32; total_samples];
    let mut beat_pos = 0;
    while beat_pos < total_samples {
        for i in 0..decay_samples {
            if beat_pos + i >= total_samples {
                break;
            }
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = (-t * 50.0).exp();
            let sine = (2.0 * std::f32::consts::PI * 60.0 * t).sin();
            buf[beat_pos + i] += sine * env * 0.8;
        }
        beat_pos += beat_interval;
    }

    for s in buf {
        write_i16(&mut writer, s);
    }
    writer.finalize().expect("finalize wav");
}

/// Synthesize a one-octave C major scale (C4 → C5) with each note `note_s`
/// long. Pure sines with 10 ms linear fade in/out to avoid click artifacts.
/// Used by key-detection tests; pure sines give weak but C-rooted chroma.
fn synth_cmajor_scale(path: &Path, note_s: f32) {
    let mut writer = hound::WavWriter::create(path, wav_spec()).expect("create wav writer");
    let notes_hz = [
        261.63, // C4
        293.66, // D4
        329.63, // E4
        349.23, // F4
        392.00, // G4
        440.00, // A4
        493.88, // B4
        523.25, // C5
    ];
    let note_samples = (note_s * SAMPLE_RATE as f32) as usize;
    let fade_samples = (SAMPLE_RATE as f32 * 0.010) as usize;

    for &freq in notes_hz.iter() {
        for i in 0..note_samples {
            let t = i as f32 / SAMPLE_RATE as f32;
            let env = if i < fade_samples {
                i as f32 / fade_samples as f32
            } else if i >= note_samples - fade_samples {
                (note_samples - i) as f32 / fade_samples as f32
            } else {
                1.0
            };
            let sample = (2.0 * std::f32::consts::PI * freq * t).sin() * env * 0.5;
            write_i16(&mut writer, sample);
        }
    }
    writer.finalize().expect("finalize wav");
}

/// Synthesize a track with leading silence, a kick-pulse audio region, and
/// trailing silence. Used by the silence-trimming test: total duration is
/// `lead + audio + trail` seconds; trimmed duration should equal `audio`.
fn synth_mixed_silence(path: &Path, lead_s: f32, audio_s: f32, trail_s: f32) {
    let mut writer = hound::WavWriter::create(path, wav_spec()).expect("create wav writer");
    let lead = (lead_s * SAMPLE_RATE as f32) as usize;
    let audio = (audio_s * SAMPLE_RATE as f32) as usize;
    let trail = (trail_s * SAMPLE_RATE as f32) as usize;

    for _ in 0..lead {
        writer.write_sample(0i16).expect("write silence");
    }

    let beat_interval = (SAMPLE_RATE as f32 * 0.5) as usize; // 120 BPM
    let pulse_len = (SAMPLE_RATE as f32 * 0.040) as usize; // 40 ms pulse
    for i in 0..audio {
        let in_beat = i % beat_interval;
        let sample = if in_beat < pulse_len {
            let t = in_beat as f32 / SAMPLE_RATE as f32;
            let env = (-t * 50.0).exp();
            let sine = (2.0 * std::f32::consts::PI * 60.0 * t).sin();
            sine * env * 0.6
        } else {
            0.0
        };
        write_i16(&mut writer, sample);
    }

    for _ in 0..trail {
        writer.write_sample(0i16).expect("write silence");
    }

    writer.finalize().expect("finalize wav");
}
