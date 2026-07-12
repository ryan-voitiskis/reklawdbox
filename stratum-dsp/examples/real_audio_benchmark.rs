//! Machine-readable runner for the versioned private-audio benchmark.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use stratum_dsp::{analyze_audio, AnalysisConfig, BeatGrid};
use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

#[derive(Deserialize, Serialize)]
struct JsonGrid {
    beats: Vec<f32>,
    bars: Vec<f32>,
}

#[derive(Serialize)]
struct BenchmarkResult {
    schema_version: u32,
    track_id: u64,
    relative_path: String,
    audio_sha256: String,
    grid_sha256: String,
    grid_source: &'static str,
    analysis: serde_json::Value,
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn downmix(buf: &AudioBufferRef<'_>, output: &mut Vec<f32>) {
    fn mix<T: Copy>(planes: &[&[T]], convert: impl Fn(T) -> f32, output: &mut Vec<f32>) {
        for index in 0..planes[0].len() {
            let sum: f32 = planes.iter().map(|plane| convert(plane[index])).sum();
            output.push(sum / planes.len() as f32);
        }
    }
    match buf {
        AudioBufferRef::F32(b) => mix(b.planes().planes(), |v| v, output),
        AudioBufferRef::F64(b) => mix(b.planes().planes(), |v| v as f32, output),
        AudioBufferRef::S8(b) => mix(b.planes().planes(), |v| v as f32 / 128.0, output),
        AudioBufferRef::S16(b) => mix(b.planes().planes(), |v| v as f32 / 32768.0, output),
        AudioBufferRef::S24(b) => mix(
            b.planes().planes(),
            |v| v.inner() as f32 / 8_388_608.0,
            output,
        ),
        AudioBufferRef::S32(b) => mix(b.planes().planes(), |v| v as f32 / 2_147_483_648.0, output),
        AudioBufferRef::U8(b) => mix(b.planes().planes(), |v| (v as f32 - 128.0) / 128.0, output),
        AudioBufferRef::U16(b) => mix(
            b.planes().planes(),
            |v| (v as f32 - 32_768.0) / 32_768.0,
            output,
        ),
        AudioBufferRef::U24(b) => mix(
            b.planes().planes(),
            |v| (v.inner() as f32 - 8_388_608.0) / 8_388_608.0,
            output,
        ),
        AudioBufferRef::U32(b) => mix(
            b.planes().planes(),
            |v| (v as f32 - 2_147_483_648.0) / 2_147_483_648.0,
            output,
        ),
    }
}

fn decode(path: &Path, max_seconds: f32) -> Result<(Vec<f32>, u32), String> {
    let file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let source = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        hint.with_extension(extension);
    }
    let probed = get_probe()
        .format(
            &hint,
            source,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|error| format!("probe {}: {error}", path.display()))?;
    let mut reader = probed.format;
    let track = reader
        .default_track()
        .ok_or_else(|| format!("no default audio track in {}", path.display()))?;
    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or("missing sample rate")?;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|error| format!("decoder: {error}"))?;
    let limit = (max_seconds * sample_rate as f32) as usize;
    let mut samples = Vec::with_capacity(limit);
    while samples.len() < limit {
        let packet = match reader.next_packet() {
            Ok(packet) => packet,
            Err(symphonia::core::errors::Error::IoError(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break
            }
            Err(error) => return Err(format!("read packet: {error}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(decoded) => downmix(&decoded, &mut samples),
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(error) => return Err(format!("decode packet: {error}")),
        }
    }
    samples.truncate(limit);
    Ok((samples, sample_rate))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 6 {
        return Err(
            "usage: real_audio_benchmark TRACK_ID AUDIO_PATH RELATIVE_PATH GRID_JSON MAX_SECONDS"
                .into(),
        );
    }
    let track_id = args[1].parse::<u64>().map_err(|error| error.to_string())?;
    let audio_path = Path::new(&args[2]);
    let relative_path = args[3].clone();
    let max_seconds = args[5].parse::<f32>().map_err(|error| error.to_string())?;
    let grid_bytes = std::fs::read(&args[4]).map_err(|error| format!("read grid JSON: {error}"))?;
    let grids: HashMap<String, JsonGrid> =
        serde_json::from_slice(&grid_bytes).map_err(|error| format!("parse grid JSON: {error}"))?;
    let absolute_path = audio_path.to_string_lossy();
    let grid = grids
        .get(absolute_path.as_ref())
        .ok_or_else(|| format!("grid JSON has no entry for {}", audio_path.display()))?;
    let grid_sha256 = format!("{:x}", Sha256::digest(serde_json::to_vec(grid).unwrap()));
    let (samples, sample_rate) = decode(audio_path, max_seconds)?;
    let duration = samples.len() as f32 / sample_rate as f32;
    let beats = grid
        .beats
        .iter()
        .copied()
        .filter(|time| *time < duration)
        .collect();
    let bars: Vec<f32> = grid
        .bars
        .iter()
        .copied()
        .filter(|time| *time < duration)
        .collect();
    let beat_grid = BeatGrid {
        downbeats: bars.clone(),
        beats,
        bars,
    };
    beat_grid.validate().map_err(|error| error.to_string())?;
    let analysis = analyze_audio(
        &samples,
        sample_rate,
        AnalysisConfig {
            external_beat_grid: Some(beat_grid),
            ..AnalysisConfig::default()
        },
    )
    .map_err(|error| error.to_string())?;
    let dub_stab_peak_bin = analysis.dub_stab.as_ref().and_then(|value| {
        value
            .histogram
            .iter()
            .enumerate()
            .max_by(|left, right| {
                left.1
                    .partial_cmp(right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(index, _)| index)
    });
    let analysis = serde_json::json!({
        "bpm": analysis.bpm,
        "bpm_confidence": analysis.bpm_confidence,
        "key": analysis.key,
        "key_confidence": analysis.key_confidence,
        "key_clarity": analysis.key_clarity,
        "grid_stability": analysis.grid_stability,
        "mod_centroid": analysis.mod_centroid,
        "harmonic_proportion": analysis.harmonic_proportion,
        "decay": analysis.decay,
        "dub_stab": analysis.dub_stab.as_ref().map(|value| serde_json::json!({
            "stab_onset_count": value.stab_onset_count,
            "stab_onset_rate": value.stab_onset_rate,
            "rate_basis": value.rate_basis,
            "histogram": value.histogram,
            "histogram_peak_bin": dub_stab_peak_bin,
            "template_match": value.template_match,
        })),
        "kick_pattern": analysis.kick_pattern.as_ref().map(|value| serde_json::json!({
            "pattern": value.pattern,
            "confidence": value.confidence,
            "kicks_per_bar": value.kicks_per_bar,
            "onset_count": value.onset_count,
            "rate_basis": value.rate_basis,
        })),
        "section_count": analysis.sections.as_ref().map(Vec::len),
        "algorithm_version": analysis.metadata.algorithm_version,
        "flags": analysis.metadata.flags,
        "confidence_warnings": analysis.metadata.confidence_warnings,
    });
    let output = BenchmarkResult {
        schema_version: 1,
        track_id,
        relative_path,
        audio_sha256: sha256_file(audio_path)?,
        grid_sha256,
        grid_source: "rekordbox_pqtz",
        analysis,
    };
    println!(
        "{}",
        serde_json::to_string(&output).map_err(|error| error.to_string())?
    );
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
