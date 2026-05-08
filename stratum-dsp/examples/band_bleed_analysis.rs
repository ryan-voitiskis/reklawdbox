//! Quantify how kick drums bleed into the chord-stab band (350–2000 Hz)
//! via attack-transient spectral leakage.
//!
//! Run: `cargo run --release --example band_bleed_analysis`
//!
//! Prints findings used to inform the chord-stab detector's kick-coincidence
//! mask width, HPSS preprocessing decision, and frequency-band edges.

use stratum_dsp::features::chroma::extractor::compute_stft;
use stratum_dsp::features::onset::band::detect_band_onsets;
use stratum_dsp::features::onset::hpss::hpss_decompose;

const SR: u32 = 44_100;
const FRAME: usize = 2048;
const HOP: usize = 512;
const KICK_BAND: (f32, f32) = (40.0, 200.0);
const STAB_BAND: (f32, f32) = (350.0, 2000.0);

fn ms_per_frame() -> f32 {
    HOP as f32 / SR as f32 * 1000.0
}

fn frame_to_hop_time_s(f: usize) -> f32 {
    f as f32 * HOP as f32 / SR as f32
}

fn frame_to_centre_time_s(f: usize) -> f32 {
    (f as f32 * HOP as f32 + FRAME as f32 / 2.0) / SR as f32
}

/// Synthesise a kick at the given time. `sharpness` controls envelope decay
/// rate (higher = sharper attack/faster decay). `harmonic_amps` is amplitudes
/// for the 60/180/540 Hz harmonics. `click_amp` is the amplitude of a 5 kHz
/// click burst at the attack moment (10 samples).
fn synth_kick(
    samples: &mut [f32],
    kick_t: f32,
    sharpness: f32,
    harmonic_amps: [f32; 3],
    click_amp: f32,
) {
    let start = (kick_t * SR as f32) as usize;
    let n = (SR as f32 * 0.1) as usize; // 100 ms support
    let harmonics = [60.0_f32, 180.0, 540.0];
    for i in 0..n {
        if start + i >= samples.len() {
            break;
        }
        let t = i as f32 / SR as f32;
        let env = (-t * sharpness).exp();
        let mut s = 0.0;
        for (h_idx, &h_freq) in harmonics.iter().enumerate() {
            s += (2.0 * std::f32::consts::PI * h_freq * t).sin() * env * harmonic_amps[h_idx];
        }
        if click_amp > 0.0 && i < 10 {
            s += (2.0 * std::f32::consts::PI * 5000.0 * t).sin() * click_amp;
        }
        samples[start + i] += s;
    }
}

/// Synthesise a 1 kHz stab tone burst at the given time.
fn synth_stab(samples: &mut [f32], stab_t: f32, amp: f32) {
    let start = (stab_t * SR as f32) as usize;
    let n = (SR as f32 * 0.2) as usize;
    for i in 0..n {
        if start + i >= samples.len() {
            break;
        }
        let t = i as f32 / SR as f32;
        let env = (-t * 30.0).exp();
        let s = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * env * amp;
        samples[start + i] += s;
    }
}

/// Compute the L2 half-wave-rectified flux time-series in a band, the same way
/// `detect_band_onsets` computes it internally — but return the flux values
/// rather than the picked peaks. Lets us see the full profile.
fn band_flux_profile(spec: &[Vec<f32>], band_hz: (f32, f32)) -> Vec<f32> {
    let bins_per_hz = FRAME as f32 / SR as f32;
    let bin_low = (band_hz.0 * bins_per_hz).ceil() as usize;
    let bin_high = (band_hz.1 * bins_per_hz).floor() as usize;
    let bin_high = bin_high.min(spec[0].len() - 1);

    let mut normalized: Vec<Vec<f32>> = Vec::with_capacity(spec.len());
    for frame in spec {
        let band = &frame[bin_low..=bin_high];
        let max_mag = band.iter().copied().fold(0.0_f32, f32::max);
        if max_mag > 1e-10 {
            normalized.push(band.iter().map(|&x| x / max_mag).collect());
        } else {
            normalized.push(vec![0.0; band.len()]);
        }
    }

    let mut flux = vec![0.0_f32];
    for i in 1..normalized.len() {
        let prev = &normalized[i - 1];
        let curr = &normalized[i];
        let sum_sq: f32 = prev
            .iter()
            .zip(curr.iter())
            .map(|(&p, &c)| (c - p).max(0.0).powi(2))
            .sum();
        flux.push(sum_sq.sqrt());
    }
    flux
}

fn print_section(title: &str) {
    println!("\n========================================================");
    println!("  {}", title);
    println!("========================================================");
}

fn main() {
    println!(
        "STFT config: sr={} Hz, frame={} samples ({:.1} ms), hop={} samples ({:.2} ms)",
        SR,
        FRAME,
        FRAME as f32 / SR as f32 * 1000.0,
        HOP,
        ms_per_frame()
    );
    println!(
        "Kick band: {:.0}–{:.0} Hz | Stab band: {:.0}–{:.0} Hz",
        KICK_BAND.0, KICK_BAND.1, STAB_BAND.0, STAB_BAND.1
    );

    // ====================================================================
    print_section("Experiment 1: Pre-echo and post-echo extent of a single kick");
    // ====================================================================
    {
        let n_samples = (SR as f32 * 2.0) as usize;
        let mut samples = vec![0.0_f32; n_samples];
        synth_kick(&mut samples, 1.0, 60.0, [0.8, 0.3, 0.0], 0.0);
        let spec = compute_stft(&samples, FRAME, HOP).unwrap();

        let kick_flux = band_flux_profile(&spec, KICK_BAND);
        let stab_flux = band_flux_profile(&spec, STAB_BAND);

        // Find the single peak in each band.
        let kick_peak = kick_flux
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        let stab_peak = stab_flux
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();

        println!("Kick at t=1.000 s. Frames containing kick samples: 83..=86 (math: 512k ≤ 44100 < 512k+2048).");
        println!();
        println!("Kick band ({:.0}–{:.0} Hz):", KICK_BAND.0, KICK_BAND.1);
        println!(
            "  peak frame {} | hop_time {:.4} s | centre_time {:.4} s | flux {:.3}",
            kick_peak.0,
            frame_to_hop_time_s(kick_peak.0),
            frame_to_centre_time_s(kick_peak.0),
            kick_peak.1
        );
        println!(
            "  hop_time error: {:+.1} ms (negative = pre-echo: reported time is before kick)",
            (frame_to_hop_time_s(kick_peak.0) - 1.0) * 1000.0
        );
        println!(
            "  centre_time error: {:+.1} ms",
            (frame_to_centre_time_s(kick_peak.0) - 1.0) * 1000.0
        );

        println!();
        println!("Stab band ({:.0}–{:.0} Hz):", STAB_BAND.0, STAB_BAND.1);
        println!(
            "  peak frame {} | hop_time {:.4} s | centre_time {:.4} s | flux {:.3}",
            stab_peak.0,
            frame_to_hop_time_s(stab_peak.0),
            frame_to_centre_time_s(stab_peak.0),
            stab_peak.1
        );
        println!(
            "  hop_time error: {:+.1} ms",
            (frame_to_hop_time_s(stab_peak.0) - 1.0) * 1000.0
        );

        // Print flux profile for ±10 frames around frame 85.
        println!();
        println!("Stab-band flux profile around the kick (frame ±10):");
        println!("  frame | hop_time | centre_t | stab_flux | kick_flux");
        for f in 75..=95 {
            let mark = if f == stab_peak.0 { " *" } else { "" };
            println!(
                "  {:>5} | {:>8.4} | {:>8.4} | {:>9.4} | {:>9.4}{}",
                f,
                frame_to_hop_time_s(f),
                frame_to_centre_time_s(f),
                stab_flux[f],
                kick_flux[f],
                mark
            );
        }

        // Find frames where stab flux exceeds 10% of its peak — the "bleed extent".
        let stab_thresh = stab_peak.1 * 0.1;
        let bleed_frames: Vec<usize> = (0..stab_flux.len())
            .filter(|&f| stab_flux[f] > stab_thresh && (f as i32 - stab_peak.0 as i32).abs() < 50)
            .collect();
        let first = *bleed_frames.first().unwrap();
        let last = *bleed_frames.last().unwrap();
        println!();
        println!("Bleed extent (frames where stab flux > 10% of peak):");
        println!(
            "  first frame {} ({:+.1} ms vs kick) ... last frame {} ({:+.1} ms vs kick)",
            first,
            (frame_to_hop_time_s(first) - 1.0) * 1000.0,
            last,
            (frame_to_hop_time_s(last) - 1.0) * 1000.0
        );
        println!(
            "  total span: {} frames ≈ {:.1} ms",
            last - first + 1,
            (last - first + 1) as f32 * ms_per_frame()
        );
    }

    // ====================================================================
    print_section("Experiment 2: Kick variation — which kicks bleed worst");
    // ====================================================================
    {
        let variants: &[(&str, f32, [f32; 3], f32)] = &[
            ("sub only (60 Hz, slow)", 30.0, [0.8, 0.0, 0.0], 0.0),
            ("sub only (60 Hz, default)", 60.0, [0.8, 0.0, 0.0], 0.0),
            ("sub only (60 Hz, sharp)", 120.0, [0.8, 0.0, 0.0], 0.0),
            ("60+180 Hz (default test kick)", 60.0, [0.8, 0.3, 0.0], 0.0),
            ("60+180+540 Hz", 60.0, [0.8, 0.3, 0.2], 0.0),
            ("60 Hz + 5 kHz click", 60.0, [0.8, 0.0, 0.0], 0.4),
            ("full kick + click", 60.0, [0.8, 0.3, 0.2], 0.4),
        ];

        println!("Each kick at t=1.0 s. Reporting peak stab-band flux and ratio");
        println!("vs the kick-band peak (lower ratio = less bleed).\n");
        println!(
            "  {:<32} | stab_peak | kick_peak | stab/kick ratio",
            "kick variant"
        );
        println!("  {:-<32}-+-----------+-----------+----------------", "");

        for &(name, sharp, harm, click) in variants {
            let n_samples = (SR as f32 * 2.0) as usize;
            let mut samples = vec![0.0_f32; n_samples];
            synth_kick(&mut samples, 1.0, sharp, harm, click);
            let spec = compute_stft(&samples, FRAME, HOP).unwrap();
            let kf = band_flux_profile(&spec, KICK_BAND);
            let sf = band_flux_profile(&spec, STAB_BAND);
            let kpk = kf.iter().copied().fold(0.0_f32, f32::max);
            let spk = sf.iter().copied().fold(0.0_f32, f32::max);
            println!(
                "  {:<32} | {:>9.3} | {:>9.3} | {:>15.3}",
                name,
                spk,
                kpk,
                spk / kpk.max(1e-9)
            );
        }
    }

    // ====================================================================
    print_section("Experiment 3: Stab vs kick bleed — SNR in the stab band");
    // ====================================================================
    {
        // Kick alone.
        let n_samples = (SR as f32 * 2.0) as usize;
        let mut samples_k = vec![0.0_f32; n_samples];
        synth_kick(&mut samples_k, 1.0, 60.0, [0.8, 0.3, 0.0], 0.0);
        let spec_k = compute_stft(&samples_k, FRAME, HOP).unwrap();
        let sf_k = band_flux_profile(&spec_k, STAB_BAND);
        let kick_only_stab = sf_k.iter().copied().fold(0.0_f32, f32::max);

        // Stab alone (1 kHz, amplitude 0.6).
        let mut samples_s = vec![0.0_f32; n_samples];
        synth_stab(&mut samples_s, 1.0, 0.6);
        let spec_s = compute_stft(&samples_s, FRAME, HOP).unwrap();
        let sf_s = band_flux_profile(&spec_s, STAB_BAND);
        let stab_only_stab = sf_s.iter().copied().fold(0.0_f32, f32::max);

        // Stab + kick simultaneously at t=1.0s.
        let mut samples_sk = vec![0.0_f32; n_samples];
        synth_kick(&mut samples_sk, 1.0, 60.0, [0.8, 0.3, 0.0], 0.0);
        synth_stab(&mut samples_sk, 1.0, 0.6);
        let spec_sk = compute_stft(&samples_sk, FRAME, HOP).unwrap();
        let sf_sk = band_flux_profile(&spec_sk, STAB_BAND);
        let combined_stab = sf_sk.iter().copied().fold(0.0_f32, f32::max);

        println!("  scenario                             | peak stab-band flux");
        println!("  {:-<37}-+-{:->20}", "", "");
        println!(
            "  kick only (60+180 Hz, default)       | {:.3}",
            kick_only_stab
        );
        println!(
            "  stab only (1 kHz, amp 0.6)           | {:.3}",
            stab_only_stab
        );
        println!(
            "  stab + kick simultaneous             | {:.3}",
            combined_stab
        );
        println!();
        println!(
            "  stab/kick-bleed ratio: {:.2}× (higher = stab dominates → easier to detect)",
            stab_only_stab / kick_only_stab.max(1e-9)
        );
        println!(
            "  on-kick stab uplift over kick-only:  {:.2}× (does the stab still 'pop'?)",
            combined_stab / kick_only_stab.max(1e-9)
        );
    }

    // ====================================================================
    print_section("Experiment 4: HPSS suppression — does it help?");
    // ====================================================================
    {
        // 4 kicks at half-second intervals (4-on-floor at 120 BPM).
        let n_samples = (SR as f32 * 3.0) as usize;
        let mut samples = vec![0.0_f32; n_samples];
        for &t in &[0.5_f32, 1.0, 1.5, 2.0] {
            synth_kick(&mut samples, t, 60.0, [0.8, 0.3, 0.0], 0.0);
        }
        // Plus a real stab at 1.25s and 2.0s (the "alongside-kick" case).
        synth_stab(&mut samples, 1.25, 0.6);
        synth_stab(&mut samples, 2.00, 0.6);

        let spec = compute_stft(&samples, FRAME, HOP).unwrap();

        let raw_onsets = detect_band_onsets(&spec, SR, FRAME, STAB_BAND, 0.85).unwrap();

        let (harmonic, _percussive) = hpss_decompose(&spec, 17).unwrap();
        let hpss_onsets = detect_band_onsets(&harmonic, SR, FRAME, STAB_BAND, 0.85).unwrap();

        let to_times = |frames: &[usize]| -> Vec<f32> {
            frames.iter().map(|&f| frame_to_hop_time_s(f)).collect()
        };

        let raw_times = to_times(&raw_onsets);
        let hpss_times = to_times(&hpss_onsets);

        println!("Audio: 4 kicks at 0.5/1.0/1.5/2.0 s + stabs at 1.25 s and 2.0 s.");
        println!("Stab-band onsets, raw spectrogram:  {:?}", raw_times);
        println!("Stab-band onsets, HPSS harmonic:    {:?}", hpss_times);

        // Categorise each onset as either a true stab (within 100 ms of 1.25 or 2.0)
        // or a kick bleed (within 100 ms of 0.5/1.0/1.5/2.0 but NOT at 1.25).
        let stabs: &[f32] = &[1.25, 2.0];
        let kicks_only: &[f32] = &[0.5, 1.0, 1.5];

        let count_hits = |onsets: &[f32], targets: &[f32]| -> usize {
            targets
                .iter()
                .filter(|&&tgt| onsets.iter().any(|&t| (t - tgt).abs() < 0.1))
                .count()
        };

        let count_false_kicks = |onsets: &[f32]| -> usize {
            onsets
                .iter()
                .filter(|&&t| kicks_only.iter().any(|&kt| (t - kt).abs() < 0.1))
                .count()
        };

        println!();
        println!("  metric                              | raw STFT | HPSS-harmonic");
        println!("  {:-<36}-+-{:-<8}-+-{:-<14}", "", "", "");
        println!(
            "  true stabs found / 2                | {:>8} | {:>14}",
            count_hits(&raw_times, stabs),
            count_hits(&hpss_times, stabs)
        );
        println!(
            "  kick-only times producing onsets    | {:>8} | {:>14}",
            count_false_kicks(&raw_times),
            count_false_kicks(&hpss_times)
        );
    }

    // ====================================================================
    print_section("Experiment 5: Mask-width sufficiency for 4-on-floor kicks");
    // ====================================================================
    {
        let n_samples = (SR as f32 * 3.0) as usize;
        let mut samples = vec![0.0_f32; n_samples];
        for &t in &[0.5_f32, 1.0, 1.5, 2.0] {
            synth_kick(&mut samples, t, 60.0, [0.8, 0.3, 0.0], 0.0);
        }

        let spec = compute_stft(&samples, FRAME, HOP).unwrap();
        let stab_onsets = detect_band_onsets(&spec, SR, FRAME, STAB_BAND, 0.85).unwrap();
        let kick_times: &[f32] = &[0.5, 1.0, 1.5, 2.0];

        let stab_times: Vec<f32> = stab_onsets
            .iter()
            .map(|&f| frame_to_hop_time_s(f))
            .collect();

        println!("Kick-only audio. All stab-band onsets are kick bleed.");
        println!("Stab-band onset times: {:?}", stab_times);

        // For each onset, find offset to nearest kick.
        let offsets: Vec<f32> = stab_times
            .iter()
            .map(|&t| {
                kick_times
                    .iter()
                    .map(|&kt| t - kt)
                    .min_by(|a, b| a.abs().partial_cmp(&b.abs()).unwrap())
                    .unwrap()
            })
            .collect();

        println!("Offsets to nearest kick (signed, ms):");
        for (i, &o) in offsets.iter().enumerate() {
            println!(
                "  onset {} at {:.4} s → {:+7.1} ms from nearest kick",
                i,
                stab_times[i],
                o * 1000.0
            );
        }

        let max_abs = offsets.iter().map(|o| o.abs()).fold(0.0_f32, f32::max);
        println!();
        println!(
            "Max |offset| = {:.1} ms → mask must be at least ±{:.0} ms to cover all bleed",
            max_abs * 1000.0,
            (max_abs * 1000.0).ceil()
        );

        // Now sweep candidate mask widths and report kept-onset count.
        let mut widths = [10.0_f32, 20.0, 30.0, 40.0, 50.0, 60.0, 80.0];
        widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
        println!();
        println!("  mask width | kick-bleed onsets surviving (lower = better)");
        for w in widths {
            let surviving = stab_times
                .iter()
                .filter(|&&t| !kick_times.iter().any(|&kt| (t - kt).abs() * 1000.0 <= w))
                .count();
            println!("  ±{:>4.0} ms   | {}/{}", w, surviving, stab_times.len());
        }
    }

    // ====================================================================
    print_section("Experiment 6: Pre-echo dependence on STFT centring convention");
    // ====================================================================
    {
        // Detector returns frame indices. Caller maps to time. The chord-stab
        // plan uses `i * hop / sr` (HOP-time). With left-aligned Hann frames,
        // the stab-band peak typically lands at frame whose CENTRE is closest
        // to the kick — so HOP-time is biased ~ -frame/(2·sr) ms.
        let n_samples = (SR as f32 * 2.0) as usize;
        let mut samples = vec![0.0_f32; n_samples];
        synth_kick(&mut samples, 1.0, 60.0, [0.8, 0.3, 0.0], 0.0);
        let spec = compute_stft(&samples, FRAME, HOP).unwrap();

        let onsets_stab = detect_band_onsets(&spec, SR, FRAME, STAB_BAND, 0.95).unwrap();
        let onsets_kick = detect_band_onsets(&spec, SR, FRAME, KICK_BAND, 0.85).unwrap();

        println!("Single kick at t=1.000 s.");
        println!(
            "Kick-band onset frames: {:?} | hop-times {:?}",
            onsets_kick,
            onsets_kick
                .iter()
                .map(|&f| frame_to_hop_time_s(f))
                .collect::<Vec<_>>()
        );
        println!(
            "Stab-band onset frames: {:?} | hop-times {:?}",
            onsets_stab,
            onsets_stab
                .iter()
                .map(|&f| frame_to_hop_time_s(f))
                .collect::<Vec<_>>()
        );

        let half_window_ms = (FRAME as f32 / 2.0) / SR as f32 * 1000.0;
        println!();
        println!(
            "Half-window time = {:.1} ms — this is the structural offset between hop-time",
            half_window_ms
        );
        println!("and the actual sample at the centre of the FFT frame. A correctly-centred kick");
        println!(
            "appears at HOP-time ≈ kick_time − {:.0} ms. The chord-stab plan masks based on",
            half_window_ms
        );
        println!("kick-band ONSET TIMES (also in hop-time), so this offset cancels out for kicks.");
        println!("But ANY use of absolute time (e.g. comparing to a beat grid in centre-time)");
        println!("would need to add {:.0} ms.", half_window_ms);
    }

    println!();
    println!("=== Done ===");
}
