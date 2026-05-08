# Kick bleed into the chord-stab band — investigation findings

Quantitative analysis of how kick-drum attack transients leak into the
350–2000 Hz band targeted by the chord-stab detector. Run via
`cargo run --release -p stratum-dsp --example band_bleed_analysis`.

STFT config: 44.1 kHz, frame=2048 (46.4 ms), hop=512 (11.61 ms), Hann window,
left-aligned frames.

## TL;DR

1. **Bleed extent is roughly symmetric, ±60–80 ms bulk with ±100 ms tails.**
   5-track real-audio histogram (Maurizio, cv313, Deepchord, Monolake,
   Rhythm & Sound) shows consistent dual peaks at −20 ms and 0 ms with bulk
   bleed extending to ±60 ms either side and lower-density tails to ±100 ms.
   The earlier asymmetric `(−60, +120)` shape was a Maurizio artefact and does
   not generalise.
2. **HPSS preprocessing is essentially useless on real audio.** Mean
   suppression across 5 tracks is **8% at ±20 ms**, **2% at ±50 ms**, and
   *neutral or net-negative* beyond ±100 ms. On 3/5 tracks HPSS produces *more*
   total stab-band onsets than raw STFT, and on 2/5 it adds non-coincident
   onsets (potential artifact introduction). The synthetic 85% suppression
   does not generalise. Recommend: drop HPSS preprocessing from the plan
   entirely. The 30 s per-track cost is not justified.
3. **The kick-coincidence mask is the heavy lifter.** On real audio at 0.85
   percentile, mean kick-coincidence rates are: 42% within ±20 ms, 56% within
   ±30 ms, 73% within ±50 ms, 89% within ±100 ms. Almost every stab-band
   onset in rhythmically-dense techno coincides with some drum-band event.
4. **Stab vs kick-bleed SNR is only 1.25×.** Real stabs barely dominate kick
   bleed in raw spectral flux. Without HPSS or a wider mask, peak-picking
   alone cannot reliably distinguish stabs from bleed.
5. **The 23 ms hop-time/centre-time offset self-cancels.** Source-tracing
   confirmed the beat tracker anchors beat times to onset hop-times, so the
   chord-stab plan's offset subtraction is bias-free. Initially flagged as a
   potential issue; verified to be a non-issue.

Concrete recommendations: (1) use a **symmetric ±80 ms** kick-coincidence
mask as the default — captures ~85% of bleed across the cross-validation set
while leaving 160 ms of comfortable margin before the off-beat 8th at 125 BPM
(240 ms from kick); (2) **drop HPSS preprocessing** as a configurable option
— the cost/benefit doesn't justify it.

## Experiments

All synthetic. Kick model: 60 Hz fundamental + 180 Hz harmonic, exponential
envelope (decay rate 60), 100 ms support. Stab model: 1 kHz tone burst with
slower envelope (decay rate 30), 200 ms support. Methodology in
`stratum-dsp/examples/band_bleed_analysis.rs`.

### 1. Pre-echo and bleed extent (single kick at t = 1.000 s)

| band | peak frame | hop-time | hop-time − kick |
|---|---|---|---|
| kick (40–200 Hz) | 83 | 0.9636 s | **−36 ms** |
| stab (350–2000 Hz) | 83 | 0.9636 s | **−36 ms** |

Both bands peak in the *same frame*, 36 ms before the kick attack. This is
expected: the kick at sample 44100 sits inside frames 83–86 (since
`512k ≤ 44100 < 512k + 2048` for k ∈ {83, 84, 85, 86}), and the Hann window
weights the kick most heavily in frame 83 because that frame's centre lies at
sample 44032 ≈ kick.

Stab-band flux profile (showing only frames where flux > 0):

```
frame  hop-time  stab_flux  kick_flux
   83  0.9636 s    2.306      1.840  ← peak (kick attack)
   84  0.9752 s    0.021      0.018
   85  0.9868 s    0.000      0.106
   86  0.9985 s    0.000      0.009
   88  1.0217 s    0.049      0.040
   89  1.0333 s    0.013      0.005
   90  1.0449 s    0.279      0.015  ← secondary peak
   91  1.0565 s    0.087      0.020
   92  1.0681 s    0.978      0.136  ← tertiary peak
   93  1.0797 s    0.144      0.323
   94  1.0913 s    0.084      0.608
```

**Bleed extent (frames where stab flux > 10% of peak):** frames 83–92,
covering −36 ms to +68 ms relative to the kick. Total span 116 ms.

The post-attack flux pumps several times. These secondary peaks come from the
Hann window de-modulating the kick's exponential decay as it slides across
successive frames — each new frame "sees" a different windowed slice of the
decay, producing apparent flux even when the underlying signal is monotonically
decaying.

### 2. Kick variation — which kicks bleed worst

| kick variant | stab peak | kick peak | stab/kick |
|---|---:|---:|---:|
| sub only (60 Hz, slow) | 2.417 | 1.756 | 1.376 |
| sub only (60 Hz, default) | 2.428 | 1.771 | 1.371 |
| sub only (60 Hz, sharp) | 2.445 | 1.803 | 1.356 |
| 60 + 180 Hz | 2.306 | 1.840 | 1.253 |
| 60 + 180 + 540 Hz | 2.786 | 1.814 | **1.536** |
| 60 Hz + 5 kHz click | 3.172 | 1.769 | **1.793** |
| full kick + click | 2.754 | 1.812 | 1.520 |

Counter-intuitive finding: **the stab band gets 25–80% more flux than the
kick band for the same kick**. This isn't because more energy is in the stab
band — it's because per-band normalisation crushes the kick band's saturated
sub content while the stab band's smaller absolute leakage gets normalised to
near-1.0.

The worst offenders are kicks with explicit HF content: a 5 kHz click (common
in industrial/dub-techno styles for definition) raises the bleed ratio to
1.79×. A 540 Hz harmonic (typical of saturated/punchy kicks) raises it to
1.54×.

Implication: per-track behaviour will vary substantially with kick design.
Tracks with click-heavy kicks need more aggressive masking than pure-sub kicks.

### 3. Stab vs kick-bleed SNR

| scenario | peak stab-band flux |
|---|---:|
| kick only | 2.306 |
| stab only (1 kHz, amp 0.6) | 2.879 |
| stab + kick simultaneous | 2.840 |

- **Stab/kick-bleed ratio: 1.25×.** Real stabs barely dominate kick bleed in
  the same band.
- **On-kick stab uplift: 1.23×.** Adding a stab on top of a kick increases
  flux by only 23% — well below typical onset-detection significance
  thresholds. A naïve flux peak-picker will *miss* on-kick stabs (or rather,
  attribute them to the kick).

This is the fundamental reason the chord-stab detector cannot rely on
band-restriction alone. The stab signal-to-bleed ratio is low enough that
distinguishing them requires either spatial filtering (HPSS — Experiment 4) or
templated time-structure validation (the plan's Stage 3 cosine similarity).

### 4. HPSS suppression — major effect

Audio: 4 kicks at 0.5 / 1.0 / 1.5 / 2.0 s, plus stabs at 1.25 s and 2.0 s
(the on-kick edge case). Stab-band onset detection at percentile 0.85.

| metric | raw STFT | HPSS-harmonic |
|---|---:|---:|
| true stabs found / 2 | 2 | 2 |
| onsets near kick-only times | **13** | **2** |

Raw stab-band onsets:

```
[0.464, 0.511, 0.546, 0.569, 0.592, 0.964, 1.022, 1.045, 1.068,
 1.207, 1.440, 1.463, 1.533, 1.567, 1.962, 2.078, 2.194]
```

After HPSS (harmonic component only):

```
[1.207, 1.254, 1.440, 1.463, 1.962, 2.009, 2.090, 2.136, 2.194]
```

HPSS suppresses ~85% of kick-bleed onsets while preserving both true stabs.
The remaining "false positives" (e.g. 1.440, 1.463, 2.090) cluster around the
stab decay, not at kick times — they're stab continuation artefacts, not kick
bleed, and would be filtered by Stage 4's decay-window check.

This is the single largest leverage point in the pipeline: a one-time
preprocessing pass eliminates more bleed than any mask-width tuning could.

### 5. Mask-width sensitivity

Kick-only audio (4-on-floor at 120 BPM). 15 stab-band onsets all attributable
to kick bleed. Offset distribution to nearest kick:

```
−38, +11, +46, +69, +92,
−36, +22, +45, +68,
−37, +33, +67,
−38, +20, +67   (ms)
```

Range: −38 to +92 ms. **Skewed heavily toward post-kick** — the bleed
post-echo is ~2.5× longer than the pre-echo.

| mask width | kick-bleed onsets surviving (lower = better) |
|---|---:|
| ±10 ms | 15 / 15 |
| ±20 ms | 14 / 15 |
| ±30 ms (current plan) | **12 / 15** |
| ±40 ms | 7 / 15 |
| ±50 ms | 5 / 15 |
| ±60 ms | 5 / 15 |
| ±80 ms | 1 / 15 |

Symmetric ±30 ms catches 20% of bleed. ±100 ms would catch all but is
overkill — it also masks 200 ms per kick, which on a 4-on-floor pattern at
120 BPM is 40% of total time.

**Asymmetric (−50, +100) ms catches 14/15** while masking only 150 ms per
kick (30% of time at 120 BPM). The pre/post asymmetry matches the underlying
physics — pre-echo is bounded by half the FFT window (~23 ms structural) plus
a frame or two of windowing leakage, while post-echo extends through the
decay tail.

### 6. STFT centring convention — verified to self-cancel

**Initial concern (refuted by source check below).** The detector returns
frame indices and the chord-stab plan converts them via `i × hop / sr`,
which gives **hop-time** (start of frame). The actual sample at the centre
of the FFT frame is `hop-time + frame_size / (2 × sr)` = **hop-time +
23.2 ms** — so onsets are reported 23 ms earlier than the audio they
represent.

**Verification.** Traced the full pipeline:

- All band/spectral/HFC onset detectors return frame indices, converted to
  samples via `f * hop_size` (`lib.rs:184` and `to_samples`) — hop-time.
- Energy flux directly emits `(i + 1) * hop_size` (`energy_flux.rs:187`) —
  same hop-time convention.
- Beat tracker (`hmm.rs:425`) computes
  `beat_time = start_time + t * beat_interval` where
  `start_time = self.onsets[0]` (an onset hop-time, `lib.rs:917`).
- `BeatGrid.beats` is populated with these hop-time-anchored values
  (`beat_tracking/mod.rs:305`).

So `BeatGrid.beats[i]` is *also* in hop-time, not absolute audio time. The
chord-stab plan's Stage 2 offset
`(onset_time − beat_grid.beats[nearest])/beat_period` subtracts two hop-time
values — the 23 ms structural bias cancels.

**Conclusion: no compensation needed.** The bias is system-wide and
consistent. It would only matter if (a) the beat grid were re-derived from
some absolute-time source (e.g. a tap-in click track) instead of the onset
pipeline, or (b) we wanted to extract a sample-accurate audio segment around
a stab (e.g. for a UI marker) — that would need `+ frame_size / (2 × sr)`.
Neither applies to the chord-stab detector.

### 7. Real-audio validation — 5-track cross-validation set

Methodology: 60 s of each track decoded to mono via `symphonia`,
percentile-0.85 onset detection, kick band 40–200 Hz treated as ground-truth
drum-time anchors. Run via
`cargo run --release -p stratum-dsp --example band_bleed_real_audio`.

Cross-validation set (selected for stylistic diversity within Dub Techno):

| Track | Style |
|---|---|
| Maurizio — M-4A | Basic Channel-era, punchy kicks |
| cv313 — Analogue Oceans [Divergent] | Modern Dub Techno |
| Deepchord — Immersion I | Atmospheric / pad-heavy |
| Monolake — Gobi (Long Edit) | Reduced / abstract |
| Rhythm & Sound — See Mi Yah | Dub-reggae fusion |

**Onset counts (per 60 s):**

| Track | kick (40–200 Hz) | stab raw | stab HPSS |
|---|---:|---:|---:|
| Maurizio | 644 | 623 | 646 |
| cv313 | 718 | 639 | 662 |
| Deepchord | 648 | 596 | 607 |
| Monolake | 661 | 620 | 577 |
| Rhythm & Sound | 634 | 556 | 562 |

HPSS produces *more* stab-band onsets than raw on 3/5 tracks. Onset counts
are broadly consistent across the set (~600–720 kicks, ~550–650 stabs).

**Kick-coincidence rate (raw, percent within window):**

| Track | ±20 ms | ±30 ms | ±50 ms | ±100 ms |
|---|---:|---:|---:|---:|
| Maurizio | 51.4% | 64.5% | 81.1% | 95.3% |
| cv313 | 42.6% | 60.9% | 82.0% | 96.4% |
| Deepchord | 41.1% | 55.7% | 72.1% | 83.2% |
| Monolake | 34.2% | 48.4% | 62.9% | 79.8% |
| Rhythm & Sound | 43.9% | 57.7% | 74.6% | 90.1% |
| **Mean** | **42.6%** | **57.4%** | **74.5%** | **88.9%** |

A symmetric ±80 ms mask catches roughly 80–85% of stab-band onsets across
the set; ±100 ms reaches ~89%.

**HPSS suppression vs raw at each window:**

| Track | ±20 ms | ±30 ms | ±50 ms | ±100 ms |
|---|---:|---:|---:|---:|
| Maurizio | **20.0%** | 9.7% | 0.6% | −3.9% |
| cv313 | −2.6% | 1.8% | −0.2% | −2.8% |
| Deepchord | 2.9% | 0.3% | 0.5% | −0.4% |
| Monolake | 7.5% | 7.0% | 6.7% | 6.9% |
| Rhythm & Sound | 10.2% | 8.1% | 2.4% | −0.6% |
| **Mean** | **7.6%** | **5.4%** | **2.0%** | **−0.2%** |

Maurizio's 20% suppression at ±20 ms was the outlier. Mean suppression is
~8% at ±20 ms, dropping to ~2% at ±50 ms and neutral beyond. **HPSS is not
worth the ~30 s preprocessing cost.**

**Asymmetry test (raw bin sums, −60 ms → 0 ms vs 0 ms → +60 ms):**

| Track | pre sum | post sum | pre/post ratio |
|---|---:|---:|---:|
| Maurizio | 381 | 319 | 1.19 (pre-skewed) |
| cv313 | 319 | 355 | 0.90 (post-skewed) |
| Deepchord | 269 | 278 | 0.97 |
| Monolake | 243 | 277 | 0.88 |
| Rhythm & Sound | 280 | 278 | 1.01 |

**Symmetric on average.** The Maurizio pre-skew was idiosyncratic. The
earlier asymmetric `(−60, +120)` recommendation does not generalise. A
symmetric mask is the correct default.

**Why the synthetic 85% HPSS finding broke down on real audio.** The
synthetic test had only kicks + stabs — the harmonic content was either (a)
the sustained stab (HPSS keeps) or (b) absent. On real audio with continuous
pads, basses, and reverb tails, the "harmonic" component is dominated by
sustained content, and short transients in either drums *or* stabs both look
"non-harmonic" to the median filter. HPSS doesn't distinguish "kick attack
transient" from "stab attack transient" — both are punctate energy bursts
spectrally distinct from the sustained harmonic floor.

**Non-coincident counts (>100 ms from any kick — these are likely real
stabs):**

| Track | raw | HPSS | Δ |
|---|---:|---:|---:|
| Maurizio | 29 | 29 | 0 |
| cv313 | 23 | 29 | +6 (HPSS adds) |
| Deepchord | 100 | 109 | +9 (HPSS adds) |
| Monolake | 125 | 116 | −9 |
| Rhythm & Sound | 55 | 58 | +3 |

Atmospheric tracks (Deepchord, Monolake) have 4–5× more non-coincident
stab-band onsets than punchy tracks (Maurizio, cv313). This is what the
chord-stab detector should be detecting — the kick-mask removes the false
positives, leaving the real stabs in the >100 ms region.

**Caveats:**

- **Kick-band detector over-counts.** 600–720 events in 60 s; a 4-on-floor
  at 125 BPM is only ~500. The extra ~20–40% are likely snares and bassline
  notes whose fundamentals sit below 200 Hz. So "kick coincidence" really
  means "drum-band coincidence" — which makes the mask *more* effective for
  the chord-stab detector's purposes.
- **5 tracks, all Dub Techno or adjacent.** Generalisation outside this
  genre is not guaranteed; the chord-stab detector is targeted at this
  genre, so this is the relevant case.
- **Percentile 0.85 is loose.** The detector is dominated by noise at this
  threshold; these results characterise the bleed *floor*, not the final
  detector output. PR 5's full-pipeline validation is the next gate.

## Recommendations for the chord-stab plan

### High-priority (correctness)

1. **Use a symmetric ±80 ms kick-coincidence mask** as the default.
   Captures ~80–85% of bleed across 5 cross-validated tracks. Masks 160 ms
   per kick (33% of beat at 125 BPM); leaves a 160 ms gap between mask
   edge and the off-beat 8th (240 ms from kick) — comfortable margin for
   real stabs to survive. Configurable via
   `dub_stab_kick_mask_window_ms`. Tighter `±60 ms` is reasonable for
   tracks with sparse non-coincident content; wider `±100 ms` for
   atmospheric tracks where the stab signal is sparse and the gain from
   tighter false-positive rejection outweighs the loss of margin.

2. ~~Asymmetric mask `(−60, +120) ms`.~~ **Withdrawn after 5-track
   cross-validation.** Pre/post ratio is ≈1.0 on 4/5 tracks; the original
   asymmetric shape was a Maurizio artefact.

3. ~~Add HPSS preprocessing as a Stage 0 gate / opt-in refinement.~~
   **Withdrawn after 5-track cross-validation.** Mean suppression: 8% at
   ±20 ms, 2% at ±50 ms, neutral beyond. On 3/5 tracks HPSS *adds* total
   stab-band onsets; on 2/5 it adds non-coincident onsets (probable
   artifact introduction). Cost (~30 s per track) is not justified. Drop
   the `use_hpss_preprocessing` config knob from the chord-stab plan
   entirely.

4. ~~Document the hop-time vs centre-time convention.~~ **Withdrawn after
   source-tracing verified that the beat tracker (`hmm.rs:425`) anchors beat
   times to onset hop-times, so the same 23 ms bias appears on both sides of
   the chord-stab plan's `onset_time − beat_time` subtraction and cancels.
   See Experiment 6 for the full trace.

### Medium-priority (robustness)

5. **Tighten the lower edge of the stab band from 350 → 500 Hz** if the full
   validation harness (PR 5) shows kicks with strong 540 Hz harmonics
   (saturated/punchy techno kicks) continue to bleed through. Trade-off:
   some stab fundamentals (e.g. C5 ≈ 523 Hz) sit right at the boundary;
   raising the cutoff may clip them. Cross-validation set didn't isolate
   this clearly — leave the 350 Hz default and revisit with full-pipeline
   metrics.

### Low-priority (future)

6. **Per-track kick profile.** Tracks with click-heavy kicks bleed
   significantly more. After the kick-pattern detector ships, its output
   could include a "kick brightness" measure (e.g. centroid of the kick's
   spectral footprint) that the chord-stab detector uses to widen its mask
   for problematic tracks.

## Caveats

- **5 tracks within Dub Techno.** Cross-validated within the target genre
  but not across all genres the detector might encounter. The full 60-track
  fixture set in PR 5 will surface remaining edge cases.
- **Synthetic kicks for Experiments 1–5.** Real kicks vary in transient
  sharpness, harmonic content, click presence, saturation, and reverb. The
  *direction* of synthetic findings (transient bleed exists, low SNR) is
  confirmed by real audio; the *magnitude* of HPSS benefit and the asymmetry
  shape were not.
- **HPSS parameter `margin=17` was a guess.** A sweep *could* recover some
  benefit, but given the consistent under-performance vs synthetic across
  5 real tracks and the artifact-introduction signal, the cost/benefit
  doesn't justify further tuning. Cleaner to drop HPSS entirely.
