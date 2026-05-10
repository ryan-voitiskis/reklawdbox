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

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Decode(String),
    #[error("{0}")]
    Subprocess(String),
    #[error("{0}")]
    Parse(String),
    #[error("{0}")]
    Analysis(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StratumResult {
    pub bpm: f64,
    pub bpm_confidence: f64,
    pub key: String,
    pub key_camelot: String,
    pub key_confidence: f64,
    pub key_clarity: f64,
    pub grid_stability: f64,
    /// "rekordbox" when the caller supplied an external grid (e.g. from
    /// ANLZ PQTZ), "hmm" when the analyzer's own beat tracker was used.
    /// Empty string on cached results from before this field existed.
    pub grid_source: String,
    pub duration_seconds: f64,
    pub processing_time_ms: f64,
    pub analyzer_version: String,
    pub mod_centroid: Option<f64>,
    pub harmonic_proportion: Option<f64>,
    pub decay_mid_tau: Option<f64>,
    pub decay_mid_r2: Option<f64>,
    pub decay_high_tau: Option<f64>,
    pub decay_high_r2: Option<f64>,
    /// Number of kick-disjoint stab-band onsets surviving the ±80 ms kick
    /// mask. Populated when the beat grid has at least two beats.
    pub dub_stab_onset_count: Option<u32>,
    /// 32-bin global beat-relative offset histogram (Stage 2).
    /// Bin 0 is on-beat; bin 16 is the offbeat-eighth. Per-bar histograms
    /// are not surfaced here — callers needing them should run the
    /// analyzer with stratum-dsp directly.
    pub dub_stab_histogram: Option<Vec<f64>>,
    /// Stage 3 — best-matching dub-techno chord-stab template name. One of
    /// "offbeat_eighth", "all_16th_offbeats", "anticipation", "on_beat".
    pub dub_stab_template: Option<String>,
    /// Cosine similarity in `[0, 1]` of the histogram against
    /// `dub_stab_template`. Treat values below ~0.7 as low-confidence.
    pub dub_stab_template_score: Option<f64>,
    pub flags: Vec<String>,
    pub warnings: Vec<String>,
}

pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["flac", "wav", "mp3", "m4a", "aac", "aiff"];

/// DB cache key for stratum-dsp.
pub const ANALYZER_STRATUM: &str = "stratum-dsp";
/// DB cache key for Essentia.
pub const ANALYZER_ESSENTIA: &str = "essentia";

/// Expected analysis schema versions. Bump these when adding/changing output
/// fields so that stale cache entries are evicted automatically.
pub const STRATUM_SCHEMA_VERSION: &str = "7";
pub const ESSENTIA_SCHEMA_VERSION: &str = "2";

const ESSENTIA_TIMEOUT_SECS: u64 = 300;

const ESSENTIA_SCRIPT: &str = include_str!("essentia_analysis.py");

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
    pub mfcc_std: Option<Vec<f64>>,
    pub spectral_centroid_cv: Option<f64>,
    pub spectral_flux_mean: Option<f64>,
    pub spectral_flux_iqr: Option<f64>,
    pub spectral_contrast_mean: Option<Vec<f64>>,
}

fn parse_essentia_stdout(stdout: &[u8]) -> Result<EssentiaOutput, AudioError> {
    let text = std::str::from_utf8(stdout)
        .map_err(|e| AudioError::Parse(format!("Essentia stdout was not valid UTF-8: {e}")))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(AudioError::Parse("Essentia stdout was empty".to_string()));
    }
    serde_json::from_str(trimmed)
        .map_err(|e| AudioError::Parse(format!("Failed to parse Essentia JSON output: {e}")))
}

pub async fn run_essentia(
    python_path: &str,
    audio_path: &str,
) -> Result<EssentiaOutput, AudioError> {
    let mut command = Command::new(python_path);
    command.args(["-c", ESSENTIA_SCRIPT, audio_path]);
    command.kill_on_drop(true);

    let output = timeout(Duration::from_secs(ESSENTIA_TIMEOUT_SECS), command.output())
        .await
        .map_err(|_| {
            AudioError::Subprocess(format!(
                "Essentia analysis timed out after {ESSENTIA_TIMEOUT_SECS}s for '{audio_path}'"
            ))
        })?
        .map_err(|e| AudioError::Subprocess(format!("Failed to start Essentia subprocess: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stderr = if stderr.is_empty() {
            "(no stderr output)".to_string()
        } else {
            stderr
        };
        return Err(AudioError::Subprocess(format!(
            "Essentia subprocess failed for '{audio_path}': {stderr}"
        )));
    }

    parse_essentia_stdout(&output.stdout)
}

/// Resolve a Rekordbox file path to an actual filesystem path.
/// Tries the raw path first; if that fails, tries percent-decoding.
pub(crate) fn resolve_audio_path(raw_path: &str) -> Result<String, AudioError> {
    if std::fs::metadata(raw_path).is_ok() {
        return Ok(raw_path.to_string());
    }

    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|e| AudioError::Parse(format!("Invalid UTF-8 in file path: {e}")))?
        .to_string();

    if std::fs::metadata(&decoded).is_ok() {
        return Ok(decoded);
    }

    Err(AudioError::Io(format!(
        "File not found (tried raw and decoded): {raw_path}"
    )))
}

pub fn decode_to_samples(path: &str) -> Result<(Vec<f32>, u32), AudioError> {
    let file = std::fs::File::open(path)
        .map_err(|e| AudioError::Io(format!("Failed to open audio file '{path}': {e}")))?;
    let media_source_stream = MediaSourceStream::new(Box::new(file), Default::default());

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
            media_source_stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::Decode(format!("Failed to probe audio format: {e}")))?;

    let mut format_reader = probed.format;

    let track = format_reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::Decode("No audio track found in file".to_string()))?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| AudioError::Decode("Audio track has no sample rate".to_string()))?;
    let n_frames_hint = track.codec_params.n_frames;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::Decode(format!("Failed to create decoder: {e}")))?;

    let mut all_samples: Vec<f32> = Vec::with_capacity(n_frames_hint.unwrap_or(0) as usize);
    let mut decode_warning_count: u64 = 0;

    loop {
        let packet = match format_reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(AudioError::Decode(format!("Error reading packet: {e}"))),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => {
                decode_warning_count += 1;
                continue;
            }
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                match decoder.decode(&packet) {
                    Ok(d) => d,
                    Err(symphonia::core::errors::Error::DecodeError(_)) => {
                        decode_warning_count += 1;
                        continue;
                    }
                    Err(e) => {
                        return Err(AudioError::Decode(format!("Decode error after reset: {e}")));
                    }
                }
            }
            Err(e) => return Err(AudioError::Decode(format!("Decode error: {e}"))),
        };

        let mono = decode_buffer_to_mono(&decoded);
        all_samples.extend_from_slice(&mono);
    }

    if decode_warning_count > 0 {
        tracing::debug!("{path}: {decode_warning_count} decode warnings (frames skipped)");
    }

    if all_samples.is_empty() {
        return Err(AudioError::Decode("Decoded zero audio samples".to_string()));
    }

    Ok((all_samples, sample_rate))
}

fn decode_buffer_to_mono(buf: &AudioBufferRef) -> Vec<f32> {
    match buf {
        AudioBufferRef::F32(b) => downmix_to_mono(b.planes().planes(), |&v| v),
        AudioBufferRef::F64(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32),
        AudioBufferRef::S8(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32 / 128.0),
        AudioBufferRef::S16(b) => downmix_to_mono(b.planes().planes(), |&v| v as f32 / 32768.0),
        AudioBufferRef::S24(b) => {
            downmix_to_mono(b.planes().planes(), |v| v.inner() as f32 / 8388608.0)
        }
        AudioBufferRef::S32(b) => {
            downmix_to_mono(b.planes().planes(), |&v| v as f32 / 2147483648.0)
        }
        AudioBufferRef::U8(b) => {
            downmix_to_mono(b.planes().planes(), |&v| (v as f32 - 128.0) / 128.0)
        }
        AudioBufferRef::U16(b) => {
            downmix_to_mono(b.planes().planes(), |&v| (v as f32 - 32768.0) / 32768.0)
        }
        AudioBufferRef::U24(b) => downmix_to_mono(b.planes().planes(), |v| {
            (v.inner() as f32 - 8388608.0) / 8388608.0
        }),
        AudioBufferRef::U32(b) => downmix_to_mono(b.planes().planes(), |&v| {
            (v as f64 - 2147483648.0) as f32 / 2147483648.0
        }),
    }
}

fn downmix_to_mono<T, F>(planes: &[&[T]], convert: F) -> Vec<f32>
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

    let channel_weight = 1.0 / num_channels as f32;
    (0..num_frames)
        .map(|i| {
            let sum: f32 = planes.iter().map(|ch| convert(&ch[i])).sum();
            sum * channel_weight
        })
        .collect()
}

/// Convert stratum-dsp's circle-of-fifths notation to standard Camelot.
///
/// stratum-dsp uses its own numbering (A=major, B=minor, C=1).
/// Standard Camelot (Rekordbox/Mixed In Key): A=minor, B=major, C=8.
///
/// Mapping: flip A↔B, number = (stratum + 6) % 12 + 1.
fn stratum_notation_to_camelot(stratum_notation: &str) -> String {
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

/// Look up a track's Rekordbox-tagged beat grid by its file path.
///
/// Opens the master.db read-only, finds the track's `AnalysisDataPath`,
/// resolves it under the USBANLZ root, and parses the PQTZ tag. Returns
/// `None` if anything in the chain fails (no DB, track not found, ANLZ
/// missing, parse error). Callers should treat `None` as "no Rekordbox
/// grid available — fall back to HMM tracking", not as an error.
///
/// Each call opens a fresh DB connection. For batch use this is wasteful
/// (~ms per track) — callers iterating many tracks may want a variant
/// that takes a borrowed connection instead.
pub fn load_rekordbox_grid_for_path(file_path: &str) -> Option<stratum_dsp::BeatGrid> {
    let db_path = crate::db::resolve_db_path()?;
    let conn = crate::db::open(&db_path).ok()?;
    let analysis_data_path: String = conn
        .query_row(
            "SELECT AnalysisDataPath FROM djmdContent WHERE FolderPath = ?1 LIMIT 1",
            [file_path],
            |row| row.get(0),
        )
        .ok()?;
    if analysis_data_path.is_empty() {
        return None;
    }
    let abs = crate::anlz::resolve_anlz_path(&analysis_data_path)?;
    crate::anlz::read_beat_grid(std::path::Path::new(&abs)).ok()
}

pub fn analyze_with_stratum(
    samples: &[f32],
    sample_rate: u32,
    external_beat_grid: Option<stratum_dsp::BeatGrid>,
) -> Result<StratumResult, AudioError> {
    let grid_source = if external_beat_grid.is_some() {
        "rekordbox"
    } else {
        "hmm"
    };
    let config = stratum_dsp::AnalysisConfig {
        external_beat_grid,
        ..stratum_dsp::AnalysisConfig::default()
    };

    let start = Instant::now();
    let result = stratum_dsp::analyze_audio(samples, sample_rate, config)
        .map_err(|e| AudioError::Analysis(format!("Analysis error: {e}")))?;
    let processing_time_ms = start.elapsed().as_secs_f64() * 1000.0;

    let confidence = stratum_dsp::compute_confidence(&result);

    let duration_seconds = result.metadata.duration_seconds as f64;

    Ok(StratumResult {
        bpm: result.bpm as f64,
        bpm_confidence: confidence.bpm_confidence as f64,
        key: result.key.name(),
        key_camelot: stratum_notation_to_camelot(&result.key.numerical()),
        key_confidence: confidence.key_confidence as f64,
        key_clarity: result.key_clarity as f64,
        grid_stability: confidence.grid_stability as f64,
        grid_source: grid_source.to_string(),
        duration_seconds,
        processing_time_ms,
        analyzer_version: result.metadata.algorithm_version.clone(),
        mod_centroid: result.mod_centroid.map(|v| v as f64),
        harmonic_proportion: result.harmonic_proportion.map(|v| v as f64),
        decay_mid_tau: result
            .decay
            .as_ref()
            .and_then(|d| d.mid.as_ref())
            .map(|b| b.tau_median as f64),
        decay_mid_r2: result
            .decay
            .as_ref()
            .and_then(|d| d.mid.as_ref())
            .map(|b| b.fit_r2_median as f64),
        decay_high_tau: result
            .decay
            .as_ref()
            .and_then(|d| d.high.as_ref())
            .map(|b| b.tau_median as f64),
        decay_high_r2: result
            .decay
            .as_ref()
            .and_then(|d| d.high.as_ref())
            .map(|b| b.fit_r2_median as f64),
        dub_stab_onset_count: result.dub_stab.as_ref().map(|d| d.stab_onset_count as u32),
        dub_stab_histogram: result
            .dub_stab
            .as_ref()
            .map(|d| d.histogram.iter().map(|&v| v as f64).collect()),
        // Surface the template name only when the cosine similarity clears
        // MIN_TEMPLATE_CONFIDENCE — below that, the histogram has peaks in
        // positions none of the canonical templates cover well, and the
        // "best match" is just the least-bad fit. The score is always
        // surfaced (when a match exists at all) so callers can see what was
        // rejected.
        dub_stab_template: result
            .dub_stab
            .as_ref()
            .and_then(|d| d.template_match.as_ref())
            .filter(|t| t.score >= stratum_dsp::features::dub_stab::MIN_TEMPLATE_CONFIDENCE)
            .map(|t| t.name.clone()),
        dub_stab_template_score: result
            .dub_stab
            .as_ref()
            .and_then(|d| d.template_match.as_ref())
            .map(|t| t.score as f64),
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
            dub_stab_histogram: Some(vec![0.0; 32]),
            dub_stab_template: Some("offbeat_eighth".to_string()),
            dub_stab_template_score: Some(0.92),
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

    // ==================== Integration tests (real audio files) ====================
    // Run with: cargo test -- --ignored

    #[test]
    #[ignore]
    fn test_real_audio_analysis() {
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
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
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
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
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
        let store_conn = crate::store::open(store_path.to_str().unwrap()).unwrap();

        let metadata = std::fs::metadata(&file_path).unwrap();
        let file_size = metadata.len() as i64;
        let file_mtime = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs() as i64);

        crate::store::set_audio_analysis(
            &store_conn,
            &file_path,
            "stratum-dsp",
            file_size,
            file_mtime,
            STRATUM_SCHEMA_VERSION,
            &features_json,
        )
        .unwrap();

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
