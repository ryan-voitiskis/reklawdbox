use serde::{Deserialize, Serialize};
use std::time::Instant;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StratumResult {
    pub bpm: f64,
    pub bpm_confidence: f64,
    pub key: String,
    pub key_camelot: String,
    pub key_confidence: f64,
    pub key_clarity: f64,
    pub grid_stability: f64,
    pub duration_seconds: f64,
    pub processing_time_ms: f64,
    pub analyzer_version: String,
    pub flags: Vec<String>,
    pub warnings: Vec<String>,
}

/// Audio file extensions accepted by all directory scanners.
pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["flac", "wav", "mp3", "m4a", "aac", "aiff"];

/// Canonical analyzer name for stratum-dsp (used as DB cache key).
pub const ANALYZER_STRATUM: &str = "stratum-dsp";
/// Canonical analyzer name for Essentia (used as DB cache key).
pub const ANALYZER_ESSENTIA: &str = "essentia";

const ESSENTIA_TIMEOUT_SECS: u64 = 300;

const ESSENTIA_SCRIPT: &str = r#"
import json
import sys
import essentia
import essentia.standard as es
import numpy as np

audio = es.MonoLoader(filename=sys.argv[1], sampleRate=44100)()
features = {}

def first_scalar_or_none(value):
    if value is None:
        return None
    if isinstance(value, (list, tuple)):
        for item in value:
            scalar = first_scalar_or_none(item)
            if scalar is not None:
                return scalar
        return None
    try:
        arr = np.asarray(value)
        if arr.size > 0:
            return float(arr.reshape(-1)[0])
    except Exception:
        pass
    try:
        return float(value)
    except Exception:
        return None

features["danceability"] = first_scalar_or_none(es.Danceability()(audio))

try:
    ebu = es.LoudnessEBUR128()(audio)
except TypeError:
    # Some Essentia builds require VECTOR_STEREOSAMPLE for EBU R128.
    # For mono sources, duplicate channel data to synthesize stereo.
    try:
        stereo_audio = np.column_stack((audio, audio))
        ebu = es.LoudnessEBUR128()(stereo_audio)
    except Exception:
        ebu = None

if ebu is not None:
    if isinstance(ebu, (tuple, list)):
        # Typical output is (momentary, short_term, integrated, loudness_range).
        # Prefer integrated/range slots when present, otherwise fallback to first two scalars.
        if len(ebu) >= 4:
            integrated = first_scalar_or_none(ebu[2])
            loudness_range = first_scalar_or_none(ebu[3])
        else:
            integrated = None
            loudness_range = None

        if integrated is None or loudness_range is None:
            scalar_values = [first_scalar_or_none(v) for v in ebu]
            scalar_values = [v for v in scalar_values if v is not None]
            if integrated is None:
                integrated = scalar_values[0] if len(scalar_values) > 0 else None
            if loudness_range is None:
                loudness_range = scalar_values[1] if len(scalar_values) > 1 else None

        features["loudness_integrated"] = integrated
        features["loudness_range"] = loudness_range
    else:
        features["loudness_integrated"] = first_scalar_or_none(ebu)
        features["loudness_range"] = None
else:
    features["loudness_integrated"] = None
    features["loudness_range"] = None

features["dynamic_complexity"] = first_scalar_or_none(es.DynamicComplexity()(audio))
features["average_loudness"] = first_scalar_or_none(es.Loudness()(audio))

rhythm = es.RhythmExtractor2013(method="multifeature")(audio)
features["bpm_essentia"] = first_scalar_or_none(rhythm[0] if isinstance(rhythm, (tuple, list)) and len(rhythm) > 0 else rhythm)
onset_result = es.OnsetRate()(audio)
if isinstance(onset_result, (tuple, list)) and len(onset_result) >= 2:
    features["onset_rate"] = first_scalar_or_none(onset_result[1])
else:
    features["onset_rate"] = first_scalar_or_none(onset_result)

beats = rhythm[1]
if len(beats) > 4:
    bl = es.BeatsLoudness(beats=beats)(audio)
    band_ratios = bl[1]
    if len(band_ratios) > 0:
        downbeat_values = [band_ratios[i][0] for i in range(0, len(band_ratios), 4)]
        all_values = [row[0] for row in band_ratios]
        downbeat_energy = sum(downbeat_values) / max(len(downbeat_values), 1)
        all_energy = sum(all_values) / max(len(all_values), 1)
        features["rhythm_regularity"] = float(downbeat_energy / max(all_energy, 1e-6))
    else:
        features["rhythm_regularity"] = None
else:
    features["rhythm_regularity"] = None

spectral_centroid = es.SpectralCentroidTime()(audio)
if hasattr(spectral_centroid, "__len__") and len(spectral_centroid) > 0:
    features["spectral_centroid_mean"] = float(sum(spectral_centroid) / len(spectral_centroid))
else:
    features["spectral_centroid_mean"] = None

# --- Frame-based features (shared loop) ---
frame_size = 2048
hop_size = 1024
windowing = es.Windowing(type='hann')
spectrum_algo = es.Spectrum()
mfcc_algo = es.MFCC(numberCoefficients=13)
contrast_algo = es.SpectralContrast()
peaks_algo = es.SpectralPeaks()
dissonance_algo = es.Dissonance()

try:
    intensity_algo = es.Intensity()
    has_intensity = True
except Exception:
    has_intensity = False

mfcc_accum = []
contrast_accum = []
dissonance_accum = []
intensity_values = []

for frame in es.FrameGenerator(audio, frameSize=frame_size, hopSize=hop_size):
    windowed = windowing(frame)
    spec = spectrum_algo(windowed)

    _, mfcc_coeffs = mfcc_algo(spec)
    mfcc_accum.append(mfcc_coeffs)

    sc = contrast_algo(spec)
    if isinstance(sc, (tuple, list)) and len(sc) >= 1:
        coeffs = sc[0]
        if hasattr(coeffs, '__len__') and len(coeffs) > 0:
            contrast_accum.append([float(x) for x in coeffs])

    freqs, mags = peaks_algo(spec)
    if len(freqs) > 1:
        diss = dissonance_algo(freqs, mags)
        diss_val = first_scalar_or_none(diss)
        if diss_val is not None:
            dissonance_accum.append(diss_val)

    if has_intensity:
        try:
            power_spec = [x**2 for x in spec]
            try:
                val = intensity_algo(power_spec)
            except TypeError:
                val = intensity_algo(frame)
            int_val = first_scalar_or_none(val)
            if int_val is not None:
                intensity_values.append(int_val)
        except Exception:
            pass

if mfcc_accum:
    features["mfcc_mean"] = [float(sum(c[i] for c in mfcc_accum) / len(mfcc_accum)) for i in range(13)]
else:
    features["mfcc_mean"] = None

if contrast_accum:
    n_bands = len(contrast_accum[0])
    features["spectral_contrast_mean"] = [sum(c[i] for c in contrast_accum) / len(contrast_accum) for i in range(n_bands)]
else:
    features["spectral_contrast_mean"] = None

if dissonance_accum:
    features["dissonance_mean"] = sum(dissonance_accum) / len(dissonance_accum)
else:
    features["dissonance_mean"] = None

if intensity_values:
    features["intensity_mean"] = sum(intensity_values) / len(intensity_values)
    mean = features["intensity_mean"]
    features["intensity_var"] = sum((v - mean)**2 for v in intensity_values) / len(intensity_values)
else:
    features["intensity_mean"] = None
    features["intensity_var"] = None

features["analyzer_version"] = essentia.__version__

json.dump(features, sys.stdout)
"#;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EssentiaOutput {
    pub analyzer_version: String,
    pub danceability: Option<f64>,
    pub loudness_integrated: Option<f64>,
    pub loudness_range: Option<f64>,
    pub dynamic_complexity: Option<f64>,
    pub average_loudness: Option<f64>,
    pub bpm_essentia: Option<f64>,
    pub onset_rate: Option<f64>,
    pub rhythm_regularity: Option<f64>,
    pub spectral_centroid_mean: Option<f64>,
    pub dissonance_mean: Option<f64>,
    pub intensity_mean: Option<f64>,
    pub intensity_var: Option<f64>,
    pub mfcc_mean: Option<Vec<f64>>,
    pub spectral_contrast_mean: Option<Vec<f64>>,
}

fn parse_essentia_stdout(stdout: &[u8]) -> Result<EssentiaOutput, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| format!("Essentia stdout was not valid UTF-8: {e}"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Essentia stdout was empty".to_string());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("Failed to parse Essentia JSON output: {e}"))
}

pub async fn run_essentia(
    python_path: &str,
    audio_path: &str,
) -> Result<EssentiaOutput, String> {
    let mut command = Command::new(python_path);
    command.args(["-c", ESSENTIA_SCRIPT, audio_path]);
    command.kill_on_drop(true);

    let output = timeout(Duration::from_secs(ESSENTIA_TIMEOUT_SECS), command.output())
        .await
        .map_err(|_| {
            format!(
                "Essentia analysis timed out after {}s for '{}'",
                ESSENTIA_TIMEOUT_SECS, audio_path
            )
        })?
        .map_err(|e| format!("Failed to start Essentia subprocess: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stderr = if stderr.is_empty() {
            "(no stderr output)".to_string()
        } else {
            stderr
        };
        return Err(format!(
            "Essentia subprocess failed for '{}': {}",
            audio_path, stderr
        ));
    }

    parse_essentia_stdout(&output.stdout)
}

/// Resolve a Rekordbox file path to an actual filesystem path.
/// Tries the raw path first; if that fails, tries percent-decoding.
pub(crate) fn resolve_audio_path(raw_path: &str) -> Result<String, String> {
    if std::fs::metadata(raw_path).is_ok() {
        return Ok(raw_path.to_string());
    }

    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|e| format!("Invalid UTF-8 in file path: {e}"))?
        .to_string();

    if std::fs::metadata(&decoded).is_ok() {
        return Ok(decoded);
    }

    Err(format!(
        "File not found (tried raw and decoded): {raw_path}"
    ))
}

pub fn decode_to_samples(path: &str) -> Result<(Vec<f32>, u32), String> {
    let file = std::fs::File::open(path)
        .map_err(|e| format!("Failed to open audio file '{path}': {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
    {
        hint.with_extension(ext);
    }

    let probed = get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Failed to probe audio format: {e}"))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track found in file".to_string())?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "Audio track has no sample rate".to_string())?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Failed to create decoder: {e}"))?;

    let mut all_samples: Vec<f32> = Vec::new();

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Error reading packet: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                eprintln!("[audio] decode warning: {e}");
                continue;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("Decode error: {e}")),
        };

        let mono = decode_buffer_to_mono(&decoded);
        all_samples.extend_from_slice(&mono);
    }

    if all_samples.is_empty() {
        return Err("Decoded zero audio samples".to_string());
    }

    Ok((all_samples, sample_rate))
}

fn decode_buffer_to_mono(buf: &AudioBufferRef) -> Vec<f32> {
    match buf {
        AudioBufferRef::F32(b) => mix_to_mono(b.planes().planes(), |&v| v),
        AudioBufferRef::F64(b) => mix_to_mono(b.planes().planes(), |&v| v as f32),
        AudioBufferRef::S8(b) => mix_to_mono(b.planes().planes(), |&v| v as f32 / 128.0),
        AudioBufferRef::S16(b) => mix_to_mono(b.planes().planes(), |&v| v as f32 / 32768.0),
        AudioBufferRef::S24(b) => {
            mix_to_mono(b.planes().planes(), |v| v.inner() as f32 / 8388608.0)
        }
        AudioBufferRef::S32(b) => mix_to_mono(b.planes().planes(), |&v| v as f32 / 2147483648.0),
        AudioBufferRef::U8(b) => mix_to_mono(b.planes().planes(), |&v| (v as f32 - 128.0) / 128.0),
        AudioBufferRef::U16(b) => {
            mix_to_mono(b.planes().planes(), |&v| (v as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U24(b) => mix_to_mono(b.planes().planes(), |v| {
            (v.inner() as f32 - 8388608.0) / 8388608.0
        }),
        AudioBufferRef::U32(b) => mix_to_mono(b.planes().planes(), |&v| {
            (v as f64 - 2147483648.0) as f32 / 2147483648.0
        }),
    }
}

fn mix_to_mono<T, F>(planes: &[&[T]], convert: F) -> Vec<f32>
where
    F: Fn(&T) -> f32,
{
    if planes.is_empty() {
        return Vec::new();
    }
    let num_channels = planes.len();
    let num_frames = planes.iter().map(|ch| ch.len()).min().unwrap_or(0);

    if num_channels == 1 {
        return planes[0].iter().take(num_frames).map(&convert).collect();
    }

    let scale = 1.0 / num_channels as f32;
    (0..num_frames)
        .map(|i| {
            let sum: f32 = planes.iter().map(|ch| convert(&ch[i])).sum();
            sum * scale
        })
        .collect()
}

/// Convert stratum-dsp's circle-of-fifths notation to standard Camelot.
///
/// stratum-dsp uses its own numbering (A=major, B=minor, C=1).
/// Standard Camelot (Rekordbox/Mixed In Key): A=minor, B=major, C=8.
///
/// Mapping: flip A↔B, number = (stratum + 6) % 12 + 1.
fn to_camelot(stratum_notation: &str) -> String {
    let (num_str, letter) = if stratum_notation.ends_with('A') || stratum_notation.ends_with('B') {
        let (n, l) = stratum_notation.split_at(stratum_notation.len() - 1);
        (n, l)
    } else {
        return stratum_notation.to_string();
    };

    let stratum_num: u32 = match num_str.parse() {
        Ok(n) if (1..=12).contains(&n) => n,
        _ => return stratum_notation.to_string(),
    };

    let camelot_num = (stratum_num + 6) % 12 + 1;
    let camelot_letter = if letter == "A" { "B" } else { "A" };
    format!("{camelot_num}{camelot_letter}")
}

pub fn analyze(samples: &[f32], sample_rate: u32) -> Result<StratumResult, String> {
    let config = stratum_dsp::AnalysisConfig::default();

    let start = Instant::now();
    let result = stratum_dsp::analyze_audio(samples, sample_rate, config)
        .map_err(|e| format!("Analysis error: {e}"))?;
    let processing_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    let confidence = stratum_dsp::compute_confidence(&result);

    let duration_seconds = result.metadata.duration_seconds as f64;

    Ok(StratumResult {
        bpm: result.bpm as f64,
        bpm_confidence: confidence.bpm_confidence as f64,
        key: result.key.name(),
        key_camelot: to_camelot(&result.key.numerical()),
        key_confidence: confidence.key_confidence as f64,
        key_clarity: result.key_clarity as f64,
        grid_stability: confidence.grid_stability as f64,
        duration_seconds,
        processing_time_ms,
        analyzer_version: result.metadata.algorithm_version.clone(),
        flags: result
            .metadata
            .flags
            .iter()
            .map(|f| format!("{f:?}"))
            .collect(),
        warnings: result.metadata.confidence_warnings.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

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
            duration_seconds: 300.5,
            processing_time_ms: 1234.5,
            analyzer_version: "stratum-dsp-1.0.0".to_string(),
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
    fn to_camelot_converts_all_major_keys() {
        // stratum-dsp A (major) → standard Camelot B (major)
        // number = (stratum + 6) % 12 + 1
        assert_eq!(to_camelot("1A"), "8B"); // C
        assert_eq!(to_camelot("2A"), "9B"); // G
        assert_eq!(to_camelot("3A"), "10B"); // D
        assert_eq!(to_camelot("4A"), "11B"); // A
        assert_eq!(to_camelot("5A"), "12B"); // E
        assert_eq!(to_camelot("6A"), "1B"); // B
        assert_eq!(to_camelot("7A"), "2B"); // F#
        assert_eq!(to_camelot("8A"), "3B"); // C#
        assert_eq!(to_camelot("9A"), "4B"); // G#
        assert_eq!(to_camelot("10A"), "5B"); // D#
        assert_eq!(to_camelot("11A"), "6B"); // A#
        assert_eq!(to_camelot("12A"), "7B"); // F
    }

    #[test]
    fn to_camelot_converts_all_minor_keys() {
        // stratum-dsp B (minor) → standard Camelot A (minor)
        assert_eq!(to_camelot("1B"), "8A"); // Am
        assert_eq!(to_camelot("2B"), "9A"); // Em
        assert_eq!(to_camelot("3B"), "10A"); // Bm
        assert_eq!(to_camelot("4B"), "11A"); // F#m
        assert_eq!(to_camelot("5B"), "12A"); // C#m
        assert_eq!(to_camelot("6B"), "1A"); // G#m
        assert_eq!(to_camelot("7B"), "2A"); // D#m
        assert_eq!(to_camelot("8B"), "3A"); // A#m
        assert_eq!(to_camelot("9B"), "4A"); // Fm
        assert_eq!(to_camelot("10B"), "5A"); // Cm
        assert_eq!(to_camelot("11B"), "6A"); // Gm
        assert_eq!(to_camelot("12B"), "7A"); // Dm
    }

    #[test]
    fn to_camelot_passes_through_invalid_input() {
        assert_eq!(to_camelot(""), "");
        assert_eq!(to_camelot("X"), "X");
        assert_eq!(to_camelot("0A"), "0A");
        assert_eq!(to_camelot("13A"), "13A");
    }

    #[test]
    fn mix_to_mono_truncates_to_shortest_channel() {
        let left = [0.25_f32, 0.50, 0.75];
        let right = [0.75_f32, 0.25];
        let planes: &[&[f32]] = &[&left, &right];

        let mono = mix_to_mono(planes, |&v| v);

        assert_eq!(mono.len(), 2, "should use the shortest channel length");
        assert!((mono[0] - 0.50).abs() < f32::EPSILON);
        assert!((mono[1] - 0.375).abs() < f32::EPSILON);
    }

    #[test]
    fn mix_to_mono_single_channel_uses_all_frames() {
        let mono_src = [0.1_f32, 0.2, 0.3];
        let planes: &[&[f32]] = &[&mono_src];

        let mono = mix_to_mono(planes, |&v| v);

        assert_eq!(mono, mono_src);
    }

    #[test]
    fn mix_to_mono_returns_empty_when_any_channel_is_empty() {
        let left: [f32; 0] = [];
        let right = [0.25_f32, 0.50, 0.75];
        let planes: &[&[f32]] = &[&left, &right];

        let mono = mix_to_mono(planes, |&v| v);

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
    fn parse_essentia_stdout_rejects_empty_output() {
        let err =
            parse_essentia_stdout(b"   \n").expect_err("empty output should produce a parse error");
        assert!(
            err.contains("empty"),
            "error should mention empty stdout, got: {err}"
        );
    }

    #[tokio::test]
    async fn run_essentia_reports_subprocess_start_failure() {
        let err = run_essentia("/definitely/missing/python", "/tmp/does-not-matter.wav")
            .await
            .expect_err("missing python binary should fail");
        assert!(
            err.contains("Failed to start Essentia subprocess"),
            "expected startup failure context, got: {err}"
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
            essentia_pkg.join("__init__.py"),
            "__version__ = '2.1-test'\n",
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
        if isinstance(audio, tuple) and len(audio) > 0 and audio[0] == "stereo":
            return ([1.0, 2.0], [3.0], -14.5, 4.2)
        raise TypeError("Cannot convert data from type VECTOR_REAL to VECTOR_STEREOSAMPLE")

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

class Intensity:
    def __call__(self, spec):
        return 0.65
"#,
        )
        .expect("fake essentia.standard should be written");

        std::fs::write(
            tmp.path().join("numpy.py"),
            r#"
class _FakeArray:
    def __init__(self, value):
        self._flat = []
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

def asarray(value):
    return _FakeArray(value)

def column_stack(cols):
    return ("stereo", cols)
"#,
        )
        .expect("fake numpy module should be written");

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

        assert_eq!(result.analyzer_version, "2.1-test");
        assert!((result.danceability.unwrap() - 2.46).abs() < 1e-6);
        assert!((result.loudness_integrated.unwrap() - (-14.5)).abs() < 1e-6);
        assert!((result.loudness_range.unwrap() - 4.2).abs() < 1e-6);
        assert!((result.onset_rate.unwrap() - 5.6).abs() < 1e-6);
        assert!(
            result.rhythm_regularity.unwrap() > 0.0,
            "rhythm_regularity should be computed from beat loudness ratios"
        );

        // Frame-based features
        let mfcc = result.mfcc_mean.as_ref().expect("mfcc_mean should be present");
        assert_eq!(mfcc.len(), 13, "mfcc_mean should have 13 coefficients");

        let contrast = result.spectral_contrast_mean.as_ref().expect("spectral_contrast_mean should be present");
        assert_eq!(
            contrast.len(),
            6,
            "spectral_contrast_mean should have 6 bands"
        );

        let dissonance = result.dissonance_mean.expect("dissonance_mean should be present");
        assert!(
            dissonance > 0.0 && dissonance < 1.0,
            "dissonance should be in (0, 1), got {dissonance}"
        );

        let intensity = result.intensity_mean.expect("intensity_mean should be present");
        assert!(intensity > 0.0, "intensity_mean should be positive");
        assert!(
            result.intensity_var.is_some(),
            "intensity_var should be present"
        );
    }

    // ==================== Integration tests (real audio files) ====================
    // Run with: cargo test -- --ignored

    #[test]
    #[ignore]
    fn test_real_audio_analysis() {
        // Find a real track from the Rekordbox DB
        let conn = crate::db::open_real_db().expect("backup tarball not found");
        let params = crate::db::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: Some(120.0),
            bpm_max: Some(140.0),
            key: None,
            playlist: None,
            has_genre: Some(true),
            label: None,
            path: None,
            added_after: None,
            added_before: None,
            exclude_samples: true,
            limit: Some(5),
            offset: None,
        };
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "no tracks found for analysis test");

        // Find a track whose file actually exists
        let track = tracks
            .iter()
            .find(|t| {
                let path = &t.file_path;
                std::fs::metadata(path).is_ok()
                    || percent_encoding::percent_decode_str(path)
                        .decode_utf8()
                        .ok()
                        .map(|d| std::fs::metadata(d.as_ref()).is_ok())
                        .unwrap_or(false)
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

        assert!(
            !samples.is_empty(),
            "decoded zero samples from {}",
            file_path
        );
        assert!(sample_rate > 0, "invalid sample rate from {}", file_path);
        eprintln!(
            "[integration] Decoded: {} samples at {} Hz ({:.1}s)",
            samples.len(),
            sample_rate,
            samples.len() as f64 / sample_rate as f64
        );

        let result =
            analyze(&samples, sample_rate).unwrap_or_else(|e| panic!("analysis failed: {e}"));

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
        // Analyze a real track, verify it can be cached and retrieved
        let conn = crate::db::open_real_db().expect("backup tarball not found");
        let params = crate::db::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: Some(120.0),
            bpm_max: Some(140.0),
            key: None,
            playlist: None,
            has_genre: Some(true),
            label: None,
            path: None,
            added_after: None,
            added_before: None,
            exclude_samples: true,
            limit: Some(5),
            offset: None,
        };
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
        let track = tracks
            .iter()
            .find(|t| {
                let path = &t.file_path;
                std::fs::metadata(path).is_ok()
                    || percent_encoding::percent_decode_str(path)
                        .decode_utf8()
                        .ok()
                        .map(|d| std::fs::metadata(d.as_ref()).is_ok())
                        .unwrap_or(false)
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

        // Decode + analyze
        let (samples, sample_rate) = decode_to_samples(&file_path).unwrap();
        let result = analyze(&samples, sample_rate).unwrap();
        let features_json = serde_json::to_string(&result).unwrap();

        // Write to a temp store
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("test-cache.sqlite3");
        let store_conn = crate::store::open(store_path.to_str().unwrap()).unwrap();

        let metadata = std::fs::metadata(&file_path).unwrap();
        let file_size = metadata.len() as i64;
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        crate::store::set_audio_analysis(
            &store_conn,
            &file_path,
            "stratum-dsp",
            file_size,
            file_mtime,
            &result.analyzer_version,
            &features_json,
        )
        .unwrap();

        // Read back
        let cached = crate::store::get_audio_analysis(&store_conn, &file_path, "stratum-dsp")
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
}
