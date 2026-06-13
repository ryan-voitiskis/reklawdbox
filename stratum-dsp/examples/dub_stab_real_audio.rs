//! Stage 1 + Stage 2 dub-stab evaluation on a real track.
//!
//! Pipeline:
//! - Decode → mono samples (capped at `ANALYSIS_DURATION_S`)
//! - STFT (frame=2048, hop=512)
//! - Beat grid: prefer Rekordbox-tagged grid (loaded from JSON dumped by
//!   `dump_rekordbox_grids.py`); fall back to HMM Viterbi seeded with the
//!   supplied BPM hint if no grid is found.
//! - Stage 1: `detect_kick_disjoint_stab_onsets` (350–2000 Hz stabs, ±80 ms
//!   kick mask using 40–120 Hz kick onsets)
//! - Stage 2: `beat_relative_offset_histogram` with circular soft-binning
//!
//! Run:
//!   cargo run --release -p stratum-dsp --example dub_stab_real_audio -- \
//!       <path> <bpm> [grid_json]

use std::collections::HashMap;
use std::env;

use stratum_dsp::analysis::result::BeatGrid;
use stratum_dsp::features::beat_tracking::generate_beat_grid;
use stratum_dsp::features::chroma::extractor::compute_stft;
use stratum_dsp::features::dub_stab::{
    beat_relative_offset_histogram, detect_kick_disjoint_stab_onsets, DubStabConfig, HISTOGRAM_BINS,
};
use stratum_dsp::features::onset::spectral_flux::detect_spectral_flux_onsets;

use serde::Deserialize;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

const FRAME: usize = 2048;
const HOP: usize = 512;
const ANALYSIS_DURATION_S: f32 = 90.0;

fn decode_mono(path: &str) -> Result<(Vec<f32>, u32), String> {
    let file = std::fs::File::open(path).map_err(|e| format!("Failed to open '{path}': {e}"))?;
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
        .map_err(|e| format!("Probe failed: {e}"))?;
    let mut reader = probed.format;

    let track = reader
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| "No audio track".to_string())?;
    let track_id = track.id;
    let sr = track
        .codec_params
        .sample_rate
        .ok_or_else(|| "No sample rate".to_string())?;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("Decoder init: {e}"))?;

    let mut samples: Vec<f32> = Vec::new();

    loop {
        let packet = match reader.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("Reader: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(symphonia::core::errors::Error::ResetRequired) => {
                decoder.reset();
                continue;
            }
            Err(e) => return Err(format!("Decode: {e}")),
        };
        downmix_to_mono(&decoded, &mut samples);

        if samples.len() as f32 / sr as f32 >= ANALYSIS_DURATION_S {
            break;
        }
    }

    samples.truncate((ANALYSIS_DURATION_S * sr as f32) as usize);
    Ok((samples, sr))
}

fn downmix_to_mono(buf: &AudioBufferRef, out: &mut Vec<f32>) {
    fn mix<T: Copy, F: Fn(T) -> f32>(planes: &[&[T]], f: F, out: &mut Vec<f32>) {
        let n = planes[0].len();
        let inv_ch = 1.0 / planes.len() as f32;
        for i in 0..n {
            let mut s = 0.0;
            for plane in planes {
                s += f(plane[i]);
            }
            out.push(s * inv_ch);
        }
    }
    match buf {
        AudioBufferRef::F32(b) => mix(b.planes().planes(), |v| v, out),
        AudioBufferRef::F64(b) => mix(b.planes().planes(), |v| v as f32, out),
        AudioBufferRef::S8(b) => mix(b.planes().planes(), |v| v as f32 / 128.0, out),
        AudioBufferRef::S16(b) => mix(b.planes().planes(), |v| v as f32 / 32768.0, out),
        AudioBufferRef::S24(b) => mix(b.planes().planes(), |v| v.inner() as f32 / 8388608.0, out),
        AudioBufferRef::S32(b) => mix(b.planes().planes(), |v| v as f32 / 2147483648.0, out),
        AudioBufferRef::U8(b) => mix(b.planes().planes(), |v| (v as f32 - 128.0) / 128.0, out),
        AudioBufferRef::U16(b) => mix(b.planes().planes(), |v| (v as f32 - 32768.0) / 32768.0, out),
        AudioBufferRef::U24(b) => mix(
            b.planes().planes(),
            |v| (v.inner() as f32 - 8388608.0) / 8388608.0,
            out,
        ),
        AudioBufferRef::U32(b) => mix(
            b.planes().planes(),
            |v| (v as f32 - 2147483648.0) / 2147483648.0,
            out,
        ),
    }
}

fn frame_to_time(f: usize, sr: u32) -> f32 {
    f as f32 * HOP as f32 / sr as f32
}

#[derive(Deserialize)]
struct JsonGrid {
    beats: Vec<f32>,
    bars: Vec<f32>,
}

fn load_grid_json(path: &str) -> Result<HashMap<String, JsonGrid>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("parse {path}: {e}"))
}

/// Drop non-finite entries and collapse runs that aren't strictly ascending.
/// `generate_beat_grid` occasionally emits duplicate timestamps (~1 ms apart);
/// dub_stab requires a strictly-ascending grid by contract.
fn sanitize_grid(xs: &mut Vec<f32>) {
    xs.retain(|x| x.is_finite());
    let mut last = f32::NEG_INFINITY;
    xs.retain(|&x| {
        if x > last {
            last = x;
            true
        } else {
            false
        }
    });
}

fn print_histogram(label: &str, hist: &[f32; HISTOGRAM_BINS]) {
    let total: f32 = hist.iter().sum();
    if total <= 0.0 {
        println!("\n{label}: (empty)");
        return;
    }
    let max = hist.iter().cloned().fold(0.0_f32, f32::max);
    println!("\n{label}  (Σ={total:.1}, peak={max:.2})");
    for (bin, &w) in hist.iter().enumerate() {
        let bar_len = (w / max * 40.0) as usize;
        let bar: String = std::iter::repeat_n('#', bar_len).collect();
        let frac = bin as f32 / HISTOGRAM_BINS as f32;
        let beat_label = match bin {
            0 => " on-beat",
            8 => " 1/4-off",
            16 => " 1/2-off (off-beat)",
            24 => " 3/4-off",
            _ => "",
        };
        println!(
            "  bin {:>2} ({:>4.2}) {:>6.2} {:<40}{}",
            bin, frac, w, bar, beat_label
        );
    }
}

fn analyse(path: &str, bpm_hint: f32, json_grid: Option<&JsonGrid>) -> Result<(), String> {
    let grid_source = if json_grid.is_some() {
        "rekordbox"
    } else {
        "hmm"
    };
    println!("=== {path} (BPM hint {bpm_hint:.1}, grid: {grid_source}) ===");
    let (samples, sr) = decode_mono(path)?;
    let dur_s = samples.len() as f32 / sr as f32;
    println!(
        "Decoded {} samples ({:.1} s) at {} Hz",
        samples.len(),
        dur_s,
        sr
    );

    let spec = compute_stft(&samples, FRAME, HOP).map_err(|e| format!("STFT: {e:?}"))?;
    println!("STFT: {} frames", spec.len());

    let beat_grid = match json_grid {
        Some(g) => {
            // Crop to the analysed window so per-bar stats reflect what we
            // actually saw. dub_stab tolerates trailing onsets past the last
            // beat by discarding them; we still need bars within range.
            let beats: Vec<f32> = g.beats.iter().copied().filter(|&t| t < dur_s).collect();
            let bars: Vec<f32> = g.bars.iter().copied().filter(|&t| t < dur_s).collect();
            let downbeats = bars.clone();
            BeatGrid {
                downbeats,
                beats,
                bars,
            }
        }
        None => {
            let bb_frames = detect_spectral_flux_onsets(&spec, 0.85)
                .map_err(|e| format!("spectral_flux: {e:?}"))?;
            let bb_times: Vec<f32> = bb_frames.iter().map(|&f| frame_to_time(f, sr)).collect();
            if bb_times.is_empty() {
                return Err("no broadband onsets — cannot build beat grid".to_string());
            }
            let (mut g, _stability) = generate_beat_grid(bpm_hint, 0.9, &bb_times, sr)
                .map_err(|e| format!("beat grid: {e:?}"))?;
            let raw_beats = g.beats.len();
            let raw_bars = g.bars.len();
            sanitize_grid(&mut g.beats);
            sanitize_grid(&mut g.bars);
            if g.beats.len() != raw_beats || g.bars.len() != raw_bars {
                println!(
                    "  (sanitized: beats {raw_beats} → {}, bars {raw_bars} → {})",
                    g.beats.len(),
                    g.bars.len()
                );
            }
            g
        }
    };
    println!(
        "Beat grid: {} beats, {} bars, {} downbeats",
        beat_grid.beats.len(),
        beat_grid.bars.len(),
        beat_grid.downbeats.len(),
    );

    let config = DubStabConfig::default();
    let stab_frames = detect_kick_disjoint_stab_onsets(&spec, sr, FRAME, HOP, &config)
        .map_err(|e| format!("Stage 1: {e:?}"))?;

    println!(
        "Stage 1: {} kick-disjoint stab onsets ({:.2}/s)",
        stab_frames.len(),
        stab_frames.len() as f32 / dur_s
    );

    let hist = beat_relative_offset_histogram(&stab_frames, HOP, sr, &beat_grid)
        .map_err(|e| format!("Stage 2: {e:?}"))?;

    print_histogram("Global histogram", &hist.global);

    // Per-bar summary: report bar count + offset-bin centroid stability.
    println!("\nPer-bar histograms: {} bars", hist.per_bar.len());
    let bar_peaks: Vec<usize> = hist
        .per_bar
        .iter()
        .map(|h| {
            h.iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .map(|(i, _)| i)
                .unwrap_or(0)
        })
        .collect();
    if !bar_peaks.is_empty() {
        // Modal peak bin across bars.
        let mut counts = [0usize; HISTOGRAM_BINS];
        for &p in &bar_peaks {
            counts[p] += 1;
        }
        let (modal_bin, modal_count) = counts
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| *c)
            .map(|(i, c)| (i, *c))
            .unwrap();
        println!(
            "  modal per-bar peak bin: {modal_bin} ({modal_count}/{} bars, {:.0}%)",
            hist.per_bar.len(),
            modal_count as f32 / hist.per_bar.len().max(1) as f32 * 100.0
        );
    }

    // Compact one-line summary that's grep-friendly across many tracks.
    let global = &hist.global;
    let on_beat = global[0]
        + global.get(1).copied().unwrap_or(0.0)
        + global.get(HISTOGRAM_BINS - 1).copied().unwrap_or(0.0);
    let off_beat = global[HISTOGRAM_BINS / 2]
        + global.get(HISTOGRAM_BINS / 2 - 1).copied().unwrap_or(0.0)
        + global.get(HISTOGRAM_BINS / 2 + 1).copied().unwrap_or(0.0);
    let total: f32 = global.iter().sum();
    let peak_bin = global
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);
    println!(
        "\nSUMMARY  stabs/s={:.2}  peak_bin={}  on/total={:.2}  off/total={:.2}  on/off={:.2}",
        stab_frames.len() as f32 / dur_s,
        peak_bin,
        if total > 0.0 { on_beat / total } else { 0.0 },
        if total > 0.0 { off_beat / total } else { 0.0 },
        if off_beat > 0.0 {
            on_beat / off_beat
        } else {
            f32::INFINITY
        }
    );

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!(
            "Usage: cargo run --release -p stratum-dsp --example dub_stab_real_audio -- \
             <path> <bpm> [grid_json]"
        );
        std::process::exit(1);
    }
    let path = &args[1];
    let bpm: f32 = args[2].parse().expect("bpm must be a number");
    // When a grid JSON is supplied we treat its absence/mismatch as a hard
    // error rather than silently falling back to HMM. The whole point of
    // running with --grid_json is to compare against the Rekordbox grid;
    // a quiet HMM run would invalidate the comparison without warning.
    let grids = args.get(3).map(|p| {
        load_grid_json(p).unwrap_or_else(|e| {
            eprintln!("Error: failed to load grid JSON: {e}");
            std::process::exit(2);
        })
    });
    let json_grid = grids.as_ref().and_then(|m| m.get(path));
    if grids.is_some() && json_grid.is_none() {
        eprintln!("Error: grid JSON has no entry for {path}");
        std::process::exit(2);
    }
    if let Err(e) = analyse(path, bpm, json_grid) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
