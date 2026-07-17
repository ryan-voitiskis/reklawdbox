use super::*;
use crate::adapters::rekordbox::anlz::{
    load_rekordbox_grid_for_path, load_rekordbox_grid_for_path_with_conn,
};

#[test]
fn grid_lookup_with_shared_connection_handles_empty_analysis_path() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE djmdContent (FolderPath TEXT, AnalysisDataPath TEXT);\
             INSERT INTO djmdContent VALUES ('/generated/track.wav', '');",
    )
    .unwrap();

    assert!(load_rekordbox_grid_for_path_with_conn(&conn, "/generated/track.wav").is_none());
}
use std::process::Stdio;

#[test]
fn grid_compatibility_wrapper_preserves_the_no_live_db_test_seam() {
    assert!(load_rekordbox_grid_for_path("/synthetic/not-read.flac").is_none());
}

#[test]
fn stratum_result_serialization_round_trip() {
    let result = StratumResult {
        bpm: 128.0,
        bpm_confidence: 0.95,
        key: "Am".to_string(),
        key_camelot: "8A".to_string(),
        key_confidence: 0.88,
        key_clarity: 0.72,
        grid_stability: 0.91,
        grid_source: "rekordbox".to_string(),
        duration_seconds: 300.5,
        processing_time_ms: 1234.5,
        analyzer_version: "stratum-dsp-1.0.0".to_string(),
        mod_centroid: Some(12.5),
        harmonic_proportion: Some(0.65),
        decay_mid_tau: Some(180.0),
        decay_mid_r2: Some(0.92),
        decay_high_tau: Some(95.0),
        decay_high_r2: Some(0.88),
        dub_stab_onset_count: Some(168),
        dub_stab_onset_rate: Some(1.87),
        dub_stab_rate_basis: Some("main_groove".to_string()),
        dub_stab_histogram: Some(vec![0.0; 32]),
        dub_stab_template: Some("offbeat_eighth".to_string()),
        dub_stab_template_score: Some(0.92),
        kick_pattern: Some("four_on_floor".to_string()),
        kick_pattern_confidence: Some(0.91),
        kick_kicks_per_bar: Some(4.0),
        kick_onset_count: Some(128),
        kick_rate_basis: Some("main_groove".to_string()),
        kick_histogram: Some(vec![0.0; 64]),
        sections: Some(vec![TrackSectionView {
            start_seconds: 0.0,
            end_seconds: 30.0,
            kind: "main_groove".to_string(),
            kick_band_rms: 1.5,
            broadband_rms: 0.5,
        }]),
        flags: vec!["MultimodalBpm".to_string()],
        warnings: vec!["Low key clarity".to_string()],
    };

    let json = serde_json::to_string(&result).expect("serialize should succeed");
    let back: StratumResult = serde_json::from_str(&json).expect("deserialize should succeed");

    assert!((back.bpm - 128.0).abs() < f64::EPSILON);
    assert!((back.bpm_confidence - 0.95).abs() < f64::EPSILON);
    assert_eq!(back.key, "Am");
    assert_eq!(back.key_camelot, "8A");
    assert!((back.key_confidence - 0.88).abs() < f64::EPSILON);
    assert!((back.key_clarity - 0.72).abs() < f64::EPSILON);
    assert!((back.grid_stability - 0.91).abs() < f64::EPSILON);
    assert!((back.duration_seconds - 300.5).abs() < f64::EPSILON);
    assert!((back.processing_time_ms - 1234.5).abs() < f64::EPSILON);
    assert_eq!(back.analyzer_version, "stratum-dsp-1.0.0");
    assert_eq!(back.flags, vec!["MultimodalBpm"]);
    assert_eq!(back.warnings, vec!["Low key clarity"]);
}

#[test]
fn stratum_result_shape_matches_schema_version() {
    assert_eq!(STRATUM_SCHEMA_VERSION, "21");

    let value = serde_json::to_value(StratumResult::default()).expect("serialize should succeed");
    let object = value
        .as_object()
        .expect("stratum result should serialize as object");
    let mut fields: Vec<&str> = object.keys().map(String::as_str).collect();
    fields.sort_unstable();

    assert_eq!(
        fields,
        vec![
            "analyzer_version",
            "bpm",
            "bpm_confidence",
            "decay_high_r2",
            "decay_high_tau",
            "decay_mid_r2",
            "decay_mid_tau",
            "dub_stab_histogram",
            "dub_stab_onset_count",
            "dub_stab_onset_rate",
            "dub_stab_rate_basis",
            "dub_stab_template",
            "dub_stab_template_score",
            "duration_seconds",
            "flags",
            "grid_source",
            "grid_stability",
            "harmonic_proportion",
            "key",
            "key_camelot",
            "key_clarity",
            "key_confidence",
            "kick_histogram",
            "kick_kicks_per_bar",
            "kick_onset_count",
            "kick_pattern",
            "kick_pattern_confidence",
            "kick_rate_basis",
            "mod_centroid",
            "processing_time_ms",
            "sections",
            "warnings",
        ]
    );
}

#[test]
fn stratum_notation_to_camelot_converts_all_major_keys() {
    assert_eq!(stratum_notation_to_camelot("1A"), "8B"); // C
    assert_eq!(stratum_notation_to_camelot("2A"), "9B"); // G
    assert_eq!(stratum_notation_to_camelot("3A"), "10B"); // D
    assert_eq!(stratum_notation_to_camelot("4A"), "11B"); // A
    assert_eq!(stratum_notation_to_camelot("5A"), "12B"); // E
    assert_eq!(stratum_notation_to_camelot("6A"), "1B"); // B
    assert_eq!(stratum_notation_to_camelot("7A"), "2B"); // F#
    assert_eq!(stratum_notation_to_camelot("8A"), "3B"); // C#
    assert_eq!(stratum_notation_to_camelot("9A"), "4B"); // G#
    assert_eq!(stratum_notation_to_camelot("10A"), "5B"); // D#
    assert_eq!(stratum_notation_to_camelot("11A"), "6B"); // A#
    assert_eq!(stratum_notation_to_camelot("12A"), "7B"); // F
}

#[test]
fn stratum_notation_to_camelot_converts_all_minor_keys() {
    assert_eq!(stratum_notation_to_camelot("1B"), "8A"); // Am
    assert_eq!(stratum_notation_to_camelot("2B"), "9A"); // Em
    assert_eq!(stratum_notation_to_camelot("3B"), "10A"); // Bm
    assert_eq!(stratum_notation_to_camelot("4B"), "11A"); // F#m
    assert_eq!(stratum_notation_to_camelot("5B"), "12A"); // C#m
    assert_eq!(stratum_notation_to_camelot("6B"), "1A"); // G#m
    assert_eq!(stratum_notation_to_camelot("7B"), "2A"); // D#m
    assert_eq!(stratum_notation_to_camelot("8B"), "3A"); // A#m
    assert_eq!(stratum_notation_to_camelot("9B"), "4A"); // Fm
    assert_eq!(stratum_notation_to_camelot("10B"), "5A"); // Cm
    assert_eq!(stratum_notation_to_camelot("11B"), "6A"); // Gm
    assert_eq!(stratum_notation_to_camelot("12B"), "7A"); // Dm
}

#[test]
fn stratum_notation_to_camelot_passes_through_invalid_input() {
    assert_eq!(stratum_notation_to_camelot(""), "");
    assert_eq!(stratum_notation_to_camelot("X"), "X");
    assert_eq!(stratum_notation_to_camelot("0A"), "0A");
    assert_eq!(stratum_notation_to_camelot("13A"), "13A");
}

#[test]
fn downmix_to_mono_truncates_to_shortest_channel() {
    let left = [0.25_f32, 0.50, 0.75];
    let right = [0.75_f32, 0.25];
    let planes: &[&[f32]] = &[&left, &right];

    let mono = downmix_to_mono(planes, |&v| v);

    assert_eq!(mono.len(), 2, "should use the shortest channel length");
    assert!((mono[0] - 0.50).abs() < f32::EPSILON);
    assert!((mono[1] - 0.375).abs() < f32::EPSILON);
}

#[test]
fn downmix_to_mono_single_channel_uses_all_frames() {
    let mono_src = [0.1_f32, 0.2, 0.3];
    let planes: &[&[f32]] = &[&mono_src];

    let mono = downmix_to_mono(planes, |&v| v);

    assert_eq!(mono, mono_src);
}

#[test]
fn downmix_to_mono_returns_empty_when_any_channel_is_empty() {
    let left: [f32; 0] = [];
    let right = [0.25_f32, 0.50, 0.75];
    let planes: &[&[f32]] = &[&left, &right];

    let mono = downmix_to_mono(planes, |&v| v);

    assert!(
        mono.is_empty(),
        "expected empty output when one channel has zero frames"
    );
}

#[test]
fn parse_essentia_stdout_trims_whitespace() {
    let parsed =
        parse_essentia_stdout(b"\n  {\"danceability\": 0.82, \"analyzer_version\": \"2.1\"}\n")
            .expect("valid JSON with whitespace should parse");
    assert_eq!(parsed.danceability, Some(0.82));
    assert_eq!(parsed.analyzer_version, "2.1");
}

#[test]
fn essentia_v3_runtime_manifest_rejects_mismatch_before_cache_write() {
    let exact = br#"{
        "analyzer_version":"2.1b6.dev1438",
        "runtime_manifest":{
            "python_version":"3.14.6",
            "python_implementation":"cpython",
            "essentia_version":"2.1b6.dev1438",
            "numpy_version":"2.5.1",
            "pyyaml_version":"6.0.3",
            "six_version":"1.17.0",
            "analyzer_contract":"essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14"
        }
    }"#;
    let parsed = parse_essentia_stdout(exact).unwrap();
    validate_runtime_manifest(&parsed).unwrap();

    let mut mismatched: serde_json::Value = serde_json::from_slice(exact).unwrap();
    mismatched["runtime_manifest"]["numpy_version"] = serde_json::json!("2.5.2");
    let mismatched = parse_essentia_stdout(&serde_json::to_vec(&mismatched).unwrap()).unwrap();
    let error = validate_runtime_manifest(&mismatched).unwrap_err();
    assert!(
        matches!(error, AudioError::Analysis(message) if message.contains("refusing schema-v3 cache write"))
    );

    let mut non_cpython: serde_json::Value = serde_json::from_slice(exact).unwrap();
    non_cpython["runtime_manifest"]["python_implementation"] = serde_json::json!("pypy");
    let non_cpython = parse_essentia_stdout(&serde_json::to_vec(&non_cpython).unwrap()).unwrap();
    assert!(validate_runtime_manifest(&non_cpython).is_err());
}

#[test]
fn parse_essentia_stdout_rejects_empty_output() {
    let err =
        parse_essentia_stdout(b"   \n").expect_err("empty output should produce a parse error");
    assert!(
        matches!(&err, AudioError::Parse(msg) if msg.contains("empty")),
        "error should be Parse mentioning empty stdout, got: {err}"
    );
}

#[tokio::test]
async fn run_essentia_reports_subprocess_start_failure() {
    let err = run_essentia("/definitely/missing/python", "/tmp/does-not-matter.wav")
        .await
        .expect_err("missing python binary should fail");
    assert!(
        matches!(&err, AudioError::Subprocess(msg) if msg.contains("Failed to start Essentia subprocess")),
        "expected Subprocess variant with startup failure context, got: {err}"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn run_essentia_handles_non_scalar_outputs_via_stereo_fallback() {
    use std::os::unix::fs::PermissionsExt;

    let python = ["python3", "/usr/bin/python3"]
        .into_iter()
        .find(|candidate| {
            std::process::Command::new(candidate)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
        .unwrap_or("python3");

    let tmp = tempfile::tempdir().expect("temp dir should be created");
    let essentia_pkg = tmp.path().join("essentia");
    std::fs::create_dir_all(&essentia_pkg).expect("essentia package dir should be created");

    std::fs::write(
        tmp.path().join("platform.py"),
        "def python_version():\n    return '3.14.6'\n",
    )
    .expect("fake platform module should be written");
    for (distribution_dir, name, version) in [
        (
            "essentia-2.1b6.dev1438.dist-info",
            "essentia",
            "2.1b6.dev1438",
        ),
        ("numpy-2.5.1.dist-info", "numpy", "2.5.1"),
        ("PyYAML-6.0.3.dist-info", "PyYAML", "6.0.3"),
        ("six-1.17.0.dist-info", "six", "1.17.0"),
    ] {
        let distribution = tmp.path().join(distribution_dir);
        std::fs::create_dir(&distribution).expect("fake distribution should be created");
        std::fs::write(
            distribution.join("METADATA"),
            format!("Metadata-Version: 2.1\nName: {name}\nVersion: {version}\n"),
        )
        .expect("fake distribution metadata should be written");
    }

    std::fs::write(
        essentia_pkg.join("__init__.py"),
        "__version__ = '2.1b6.dev1438'\n",
    )
    .expect("fake essentia __init__ should be written");

    std::fs::write(
        essentia_pkg.join("standard.py"),
        r#"
class MonoLoader:
    def __init__(self, filename, sampleRate=44100):
        self.filename = filename
        self.sampleRate = sampleRate
    def __call__(self):
        return [0.1, 0.2, 0.3, 0.4]

class Danceability:
    def __call__(self, audio):
        return [2.46]

class LoudnessEBUR128:
    def __call__(self, audio):
        if isinstance(audio, list) and len(audio) > 0 and isinstance(audio[0], tuple):
            return ([1.0, 2.0], [3.0], -14.5, 4.2)
        raise TypeError("Cannot convert data from type VECTOR_REAL to VECTOR_STEREOSAMPLE")

class StereoMuxer:
    def __call__(self, left, right):
        return list(zip(left, right))

class DynamicComplexity:
    def __call__(self, audio):
        return [3.4]

class Loudness:
    def __call__(self, audio):
        return 21696.25

class RhythmExtractor2013:
    def __init__(self, method="multifeature"):
        self.method = method
    def __call__(self, audio):
        return (119.02, [0, 1, 2, 3, 4, 5, 6, 7])

class OnsetRate:
    def __call__(self, audio):
        return ([0.18, 0.64, 1.02], 5.6)

class BeatsLoudness:
    def __init__(self, beats):
        self.beats = beats
    def __call__(self, audio):
        return (None, [[1.0], [0.8], [1.2], [1.1], [0.9], [0.7], [1.0], [0.95]])

class SpectralCentroidTime:
    def __call__(self, audio):
        return [100.0, 200.0]

class FrameGenerator:
    def __init__(self, audio, frameSize=2048, hopSize=1024):
        pass
    def __iter__(self):
        yield [0.1, 0.2, 0.3]
        yield [0.4, 0.5, 0.6]

class Windowing:
    def __init__(self, type='hann'):
        pass
    def __call__(self, frame):
        return frame

class Spectrum:
    def __call__(self, frame):
        return [abs(x) for x in frame]

class MFCC:
    def __init__(self, numberCoefficients=13):
        self.n = numberCoefficients
    def __call__(self, spec):
        bands = [0.5] * 40
        coeffs = [float(i * 0.1) for i in range(self.n)]
        return (bands, coeffs)

class SpectralContrast:
    def __call__(self, spec):
        return ([1.0, 2.0, 3.0, 4.0, 5.0, 6.0], [0.1, 0.2, 0.3, 0.4, 0.5, 0.6])

class SpectralPeaks:
    def __call__(self, spec):
        return ([100.0, 200.0, 300.0], [0.5, 0.3, 0.1])

class Dissonance:
    def __call__(self, freqs, mags):
        return 0.35

class Centroid:
    def __init__(self, range=22050):
        pass
    def __call__(self, spec):
        return 0.42

class Flux:
    def __call__(self, spec):
        return 0.15

class Intensity:
    def __call__(self, spec):
        return 0.65
"#,
    )
    .expect("fake essentia.standard should be written");

    std::fs::write(
            tmp.path().join("numpy.py"),
            r#"
import builtins as _builtins

__version__ = "2.5.1"

class _FakeArray:
    def __init__(self, value):
        self._flat = []
        self._rows = None
        if isinstance(value, (list, tuple)) and value and isinstance(value[0], (list, tuple)):
            self._rows = [list(r) for r in value]
            for r in self._rows:
                self._flat.extend(r)
        else:
            self._flatten(value)
        self.size = len(self._flat)

    def _flatten(self, value):
        if isinstance(value, _FakeArray):
            for item in value._flat:
                self._flatten(item)
            return
        if isinstance(value, (list, tuple)):
            for item in value:
                self._flatten(item)
            return
        self._flat.append(value)

    def reshape(self, *_):
        return self._flat

    def __pow__(self, exp):
        return _FakeArray([x ** exp for x in self._flat])

    def __iter__(self):
        return iter(self._flat)

    def __len__(self):
        return len(self._flat)

    def __getitem__(self, idx):
        return self._flat[idx]

def asarray(value):
    return _FakeArray(value)

def array(value):
    if isinstance(value, _FakeArray):
        return value
    return _FakeArray(value)

def sum(arr):
    if isinstance(arr, _FakeArray):
        return _builtins.sum(arr._flat)
    return _builtins.sum(arr)

def mean(arr, axis=None):
    if isinstance(arr, _FakeArray) and axis == 0 and arr._rows:
        ncols = len(arr._rows[0])
        nrows = len(arr._rows)
        return _FakeArray([_builtins.sum(arr._rows[r][c] for r in range(nrows)) / nrows for c in range(ncols)])
    if isinstance(arr, _FakeArray):
        vals = arr._flat
    elif isinstance(arr, (list, tuple)):
        vals = list(arr)
    else:
        return float(arr)
    return _builtins.sum(vals) / max(len(vals), 1)

def std(arr, axis=None):
    if isinstance(arr, _FakeArray) and axis == 0 and arr._rows:
        ncols = len(arr._rows[0])
        nrows = len(arr._rows)
        result = []
        for c in range(ncols):
            col = [arr._rows[r][c] for r in range(nrows)]
            m = _builtins.sum(col) / nrows
            var = _builtins.sum((x - m) ** 2 for x in col) / nrows
            result.append(var ** 0.5)
        return _FakeArray(result)
    if isinstance(arr, _FakeArray):
        vals = arr._flat
    elif isinstance(arr, (list, tuple)):
        vals = list(arr)
    else:
        return 0.0
    m = _builtins.sum(vals) / max(len(vals), 1)
    var = _builtins.sum((x - m) ** 2 for x in vals) / max(len(vals), 1)
    return var ** 0.5

def sort(arr):
    if isinstance(arr, _FakeArray):
        return _FakeArray(sorted(arr._flat))
    return _FakeArray(sorted(arr))

def percentile(arr, p):
    if isinstance(arr, _FakeArray):
        vals = sorted(arr._flat)
    else:
        vals = sorted(arr)
    if not vals:
        return 0.0
    k = (p / 100.0) * (len(vals) - 1)
    lo = int(k)
    hi = min(lo + 1, len(vals) - 1)
    frac = k - lo
    return vals[lo] + frac * (vals[hi] - vals[lo])

"#,
        )
        .expect("fake numpy module should be written");

    std::fs::write(tmp.path().join("yaml.py"), "__version__ = '6.0.3'\n")
        .expect("fake yaml module should be written");
    std::fs::write(tmp.path().join("six.py"), "__version__ = '1.17.0'\n")
        .expect("fake six module should be written");

    let wrapper = tmp.path().join("fake-python");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nPYTHONPATH='{}' exec '{}' \"$@\"\n",
            tmp.path().to_string_lossy(),
            python
        ),
    )
    .expect("python wrapper should be written");
    let mut perms = std::fs::metadata(&wrapper)
        .expect("wrapper metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&wrapper, perms).expect("wrapper should be executable");

    let result = run_essentia(
        wrapper
            .to_str()
            .expect("wrapper path should be valid UTF-8"),
        "/tmp/ignored.wav",
    )
    .await
    .expect("run_essentia should succeed with fake modules");

    assert_eq!(result.analyzer_version, "2.1b6.dev1438");
    assert_eq!(
        result
            .runtime_manifest
            .as_ref()
            .expect("runtime manifest should be emitted")
            .analyzer_contract,
        crate::adapters::audio::essentia_environment::ESSENTIA_CONTRACT_ID
    );
    assert_eq!(
        result.runtime_manifest.unwrap().python_implementation,
        "cpython"
    );
    assert!((result.danceability.unwrap() - 2.46).abs() < 1e-6);
    assert!((result.loudness_integrated.unwrap() - (-14.5)).abs() < 1e-6);
    assert!((result.loudness_range.unwrap() - 4.2).abs() < 1e-6);
    assert!((result.onset_rate.unwrap() - 5.6).abs() < 1e-6);
    assert!(
        result.rhythm_regularity.unwrap() > 0.0,
        "rhythm_regularity should be computed from beat loudness ratios"
    );

    // Frame-based features
    let mfcc = result
        .mfcc_mean
        .as_ref()
        .expect("mfcc_mean should be present");
    assert_eq!(mfcc.len(), 13, "mfcc_mean should have 13 coefficients");

    let contrast = result
        .spectral_contrast_mean
        .as_ref()
        .expect("spectral_contrast_mean should be present");
    assert_eq!(
        contrast.len(),
        6,
        "spectral_contrast_mean should have 6 bands"
    );

    let dissonance = result
        .dissonance_mean
        .expect("dissonance_mean should be present");
    assert!(
        dissonance > 0.0 && dissonance < 1.0,
        "dissonance should be in (0, 1), got {dissonance}"
    );

    let intensity = result
        .intensity_mean
        .expect("intensity_mean should be present");
    assert!(intensity > 0.0, "intensity_mean should be positive");
    assert!(
        result.intensity_var.is_some(),
        "intensity_var should be present"
    );

    // Phase 1: temporal statistics
    let mfcc_std = result
        .mfcc_std
        .as_ref()
        .expect("mfcc_std should be present");
    assert_eq!(mfcc_std.len(), 13, "mfcc_std should have 13 coefficients");

    assert!(
        result.spectral_centroid_cv.is_some(),
        "spectral_centroid_cv should be present"
    );
    assert!(
        result.spectral_flux_mean.is_some(),
        "spectral_flux_mean should be present"
    );
    assert!(
        result.spectral_flux_iqr.is_some(),
        "spectral_flux_iqr should be present"
    );
}

fn write_generated_wav(path: &std::path::Path, sample_rate: u32, seconds: u32) {
    let sample_count = sample_rate * seconds;
    let mut pcm = Vec::with_capacity(sample_count as usize * 2);
    for index in 0..sample_count {
        let phase = index as f32 / sample_rate as f32;
        let tone = (phase * 220.0 * std::f32::consts::TAU).sin() * 0.18;
        let beat_position = index % (sample_rate / 2);
        let click = if beat_position < 400 {
            (1.0 - beat_position as f32 / 400.0) * 0.75
        } else {
            0.0
        };
        let sample = ((tone + click).clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }

    let data_size = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_size).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(&pcm);
    std::fs::write(path, wav).unwrap();
}

#[test]
fn generated_wav_fixture_decodes_and_analyzes_without_private_audio() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("120-bpm-generated.wav");
    write_generated_wav(&path, 44_100, 8);

    let (samples, sample_rate) = decode_to_samples(path.to_str().unwrap()).unwrap();
    assert_eq!(sample_rate, 44_100);
    assert_eq!(samples.len(), 44_100 * 8);
    assert!(samples.iter().all(|sample| sample.is_finite()));

    let result = analyze_with_stratum(&samples, sample_rate, None).unwrap();
    assert!(result.bpm.is_finite() && result.bpm > 0.0);
    assert!(result.duration_seconds >= 7.9 && result.duration_seconds <= 8.1);
    assert!(!result.analyzer_version.is_empty());
}

// ==================== Integration tests (real audio files) ====================
// Run with: cargo test -- --ignored

#[test]
#[ignore]
fn test_real_audio_analysis() {
    let conn = crate::adapters::rekordbox::open_real_db().expect("backup tarball not found");
    let params = crate::adapters::rekordbox::SearchParams {
        query: None,
        artist: None,
        genre: None,
        rating_min: None,
        bpm_min: Some(120.0),
        bpm_max: Some(140.0),
        key: None,
        playlist: None,
        has_genre: Some(true),
        has_label: None,
        year_zero: None,
        label: None,
        path: None,
        path_prefix: None,
        added_after: None,
        added_before: None,
        exclude_samples: true,
        limit: Some(5),
        offset: None,
    };
    let tracks = crate::adapters::rekordbox::search_tracks(&conn, &params).unwrap();
    assert!(!tracks.is_empty(), "no tracks found for analysis test");

    let track = tracks
        .iter()
        .find(|t| {
            let path = &t.file_path;
            std::fs::metadata(path).is_ok()
                || percent_encoding::percent_decode_str(path)
                    .decode_utf8()
                    .ok()
                    .is_some_and(|d| std::fs::metadata(d.as_ref()).is_ok())
        })
        .expect("no track with accessible audio file found");

    let file_path = if std::fs::metadata(&track.file_path).is_ok() {
        track.file_path.clone()
    } else {
        percent_encoding::percent_decode_str(&track.file_path)
            .decode_utf8()
            .unwrap()
            .to_string()
    };

    eprintln!(
        "[integration] Analyzing: {} - {} ({})",
        track.artist, track.title, file_path
    );

    let (samples, sample_rate) =
        decode_to_samples(&file_path).unwrap_or_else(|e| panic!("decode failed: {e}"));

    assert!(!samples.is_empty(), "decoded zero samples from {file_path}");
    assert!(sample_rate > 0, "invalid sample rate from {file_path}");
    eprintln!(
        "[integration] Decoded: {} samples at {} Hz ({:.1}s)",
        samples.len(),
        sample_rate,
        samples.len() as f64 / sample_rate as f64
    );

    let result = analyze_with_stratum(&samples, sample_rate, None)
        .unwrap_or_else(|e| panic!("analysis failed: {e}"));

    assert!(
        result.bpm > 0.0,
        "BPM should be positive, got {}",
        result.bpm
    );
    assert!(
        result.bpm < 300.0,
        "BPM should be < 300, got {}",
        result.bpm
    );
    assert!(!result.key.is_empty(), "key should be non-empty");
    assert!(
        !result.key_camelot.is_empty(),
        "camelot key should be non-empty"
    );
    assert!(result.duration_seconds > 0.0, "duration should be positive");
    assert!(
        result.processing_time_ms > 0.0,
        "processing time should be positive"
    );
    assert!(
        !result.analyzer_version.is_empty(),
        "analyzer version should be non-empty"
    );

    eprintln!(
        "[integration] Result: BPM={:.2} (conf={:.2}), Key={} / {} (conf={:.2}, clarity={:.2}), grid={:.2}, {:.1}s in {:.0}ms",
        result.bpm,
        result.bpm_confidence,
        result.key,
        result.key_camelot,
        result.key_confidence,
        result.key_clarity,
        result.grid_stability,
        result.duration_seconds,
        result.processing_time_ms,
    );
}

#[test]
#[ignore]
fn test_audio_analysis_cache_round_trip() {
    let conn = crate::adapters::rekordbox::open_real_db().expect("backup tarball not found");
    let params = crate::adapters::rekordbox::SearchParams {
        query: None,
        artist: None,
        genre: None,
        rating_min: None,
        bpm_min: Some(120.0),
        bpm_max: Some(140.0),
        key: None,
        playlist: None,
        has_genre: Some(true),
        has_label: None,
        year_zero: None,
        label: None,
        path: None,
        path_prefix: None,
        added_after: None,
        added_before: None,
        exclude_samples: true,
        limit: Some(5),
        offset: None,
    };
    let tracks = crate::adapters::rekordbox::search_tracks(&conn, &params).unwrap();
    let track = tracks
        .iter()
        .find(|t| {
            let path = &t.file_path;
            std::fs::metadata(path).is_ok()
                || percent_encoding::percent_decode_str(path)
                    .decode_utf8()
                    .ok()
                    .is_some_and(|d| std::fs::metadata(d.as_ref()).is_ok())
        })
        .expect("no track with accessible audio file found");

    let file_path = if std::fs::metadata(&track.file_path).is_ok() {
        track.file_path.clone()
    } else {
        percent_encoding::percent_decode_str(&track.file_path)
            .decode_utf8()
            .unwrap()
            .to_string()
    };

    let (samples, sample_rate) = decode_to_samples(&file_path).unwrap();
    let result = analyze_with_stratum(&samples, sample_rate, None).unwrap();
    let features_json = serde_json::to_string(&result).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("test-cache.sqlite3");
    let store_conn = crate::adapters::state::open(store_path.to_str().unwrap()).unwrap();

    let metadata = std::fs::metadata(&file_path).unwrap();
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    crate::adapters::state::set_audio_analysis_with_fingerprint(
        &store_conn,
        &file_path,
        "stratum-dsp",
        file_size,
        file_mtime,
        STRATUM_SCHEMA_VERSION,
        STRATUM_HMM_INPUT_FINGERPRINT,
        &features_json,
    )
    .unwrap();

    let cached = crate::adapters::state::get_audio_analysis(&store_conn, &file_path, "stratum-dsp")
        .unwrap()
        .expect("should find cached entry");

    assert_eq!(cached.file_path, file_path);
    assert_eq!(cached.file_size, file_size);
    assert_eq!(cached.file_mtime, file_mtime);

    let cached_result: StratumResult = serde_json::from_str(&cached.features_json).unwrap();
    assert!((cached_result.bpm - result.bpm).abs() < f64::EPSILON);
    assert_eq!(cached_result.key, result.key);
    assert_eq!(cached_result.key_camelot, result.key_camelot);

    eprintln!(
        "[integration] Cache round-trip OK: BPM={:.2}, Key={}",
        cached_result.bpm, cached_result.key
    );
}
