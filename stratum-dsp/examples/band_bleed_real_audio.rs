//! Real-audio validation of the synthetic-only kick-bleed findings.
//!
//! Decodes a track via symphonia, downmixes to mono, runs:
//! - kick-band onsets (treated as ground-truth kick times)
//! - stab-band onsets, raw STFT
//! - stab-band onsets, HPSS harmonic component
//!
//! Reports kick-coincidence rates so we can compare raw vs HPSS suppression
//! on real audio with continuous harmonic content (pads, basses), which is
//! the case the synthetic kick-only test could not exercise.
//!
//! Run: `cargo run --release -p stratum-dsp --example band_bleed_real_audio -- <path>`

use std::env;

use stratum_dsp::features::chroma::extractor::compute_stft;
use stratum_dsp::features::onset::band::detect_band_onsets;
use stratum_dsp::features::onset::hpss::hpss_decompose;

use symphonia::core::audio::AudioBufferRef;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

const FRAME: usize = 2048;
const HOP: usize = 512;
const KICK_BAND: (f32, f32) = (40.0, 200.0);
const STAB_BAND: (f32, f32) = (350.0, 2000.0);
const ANALYSIS_DURATION_S: f32 = 60.0; // cap to keep HPSS cost reasonable

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

/// For each onset in `onsets`, find signed offset (seconds) to nearest reference.
fn nearest_offsets(onsets: &[f32], refs: &[f32]) -> Vec<f32> {
    onsets
        .iter()
        .map(|&t| {
            refs.iter()
                .map(|&r| t - r)
                .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
                .unwrap_or(f32::INFINITY)
        })
        .collect()
}

fn count_within(offsets: &[f32], ms: f32) -> usize {
    let s = ms / 1000.0;
    offsets.iter().filter(|o| o.abs() <= s).count()
}

/// Print signed-offset histogram in 10 ms bins from -200 to +200 ms.
fn print_offset_histogram(offsets: &[f32], label: &str) {
    let bins = 40;
    let bin_width_ms = 10.0;
    let lo_ms = -200.0;
    let mut counts = vec![0usize; bins];
    for &o in offsets {
        let ms = o * 1000.0;
        if ms.abs() > 200.0 {
            continue;
        }
        let idx = ((ms - lo_ms) / bin_width_ms) as usize;
        let idx = idx.min(bins - 1);
        counts[idx] += 1;
    }
    let max = *counts.iter().max().unwrap_or(&1).max(&1);
    println!("\n{label} (signed offset to nearest kick, 10 ms bins, |offset| ≤ 200 ms):");
    for (i, &c) in counts.iter().enumerate() {
        let lo = lo_ms + i as f32 * bin_width_ms;
        let bar_len = (c as f32 / max as f32 * 40.0) as usize;
        let bar: String = std::iter::repeat_n('#', bar_len).collect();
        println!("  {:>+5.0} ms | {:>3} {}", lo, c, bar);
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: cargo run --release -p stratum-dsp --example band_bleed_real_audio -- <audio path>");
        std::process::exit(1);
    }
    let path = &args[1];

    println!("Loading: {path}");
    let (samples, sr) = match decode_mono(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Decode error: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "Decoded {} samples ({:.1} s) at {} Hz",
        samples.len(),
        samples.len() as f32 / sr as f32,
        sr
    );

    println!("Computing STFT (frame={FRAME}, hop={HOP})...");
    let spec = compute_stft(&samples, FRAME, HOP).expect("STFT failed");
    println!("STFT: {} frames", spec.len());

    println!("Computing HPSS (margin=17, this may take ~30s)...");
    let t0 = std::time::Instant::now();
    let (harmonic, _percussive) = hpss_decompose(&spec, 17).expect("HPSS failed");
    println!("HPSS done in {:.1}s", t0.elapsed().as_secs_f32());

    let percentile = 0.85;
    let kick_onsets_frames =
        detect_band_onsets(&spec, sr, FRAME, KICK_BAND, percentile).expect("kick onsets");
    let stab_raw_frames =
        detect_band_onsets(&spec, sr, FRAME, STAB_BAND, percentile).expect("stab raw");
    let stab_hpss_frames =
        detect_band_onsets(&harmonic, sr, FRAME, STAB_BAND, percentile).expect("stab hpss");

    let to_t =
        |frames: &[usize]| -> Vec<f32> { frames.iter().map(|&f| frame_to_time(f, sr)).collect() };
    let kicks = to_t(&kick_onsets_frames);
    let stab_raw = to_t(&stab_raw_frames);
    let stab_hpss = to_t(&stab_hpss_frames);

    println!();
    println!("=== Onset counts (percentile {percentile}) ===");
    println!("  kick-band (40–200 Hz, ground truth): {}", kicks.len());
    println!("  stab-band raw STFT:                  {}", stab_raw.len());
    println!("  stab-band HPSS harmonic:             {}", stab_hpss.len());

    if kicks.is_empty() {
        eprintln!("No kicks detected — cannot compute coincidence stats.");
        return;
    }

    let off_raw = nearest_offsets(&stab_raw, &kicks);
    let off_hpss = nearest_offsets(&stab_hpss, &kicks);

    println!();
    println!("=== Kick-coincidence at various windows ===");
    println!("  window   | stab-raw within | stab-HPSS within | raw kick-rate | HPSS kick-rate | suppression");
    println!("  ---------+-----------------+------------------+---------------+----------------+------------");
    for &ms in &[20.0_f32, 30.0, 50.0, 100.0, 150.0] {
        let r = count_within(&off_raw, ms);
        let h = count_within(&off_hpss, ms);
        let raw_rate = r as f32 / stab_raw.len().max(1) as f32 * 100.0;
        let hpss_rate = h as f32 / stab_hpss.len().max(1) as f32 * 100.0;
        let suppression = 1.0 - (h as f32 / r.max(1) as f32);
        println!(
            "  ±{:>4.0} ms | {:>15} | {:>16} | {:>12.1}% | {:>13.1}% | {:>10.1}%",
            ms,
            r,
            h,
            raw_rate,
            hpss_rate,
            suppression * 100.0
        );
    }

    println!();
    println!("=== Non-coincident retention (control: HPSS shouldn't kill non-kick events) ===");
    {
        let ms = 100.0_f32;
        let s = ms / 1000.0;
        let r_far = off_raw.iter().filter(|o| o.abs() > s).count();
        let h_far = off_hpss.iter().filter(|o| o.abs() > s).count();
        let retention = h_far as f32 / r_far.max(1) as f32 * 100.0;
        println!(
            "  Onsets >|{:.0} ms| from any kick: raw {} → HPSS {} (retention {:.1}%)",
            ms, r_far, h_far, retention
        );
    }

    print_offset_histogram(&off_raw, "RAW stab-band onsets");
    print_offset_histogram(&off_hpss, "HPSS-harmonic stab-band onsets");
}
