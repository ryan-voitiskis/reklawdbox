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

/// Result of stratum-dsp audio analysis, suitable for caching and display.
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

const ESSENTIA_TIMEOUT_SECS: u64 = 300;

const ESSENTIA_SCRIPT: &str = r#"
import json
import sys
import essentia
import essentia.standard as es

audio = es.MonoLoader(filename=sys.argv[1], sampleRate=44100)()
features = {}

features["danceability"] = float(es.Danceability()(audio)[0])

ebu = es.LoudnessEBUR128()(audio)
features["loudness_integrated"] = float(ebu[0])
features["loudness_range"] = float(ebu[2])

features["dynamic_complexity"] = float(es.DynamicComplexity()(audio)[0])
features["average_loudness"] = float(es.Loudness()(audio))

rhythm = es.RhythmExtractor2013(method="multifeature")(audio)
features["bpm_essentia"] = float(rhythm[0])
features["onset_rate"] = float(es.OnsetRate()(audio)[0])

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

features["analyzer_version"] = essentia.__version__

json.dump(features, sys.stdout)
"#;

/// Parse Essentia subprocess stdout into JSON.
fn parse_essentia_stdout(stdout: &[u8]) -> Result<serde_json::Value, String> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| format!("Essentia stdout was not valid UTF-8: {e}"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Essentia stdout was empty".to_string());
    }
    serde_json::from_str(trimmed).map_err(|e| format!("Failed to parse Essentia JSON output: {e}"))
}

/// Run Essentia feature extraction through a Python subprocess.
pub async fn run_essentia(
    python_path: &str,
    audio_path: &str,
) -> Result<serde_json::Value, String> {
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

/// Decode an audio file to mono f32 samples using symphonia.
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

/// Convert an AudioBufferRef to mono f32 samples by averaging channels.
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

/// Mix multiple channels to mono by averaging, using a conversion function.
fn mix_to_mono<T, F>(planes: &[&[T]], convert: F) -> Vec<f32>
where
    F: Fn(&T) -> f32,
{
    if planes.is_empty() {
        return Vec::new();
    }
    let num_channels = planes.len();
    let num_frames = planes[0].len();

    if num_channels == 1 {
        return planes[0].iter().map(&convert).collect();
    }

    let scale = 1.0 / num_channels as f32;
    (0..num_frames)
        .map(|i| {
            let sum: f32 = planes.iter().map(|ch| convert(&ch[i])).sum();
            sum * scale
        })
        .collect()
}

/// Run stratum-dsp analysis on decoded audio samples.
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
        key_camelot: result.key.numerical(),
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

    #[test]
    fn stratum_result_serialization_round_trip() {
        let result = StratumResult {
            bpm: 128.0,
            bpm_confidence: 0.95,
            key: "Am".to_string(),
            key_camelot: "1B".to_string(),
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
        assert_eq!(back.key_camelot, "1B");
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
    fn parse_essentia_stdout_trims_whitespace() {
        let parsed =
            parse_essentia_stdout(b"\n  {\"danceability\": 0.82, \"analyzer_version\": \"2.1\"}\n")
                .expect("valid JSON with whitespace should parse");
        assert_eq!(parsed["danceability"], 0.82);
        assert_eq!(parsed["analyzer_version"], "2.1");
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
            exclude_samples: true,
            limit: Some(5),
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
            exclude_samples: true,
            limit: Some(5),
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
