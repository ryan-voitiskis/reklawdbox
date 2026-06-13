# Sub-Rumble and Sidechain Depth Detectors: Implementation Plan

**Date:** 2026-04-26
**Status:** Design proposal. No implementation yet. Validation strategy included; **wire-into-classification gated on validation pass**.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) — features A4 and A5 from that doc. [chord-stab-detector-plan.md](chord-stab-detector-plan.md) — structural template; PR breakdown and validation methodology mirror that plan.

## Goal

Add two stratum-dsp features that operate on amplitude envelopes within specific frequency bands, beat-aligned via the existing beat grid:

- **`sub_rumble_proportion: f32 ∈ [0, 1]`** — sustained low-band energy *between* kicks vs energy *during* kicks. High in Deep Techno (Berghain rumble); near zero in Tech House (sidechain ducks the lows).
- **`sidechain_depth: f32 ∈ [0, 1]`** — depth of beat-rate amplitude modulation in the mid band (the duck-and-recover pumping). High in Tech House and modern House; zero in Deep Techno and Dub Techno.

These are grouped because both reduce to "RMS envelope in a frequency band, sliced against the beat grid." Most code is shared. Cost target combined: under 5% of total analysis time, reusing the existing shared STFT and beat grid.

Neither signal can be derived from any currently-cached feature.

## Why now

The decision tree currently has no signal that distinguishes Deep Techno from Tech House when enrichment is ambiguous. They have similar BPM (124–128), 4/4 kicks, often similar timbral statistics. The two features that hear the difference instantly — sub-bass rumble vs sidechain pumping — are inverse signatures of each other: a track with high `sub_rumble_proportion` cannot have high `sidechain_depth` and vice versa. Computing them together gives the tree a clean two-axis split for that boundary.

## Shared Infrastructure (do this first)

### S1. Bandpass primitive — FFT-bin selection, not a time-domain filter

stratum-dsp does not have a reusable time-domain bandpass filter. The established pattern (used by `decay.rs`, `modulation.rs`, the proposed `dub_stab` band-onset detection) is **frequency-bin selection on the existing shared STFT**. Both A4 and A5 follow this.

Add a new helper in `stratum-dsp/src/features/envelope.rs`:

```
pub fn band_rms_envelope(
    magnitude_spec_frames: &[Vec<f32>],
    sample_rate: u32,
    band_hz: (f32, f32),
) -> Vec<f32>
```

Returns a per-frame RMS envelope summing magnitude squared over the bins in `band_hz`, then sqrt. One value per STFT frame (envelope sample rate ≈ 86 Hz at sr=44100, hop=512). This is the single primitive both detectors call, with different bands.

### S2. Beat-windowed envelope analyzer

Both detectors need to slice the envelope by beat interval. Add a helper in the same module:

```
pub struct BeatWindow {
    pub start_frame: usize,   // inclusive
    pub end_frame: usize,     // exclusive
    pub kick_end_frame: usize, // start_frame + ceil(50ms / ms_per_frame), capped at end_frame
}

pub fn beat_windows(
    beat_grid: &BeatGrid,
    n_frames: usize,
    ms_per_frame: f32,
    kick_window_ms: f32, // default 50.0
) -> Vec<BeatWindow>
```

For each beat in `beat_grid.beats`, produce a window from that beat to the next beat, with a "kick portion" (first `kick_window_ms`) and an "off-kick portion" (the remainder). At low BPM, `kick_window_ms` may exceed the beat interval; clamp `kick_end_frame` to `end_frame`.

Both detectors iterate these windows. This is the shared abstraction the parent doc identified.

### S3. Note on `mod_centroid` infrastructure reuse

`stratum-dsp/src/features/modulation.rs` already FFTs amplitude envelopes (per-band) for the `mod_centroid` feature. **`mod_centroid` itself is empirically invalidated** for genre discrimination (see `genre-classification-improvements.md` § Empirically Invalidated Features) and must not be reintroduced as a discriminator. But the *envelope-FFT machinery* — building an amplitude envelope, FFT'ing it, indexing modulation-frequency bins — is exactly what A5 needs at a single frequency (the beat rate). Refactor that machinery into a shared helper in `envelope.rs` so both `modulation.rs` and the new `sidechain.rs` call it. Do not delete or modify the `mod_centroid` output field; leave the invalidated feature in place but unread by the classifier.

---

## A4: Sub-Rumble Proportion

### Signal definition

A track has high sub-rumble if there is sustained low-frequency energy *between* kick transients, in a band *below* the kick fundamental. Specifically:

1. **Sub band**: 30–60 Hz — sub-bass region. Below the typical kick fundamental (60–80 Hz) but above the DC/rumble floor (<30 Hz, typically inaudible/subwoofer-only).
2. **Kick band**: 60–100 Hz — kick-drum fundamental and second harmonic.
3. The metric is the **ratio of sub-band RMS in the off-kick portion of each beat to kick-band RMS in the kick portion**, averaged over beats.

This separates Deep Techno's signature "rumble between kicks" (high ratio) from Tech House (sidechain ducks both the sub and the mids during the kick recovery, so the off-kick sub is near zero). Drone Techno without a kick will have undefined kick-band reference; handle as edge case.

### Algorithm

```
sub_env  = band_rms_envelope(stft, sr, (30.0, 60.0))    // S1
kick_env = band_rms_envelope(stft, sr, (60.0, 100.0))   // S1
windows  = beat_windows(beat_grid, n_frames, ms_per_frame, 50.0)  // S2

For each window:
  kick_rms[i]    = mean(kick_env[start..kick_end])
  sub_off_rms[i] = mean(sub_env[kick_end..end])
  sub_on_rms[i]  = mean(sub_env[start..kick_end])

Robust kick reference:
  kick_ref = median(kick_rms across beats)
  if kick_ref < KICK_PRESENCE_FLOOR (e.g. 1e-4): return None (no kick — see edge cases)

Per-beat ratio:
  r[i] = sub_off_rms[i] / max(kick_ref, eps)
  // Optionally subtract sub_on_rms[i] component to discount the kick's own
  // sub-band leakage — but the sub-band is below the kick fundamental, so
  // this should be small. Validate empirically.

Aggregate:
  raw_proportion = median(r[i] across beats)
  sub_rumble_proportion = squash(raw_proportion, 0.5, 1.5)
  // squash: piecewise-linear so that:
  //   raw <= 0.05 → 0.0  (no rumble)
  //   raw  = 0.5  → 0.5  (typical Deep Techno)
  //   raw >= 1.5  → 1.0  (extreme rumble, sub louder than kick)
  // Calibrate squash anchors against the validation set (§ Validation).
```

Median rather than mean to resist transient outliers (snare fills, breakdowns).

### Edge cases

1. **Off-beat / broken-beat kicks (Electro)**: Kicks don't land in the first 50 ms of the beat window. Mitigation: detect this via the kick-pattern classifier (A2 from parent doc) when it lands; for now, also report `sub_rumble_proportion = None` if `kick_band_alignment_score` (computed inline: fraction of beats with kick_rms[i] > 2 × median kick_rms) is below 0.5. Without good kick alignment the metric is noise.

2. **No kick at all (Drone Techno)**: `kick_ref` falls below `KICK_PRESENCE_FLOOR`. Return `None`. Drone Techno is correctly characterised by *absence* of this feature, not by a numeric value. The classifier should handle the `None` case as "feature inapplicable" rather than "feature is zero."

3. **Very low BPM**: At 60 BPM, beat interval is 1000 ms; the 50 ms kick window is only 5 % of the beat. The off-kick window is 950 ms of mostly tail — actually a *better* signal for rumble. No mitigation needed beyond the existing `kick_window_ms` clamp in S2.

4. **Tracks with sidechained sub-bass that *isn't* sidechain pumping** (e.g. Trap with deliberately-ducked 808s): These will score near zero on `sub_rumble_proportion`, correctly indicating no rumble. Not a misclassification — sidechained 808s genuinely lack between-kick rumble.

5. **Halftime tracks**: Beat grid is half-density. The 50 ms kick window is unchanged. Off-kick window is twice as long. Should still work; validate.

### Output

```rust
pub struct SubRumbleResult {
    pub proportion: Option<f32>,    // None if kick reference invalid
    pub kick_alignment: f32,        // 0.0–1.0, fraction of beats with strong kick
    pub raw_ratio: Option<f32>,     // pre-squash, for debug
    pub usable_beats: u32,
}
```

`AnalysisResult.sub_rumble_proportion: Option<f32>` for the cache.

---

## A5: Sidechain Depth

### Signal definition

A track has high sidechain depth if the mid-band amplitude is strongly amplitude-modulated at the beat rate, with a duck-and-recover pattern:

1. **Mid band**: 250–2000 Hz. Sidechain is typically applied to pads, plucks, and mids — not the kick itself. This is the canonical Tech House sidechain band. (250 Hz lower bound stays clear of bass-band ducking, which has different dynamics.)
2. **Modulation frequency**: 60 / BPM Hz. At 125 BPM that's ≈ 2.08 Hz. At 130 BPM, ≈ 2.17 Hz.
3. **Modulation depth**: peak-to-trough amplitude swing in the mid envelope, normalised so 0 = no modulation, 1 = the envelope drops to zero at each beat.

### Algorithm — primary (frequency-domain)

Reuse the envelope-FFT machinery factored from `mod_centroid` (§ S3).

```
mid_env = band_rms_envelope(stft, sr, (250.0, 2000.0))    // S1
mid_env = trim(mid_env, 15%)                              // existing convention
mid_env_log = log(mid_env + eps)                          // dB-domain modulation
                                                           // (optional; validate)
fft_mid_env = FFT(mid_env)

beat_rate_hz = bpm / 60.0
mod_freq_resolution = envelope_sr / fft_size
beat_bin = round(beat_rate_hz / mod_freq_resolution)

// Look at a small range of bins around the beat rate to absorb tempo wobble
peak_mag = max(|fft_mid_env[beat_bin - 1 ..= beat_bin + 1]|)

// Modulation depth from the AM identity: for x(t) = A(1 + m·cos(ωt)),
// the FFT magnitude at ω is A·m/2 and at DC is A.
dc_mag = |fft_mid_env[0]|
modulation_depth = clamp(2.0 * peak_mag / max(dc_mag, eps), 0.0, 1.0)

// Confirm the peak is at the beat rate, not a harmonic
half_beat_mag = |fft_mid_env[beat_bin / 2]|       // half-time pumping
double_beat_mag = |fft_mid_env[2 * beat_bin]|     // 8th-note pumping
if half_beat_mag > peak_mag * 1.5:
    flag as halftime_sidechain                    // record but still report
if double_beat_mag > peak_mag * 1.5:
    modulation_depth *= 0.5                       // 8th-note tremolo, not sidechain

sidechain_depth = modulation_depth
```

### Algorithm — secondary (time-domain) confirmation

Run alongside the primary method as a sanity check. Uses S2 windows directly.

```
For each beat window:
  start_rms = mean(mid_env[start_frame .. start_frame + 50ms])     // ducked
  end_rms   = mean(mid_env[end_frame - 50ms .. end_frame])         // recovered

Aggregate:
  duck_ratio = median(start_rms / end_rms across beats)
  // duck_ratio near 0.0 = full sidechain (silent on the beat, recovered by the next)
  // duck_ratio near 1.0 = no sidechain (level is constant)

time_domain_depth = clamp(1.0 - duck_ratio, 0.0, 1.0)
```

If primary and time-domain depths disagree by more than 0.3, flag the track as ambiguous and report the lower of the two (conservative). Both methods agreeing is cheap insurance against artefacts (long pads with FM modulation, severe dynamics processing).

### Edge cases

1. **Halftime sidechain** (kick on every other beat, sidechain follows): Modulation peak at beat_bin/2 not beat_bin. Detected by the half_beat check above; report the depth at the dominant peak rather than zero. The classifier can treat halftime sidechain as a Tech House signature too — it is.

2. **Tremolo at non-beat rates** (synth LFO at 4 Hz on a 125 BPM track ≈ 1.92× beat rate): No peak at beat_bin, so `sidechain_depth` is correctly low. The double_beat check avoids confusion with deliberate 8th-note tremolo.

3. **Manual envelope shaping in Drone Techno / Ambient**: Slow, irregular envelope changes that don't lock to the beat grid. Will not produce a peak at the beat rate. Correctly scored zero.

4. **Strong synth tremolo that mimics sidechain at the beat rate**: Genuine ambiguity. Acoustically these *do* sound like sidechain. Acceptable to score them as sidechain — Tech House producers also use beat-rate tremolo intentionally; the genre signature is the audible pumping, not the production technique.

5. **Tracks with very low `grid_stability`**: Beat rate is unreliable. Mitigation: short-circuit and return `None` if `grid_stability < 0.4`, matching the `dub_stab` plan's guard. Better than reporting a confidently-wrong number.

6. **Breakdowns and intros without kick / sidechain**: The middle 70 % trim already handled by stratum-dsp convention, plus median aggregation, should make this robust. Validate.

### Output

```rust
pub struct SidechainResult {
    pub depth: Option<f32>,            // None if grid unreliable
    pub method_agreement: f32,         // |primary - time_domain|
    pub halftime_flag: bool,
    pub usable_beats: u32,
}
```

`AnalysisResult.sidechain_depth: Option<f32>` for the cache.

---

## Integration Points (shared)

### stratum-dsp side

1. **New module:** `stratum-dsp/src/features/envelope.rs` — shared primitives `band_rms_envelope`, `beat_windows`, `BeatWindow`. ~100 LOC.
2. **New module:** `stratum-dsp/src/features/sub_rumble.rs` — `detect_sub_rumble(stft, beat_grid, sample_rate, config) -> SubRumbleResult`. ~150 LOC.
3. **New module:** `stratum-dsp/src/features/sidechain.rs` — `detect_sidechain(stft, beat_grid, bpm, sample_rate, config) -> SidechainResult`. ~200 LOC.
4. **Module registration:** `stratum-dsp/src/features/mod.rs` — `pub mod envelope; pub mod sub_rumble; pub mod sidechain;`.
5. **Refactor** the per-band envelope construction inside `modulation.rs` to call `envelope::band_rms_envelope`. Keep `mod_centroid` output unchanged (`AnalysisResult.mod_centroid`); this is purely a code dedup. Do not extend its lifetime as a discriminator.
6. **Pipeline wiring:** `stratum-dsp/src/lib.rs:1593–1620`, alongside `mod_centroid`, `harmonic_proportion`, `decay`. Both detectors run after the beat grid is built (line 913). They depend on `beat_grid` and the BPM estimate, so they sit after both are available.
7. **Result struct:** `stratum-dsp/src/analysis/result.rs:185–232` — add fields:
   ```rust
   #[serde(skip_serializing_if = "Option::is_none")]
   pub sub_rumble_proportion: Option<f32>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub sidechain_depth: Option<f32>,
   ```
8. **Config:** `stratum-dsp/src/config.rs` — add `SubRumbleConfig { sub_band_hz: (f32, f32), kick_band_hz: (f32, f32), kick_window_ms: f32, kick_presence_floor: f32 }` and `SidechainConfig { mid_band_hz: (f32, f32), grid_stability_min: f32, method_disagreement_threshold: f32 }` with defaults `(30.0, 60.0), (60.0, 100.0), 50.0, 1e-4` and `(250.0, 2000.0), 0.4, 0.3` respectively.

### reklawdbox side

9. **`StratumResult`:** `src/audio.rs:30–48` — add `pub sub_rumble_proportion: Option<f64>` and `pub sidechain_depth: Option<f64>`.
10. **Schema version:** `src/audio.rs:60` — bump `STRATUM_SCHEMA_VERSION`. If the chord-stab plan also lands, coordinate the bump (single increment per release, not per feature). Auto-evicts cached results.
11. **Mapping:** `src/tools/classify_handler.rs:706` area — extract from stratum JSON, mirroring the `decay_mid_tau` pattern at `:698`.
12. **`AudioFeatures`:** `src/classify.rs:143` block — add `sub_rumble_proportion: Option<f64>` and `sidechain_depth: Option<f64>`. Thread through.
13. **Classification consumption:** see "Classification wiring" below — gated on validation.

---

## Validation (gates everything else)

The prior research (`genre-classification-improvements.md` § Empirically Invalidated Features) is unambiguous: features that look obvious from theory have failed empirical validation before — `mod_centroid`, `harmonic_proportion`, `bpm_confidence`, `grid_stability` all looked plausible and showed total overlap. **Both detectors ship and run against the validation set before being wired into classification.**

### Fixture set

A combined set serves both detectors. ~40 tracks total.

| Bucket | Target N | A4 expectation | A5 expectation |
|---|---|---|---|
| Deep Techno (Berghain template) | 8 | high (>0.5) | zero (<0.1) |
| Tech House | 8 | zero (<0.1) | high (>0.5) |
| Modern House | 4 | low–mid | mid–high (>0.3) |
| Deep House | 4 | mid (~0.3) | low–mid (~0.3) |
| Dub Techno | 4 | mid (chord stab is mid-band, sub is sustained but not always rumbling) | zero |
| Drone Techno (no kick) | 4 | None (kick_ref below floor) | None or zero |
| Electro (broken beat) | 4 | None (kick_alignment < 0.5) | zero (no sidechain) |
| Minimal | 4 | low | zero–low |

### Acceptance criteria

The detectors ship into classification only if **both pass**:

**A4 (sub_rumble_proportion):**
1. Deep Techno mean ≥ 2× Tech House mean.
2. No overlap at 90th percentile between Deep Techno (lowest 10 %) and Tech House (highest 90 %).
3. At least 6/8 Deep Techno tracks score above 0.4.
4. No more than 2/40 false positives (negative-bucket tracks above 0.4, excluding House family which is allowed mid).

**A5 (sidechain_depth):**
1. Tech House mean ≥ 2× Deep Techno mean.
2. No overlap at 90th percentile between Tech House (lowest 10 %) and Deep Techno (highest 90 %).
3. At least 6/8 Tech House tracks score above 0.4.
4. No more than 2/40 false positives.

**Joint criterion (the inverse-signature claim):**
5. For every track, `sub_rumble_proportion + sidechain_depth ≤ 1.2`. Both being high simultaneously would invalidate the underlying assumption (a track cannot have heavy rumble and heavy sidechain ducking at the same time). If this is violated, debug before shipping.

### Failure modes and recourse

- **A4 fails on band choice**: Try (40, 80) Hz for sub and (80, 120) Hz for kick. Berghain rumble lives lower than I'd assume.
- **A5 fails on band choice**: Try widening to (200, 4000) Hz; some Tech House sidechains the highs too. Or narrowing to (300, 1500) Hz to focus on the pad/lead band.
- **Method disagreement on A5**: Frequency-domain peak extraction is sensitive to envelope_sr / fft_size resolution. If `mod_freq_resolution` exceeds 0.3 Hz at typical track lengths, widen the peak search to ±2 bins.
- **A4 returns `None` for many tracks**: kick alignment threshold may be too strict. Lower from 0.5 to 0.3.

If only marginal failure: ship with a higher classification threshold (e.g. require `sub_rumble_proportion > 0.6` to set a flag, instead of > 0.4) and accept fewer positives but fewer false positives.

### Validation harness

```
stratum-dsp/tests/envelope_features_validation.rs (new, gated behind a feature flag
or env var since it's expensive and depends on local fixtures):

  - Iterate over fixture WAV files, decode, run analyze_audio,
    capture (sub_rumble_proportion, sidechain_depth).
  - Group by bucket from filename prefix or manifest TOML.
  - Print per-track scores, per-bucket mean/median/stddev.
  - Assert acceptance criteria, including the joint criterion (5).
  - Output: a markdown report committed alongside the implementation PR.
```

Offline, run-on-demand. Standard `cargo test` integration tests should not depend on it.

---

## Classification Wiring (post-validation)

Once validation passes, the features wire in at three layers, each independently testable:

1. **Tree-side flags:** Add to `CharFlag`:
   - `CharFlag::SubRumble` set when `audio.sub_rumble_proportion > 0.4`.
   - `CharFlag::Sidechain` set when `audio.sidechain_depth > 0.4`.
   Wire in `compute_audio_profile` at `src/classify.rs:280`.

2. **Same-family resolver:** In `resolve_same_family_specificity` at `src/classify.rs:914`:
   - `Sidechain` set + Techno-family votes → prefer Tech House over Deep Techno / Techno.
   - `SubRumble` set + Techno-family votes + `Atonal` (B1) → prefer Deep Techno over Techno.
   - `Sidechain` set + House-family votes + 4/4 kick → prefer Tech House over Deep House.

3. **Conjunctive templates** from the parent doc:
   - C1 (Deep Techno Berghain): incorporate `SubRumble` as a positive flag.
   - C5 (Tech House over Deep Techno): incorporate `Sidechain` (the parent doc already references A5 explicitly here).

Default to a moderate vote weight (similar to `AFFINITY_CAP = 0.5`) at first. After production proves it, raise to a stronger override, especially the combined `SubRumble & !Sidechain` and `Sidechain & !SubRumble` conjunctions, which are highly diagnostic precisely because the two signals are inverse.

---

## Risks and Open Questions

1. **Bin resolution at 30–60 Hz.** With frame_size=2048 and sr=44100, FFT bin width is ≈ 21.5 Hz. The 30–60 Hz sub-band is only 1–2 bins wide. This may be too coarse to separate sub-band from kick fundamental cleanly. Mitigation: if validation suffers, increase frame_size for `band_rms_envelope` only (recomputing a coarser STFT just for envelope extraction is cheap), or shift to a time-domain biquad filter as a one-off — but adding a time-domain filter sets a new precedent for the crate, so prefer the FFT-resolution route first.

2. **Beat-rate FFT resolution for A5.** At a 3-minute track with envelope_sr ≈ 86 Hz, fft_size ≈ 16384, resolution is ≈ 5.2 mHz — comfortably under the ±0.3 Hz wobble tolerance. At a 30-second clip resolution worsens. Most tracks are long enough that this is fine; flag short clips.

3. **Mid-band overlap with chord-stab band.** The Dub Techno chord-stab detector (planned A1) uses 200–2000 Hz, overlapping our sidechain mid band (250–2000 Hz). Conceptually distinct: chord-stab detects *transient onsets* in that band; sidechain detects *amplitude modulation* of the *steady-state* level. They should not interact, but if a Dub Techno track scores high on both `dub_stab_score` and `sidechain_depth` something is wrong. Add a cross-check in validation.

4. **Inverse-signature claim is empirical, not provable.** The argument that "a track cannot have heavy rumble and heavy sidechain at the same time" is theoretically defensible (sidechain ducks the lows by construction) but production techniques are creative. Validation criterion (5) checks this directly. If violated, the two features can still ship independently — but the conjunctive templates (C1 and C5) need rethinking.

5. **`mod_centroid` infrastructure refactor risk.** Pulling the envelope-FFT loop out of `modulation.rs` and into `envelope.rs` changes the call path of an existing, validated feature. Even though `mod_centroid` is invalidated as a *discriminator*, its numeric output is still cached and may be referenced in offline analyses. Refactor must be byte-identical in output (gated by a regression test that compares pre/post values on a fixture).

6. **Cost estimate.** Both detectors share the STFT and beat grid. Each adds: one band envelope (cheap), one pass over beats (cheap), one FFT for A5 (the same shape as `mod_centroid` does today, ~200 µs). Combined overhead estimate: 3–6 % of total analysis time. Acceptable, but worth measuring.

---

## Suggested Implementation Order

A single PR is too big. Mirror the chord-stab plan's PR breakdown:

1. **PR 1 — `envelope` module.** `band_rms_envelope`, `beat_windows`, `BeatWindow`. Pure primitives, full tests against synthetic input. Refactor `modulation.rs` to use `band_rms_envelope` (with regression test for bytewise-identical `mod_centroid`).
2. **PR 2 — `sub_rumble` module.** New module, full algorithm. Tests against synthetic input (kick-only signal → low; sustained sub + kick → high; no kick → None). No integration into `AnalysisResult` yet.
3. **PR 3 — `sidechain` module.** New module, primary + time-domain methods. Tests against synthetic input (constant mid → 0; AM-modulated mid at beat rate → high; tremolo at 8th rate → low).
4. **PR 4 — Result-struct integration + schema bump.** Wire into `AnalysisResult`, `StratumResult`, schema version. Re-analysis of cached tracks happens automatically.
5. **PR 5 — Validation harness and report.** Run against the fixture set, commit the report. **STOP HERE if validation fails for either feature.** If only one passes, ship that one alone.
6. **PR 6 — Classification wiring.** Only if PR 5 passes. Add `CharFlag::SubRumble` and `CharFlag::Sidechain` to `compute_audio_profile`, same-family resolver rules, conjunctive template integration.

Each PR is independently revertable. PR 6 is gated on PR 5's report.

## Cost Estimate

| Stage | Effort |
|---|---|
| PR 1 (envelope primitives + modulation refactor) | 1 day |
| PR 2 (sub_rumble) | 1 day |
| PR 3 (sidechain — both methods) | 1.5 days |
| PR 4 (result-struct integration) | 0.5 day |
| PR 5 (validation harness + run) | 1.5 days (fixture sourcing for both buckets) |
| PR 6 (classification wiring) | 0.5 day |
| **Total** | ~6 days |

Budget for 7–8 days realistically. Validation may surface band-choice or method-disagreement issues requiring iteration on PRs 2–3.
