# Chord-Stab Detector: Implementation Plan

**Date:** 2026-04-26 (revised 2026-04-27 with review-driven fixes)
**Status:** Design proposal. No implementation yet. Validation strategy included; **wire-into-classification gated on validation pass**.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) — this is feature A1 from that doc, the "single biggest gap" in audio-based discrimination of Dub Techno from its neighbours.

## Goal

Add a stratum-dsp feature that detects the periodic mid-band chord stab/wash pattern characteristic of Dub Techno. Output `dub_stab_score: f32 ∈ [0, 1]` and `dub_stab_period: Option<u8>` (period in 16th notes). Cost target: under 5% of total analysis time, reusing the existing shared STFT.

This is the single discriminator that genuinely cannot be derived from any currently-cached feature. It's what cleanly separates Dub Techno from Deep Techno and Drone Techno — three sonically and DJ-context distinct genres that the current pipeline frequently confuses.

## Signal Definition

A chord stab in Dub Techno has six characteristics. The detector must check all six, conjunctively:

1. **Frequency band**: energy concentrated in ~350–2000 Hz (chord fundamentals + low partials). Lower bound raised from the original 200 Hz draft because techno kicks produce strong spectral flux up to ~1.5 kHz via their click/punch transient — a 200 Hz lower bound puts kicks squarely on top of stabs and corrupts Stage 1's onset list. 350 Hz sits above the kick body. If validation shows missed positives, the bound is configurable.
2. **Onset envelope**: short attack (under 30 ms), decay tail of 100–600 ms. Distinguishable from sustained pads (>1 s) and from hats/claps (<50 ms).
3. **Periodicity**: regular, locked to the beat grid. The dominant pattern is one of:
   - Every off-beat 8th note (4 stabs/bar, the "every-and" pattern)
   - Off-beat 8ths only on certain beats (1 or 2 stabs/bar)
   - Less commonly: every 16th-note off-beat (8 stabs/bar)
4. **Phase alignment to beats**: stabs sit on off-beats (between beats), *not* on beats themselves. This separates them from kicks and snare backbeats.
5. **Persistence**: present across most of the track (≥60% of bars), not a brief intro flourish.
6. **Spectral character**: harmonic (multi-pitch) rather than noise-like. The stab is a chord, not a hat.

Negative classes the detector must reject:
- Off-beat hi-hats (wrong frequency band — fail #1)
- Snare backbeats (on-beat, not off-beat — fail #4)
- Sustained pads (no transient onsets — fail #2)
- Lead synths (irregular timing — fail #3 and/or #5)
- Tech House sidechain pumping (creates mid-band amplitude modulation but no transient onset — fail #2)
- Bassline plucks (wrong frequency band — fail #1)

## Algorithm

A four-stage pipeline. Each stage outputs a partial signal; the final score is a multiplicative combination, so all four must be present for a high score.

### Stage 1 — Mid-band onset detection (with kick-coincidence masking)

Reuses the existing shared STFT (`stratum-dsp/src/lib.rs:166`, computed once for all detectors).

```
For each STFT frame, compute spectral flux limited to bins in 350–2000 Hz:
  flux_mid[t] = Σ_{bins ∈ band} max(0, |X[t, bin]| − |X[t-1, bin]|)

Apply percentile thresholding (matching existing onset detectors at
`features/onset/spectral_flux.rs:69`, configurable via `onset_threshold_percentile`).

Then mask kick-coincident candidates: drop any candidate onset whose frame falls
within ±30 ms of a kick-band onset (40–120 Hz, computed via the same primitive).
This removes the dominant Stage-1 false-positive class — kick punch transients
that bleed into the mid band — without depending on Stage 2's geometry.

Output: list of kick-disjoint candidate onset frame indices.
```

This is a sibling of the existing `detect_spectral_flux_onsets` at `features/onset/spectral_flux.rs:69`, parameterised to a frequency band rather than the full spectrum. Cleanest to add a new function `detect_band_onsets(fft_magnitudes, sample_rate, frame_size, band_hz, threshold)` and have the new detector call it with `band_hz = (350.0, 2000.0)`.

The kick-masking step calls the same primitive a second time with `band_hz = (40.0, 120.0)`, then sets-differences within the ±30 ms tolerance. If the kick-pattern detector (sibling plan) has already run and cached its onsets, reuse that result rather than recomputing.

Cost: O(n_frames × bins_in_band) over the existing STFT — single-digit-percent overhead, doubled by the second band-onset pass for kick masking. Still well within the 5–10% budget.

### Stage 2 — Beat-relative offset histogram (soft-binned, per-bar)

Two changes from the original draft: **soft binning** (to handle realistic micro-timing drift) and **per-bar sub-histograms** (to make persistence well-defined).

```
For each candidate onset:
  onset_time = frame_idx × hop_size / sample_rate    // in seconds
  nearest_beat = binary_search(beat_grid.beats, onset_time)
  beat_period = beat_grid.beats[nearest_beat + 1] − beat_grid.beats[nearest_beat]
  offset = (onset_time − beat_grid.beats[nearest_beat]) / beat_period
  // offset ∈ [0.0, 1.0), where 0.0 = on beat, 0.5 = off-beat 8th, 0.25/0.75 = 16ths

  // Soft binning: 32-bin histogram with Gaussian smoothing (σ = 0.5 bin)
  // around the continuous offset, instead of hard rounding to a 16-bin index.
  // This avoids snapping a genuine offset of 0.47 (realistic micro-timing) to
  // the wrong bin in a sparse template.
  for bin in 0..32:
    weight = gaussian(bin / 32.0 − offset, σ = 1.0/64.0)  // wraps around [0,1)
    histogram_global[bin] += weight
    bar_idx = locate_bar(beat_grid.bars, onset_time)
    histogram_per_bar[bar_idx][bin] += weight
```

Discards onsets outside the beat grid (before the first detected beat or after the last). Beat grid format from the existing `BeatGrid { downbeats: Vec<f32>, beats: Vec<f32>, bars: Vec<f32> }` at `stratum-dsp/src/analysis/result.rs:143-154`; all three are in seconds.

Output:
1. A 32-bin global histogram of beat-relative onset positions across the track (used by Stage 3 template scoring).
2. A 32-bin sub-histogram **per bar** (used by the persistence calculation in the final score). The per-bar sub-histograms enable a meaningful "bars-with-stabs / total-bars" ratio rather than the original draft's hand-waved global-derived persistence.

### Stage 3 — Pattern matching (v1: T1+T2 only)

Score the global histogram against two templates representing the dominant Dub Techno stab patterns:

| Template | Period (16ths) | Histogram peaks at (32-bin scale) |
|---|---|---|
| T1: every off-beat 8th | 8  | bin 16 only (mid-beat)        |
| T2: every off-beat 16th | 4 | bins 8, 16, 24                |

**T3 (half-bar) and T4 (bar-level) deferred from v1.** Both require bar-relative re-bucketing that adds non-trivial pipeline complexity, and Dub Techno is dominated by T1 + T2 in the user's listening canon. Add later as a strict extension if validation surfaces specific misses.

**On-beat masking before scoring.** Real Dub Techno has strong on-beat kicks coexisting with off-beat stabs, so the original draft's "if peak at bin 0 return 0" guard is wrong — it vetoes textbook positives. Instead, *zero out* the on-beat region (bins 0±1 and 16th-bin neighbours of the implicit kick/snare positions) before scoring. The histogram then represents only the off-beat content, and template correlation reflects stab presence honestly.

```
masked_histogram = histogram with bins {0, 1, 31} and {15, 16, 17} kept,
                   bin {0, 1, 31} (on-beat region) zeroed out for T1/T2 scoring.
                   // Bin 16 IS the off-beat 8th and stays.
```

Score (cosine similarity on z-normalised histograms — *not* Pearson correlation, which is dominated by absolute energy on sparse templates):

```
For each template t in {T1, T2}:
  template_score[t] = cosine_similarity(z_normalize(masked_histogram),
                                         template_pattern[t])
                    × concentration[t]

  where concentration[t] = energy_in_template_bins / energy_in_masked_histogram
                          // Direct likelihood ratio: did off-beat energy
                          // actually land at the template's expected bins?
                          // This is what the original draft's leakage_penalty
                          // was gesturing at; promote it to the primary
                          // measure, not a multiplier on a flawed metric.
```

The dominant template wins. If no template scores above 0.4 (configurable), return `dub_stab_score = 0`.

### Stage 4 — Decay-tail confirmation

Reuses primitives from `features/decay.rs` (`post_transient_decay` at line 45).

```
For each onset matching the dominant template AND not within the decay window
of a previous matching onset (avoids fitting overlapping decays — the next stab
arriving before the previous fully decays biases tau low):
  measure post-onset spectral decay in the 350–2000 Hz band over a 1-second window
  fit exponential decay → tau_ms

Aggregate tau across isolated matching onsets in LOG SPACE:
  log_taus = [ln(tau_ms[i]) for i in isolated_matches]
  median_log_tau = median(log_taus)
  avg_tau_geom = exp(median_log_tau)
  // Median in log-space is robust to heavy-tailed tau distributions and to
  // bad single-onset fits. Original draft's mean(tau) was dominated by outliers.

If avg_tau_geom < 50 ms:    likely hi-hats/transients, not chord stabs   → return 0
If avg_tau_geom > 3000 ms:  likely sustained pad, not a stab              → return 0
                            // Upper bound raised from 1500 ms after review:
                            // canonical reverb-soaked Dub Techno (Basic
                            // Channel, Echocord) has stab tails that ARE the
                            // reverb. Validation set must include 3+ such
                            // canonical positives to lock the cap empirically.
Otherwise:             compute decay_match_score                          → use as multiplier

decay_match_score = exp(−|log(avg_tau_geom) − log(400)|² / σ²)
                    // peaks at avg_tau_geom = 400 ms (typical chord stab decay
                    // including some reverb contribution).
                    // σ chosen so the score is ~0.5 at 150 ms and 1200 ms.
```

Cost: small — `post_transient_decay` is already cheap, and we run it on a subset of onsets only (those matching the template) with a min-separation filter.

### Final score

Persistence is now well-defined via the per-bar sub-histograms from Stage 2:

```
For each bar b:
  bar_score[b] = score Stage 3 against histogram_per_bar[b]
                 (using the same masked, z-normalised cosine similarity)
  bar_matches[b] = (bar_score[b] >= 0.4)

persistence_raw = count(bar_matches) / total_bars

// Soft floor to avoid the knife-edge: a real positive with persistence_raw=0.3
// (a track where the stab pattern is established for the first 30% of bars and
// breaks down for the rest) shouldn't have the whole score collapse multiplicatively.
persistence = max(persistence_raw, 0.3)
              // Below 0.3, the floor kicks in; above 0.3, persistence reads true.
              // The 0.3 floor itself caps the lower-bound contribution; the
              // template_score gate at 0.4 (Stage 3) prevents pure-noise tracks
              // from sneaking through via the floor alone.

template_score = best of stage 3 (on the global histogram)
decay_match = stage 4

dub_stab_score = template_score × persistence × decay_match

dub_stab_period = period of the winning template in 16ths
                  // None if dub_stab_score < 0.2 (configurable noise floor)
```

Multiplicative combination ensures all conditions matter. A track with off-beat onsets but wrong decay (hats) scores low. A track with right-decay events but no off-beat alignment (pads with random clicks) scores low. The persistence soft-floor avoids the original draft's knife-edge where any single sub-factor near 0 collapsed the whole score.

**Stereo note:** v1 sums to mono before STFT (existing pipeline behaviour). Heavy ping-pong delay common in Dub Techno can smear the temporal pattern across channels; if validation shows specific ping-pong-heavy tracks scoring low, follow-up work can run the detector per-channel and OR the results. Logged as Risk #7 below.

## Integration Points

### stratum-dsp side

1. **New module:** `stratum-dsp/src/features/dub_stab.rs` (~300 LOC estimated). Contains:
   - `pub fn detect_dub_stab(stft, beat_grid, sample_rate, config) -> DubStabResult`
   - `DubStabResult { score: f32, period: Option<u8>, template: DubStabTemplate, debug: DubStabDebug }`
   - Helpers: `detect_band_onsets`, `compute_beat_offset_histogram`, `score_templates`, `confirm_decay_tail`
2. **Module registration:** `stratum-dsp/src/features/mod.rs` — add `pub mod dub_stab;`.
3. **Pipeline wiring:** `stratum-dsp/src/lib.rs:1593–1620` (alongside `mod_centroid`, `harmonic_proportion`, `decay`). Call `detect_dub_stab(&stft, &beat_grid, sample_rate, &config)` and store result.
4. **Result struct:** `stratum-dsp/src/analysis/result.rs:185–232` — add fields:
   ```rust
   #[serde(skip_serializing_if = "Option::is_none")]
   pub dub_stab_score: Option<f32>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub dub_stab_period: Option<u8>,
   ```
5. **Config:** `stratum-dsp/src/config.rs` — `AnalysisConfig` is monolithic (no per-feature struct precedent in this codebase). Add fields directly:
   ```rust
   pub dub_stab_band_low_hz: f32,           // 350.0
   pub dub_stab_band_high_hz: f32,          // 2000.0
   pub dub_stab_kick_band_low_hz: f32,      // 40.0  (for kick-coincidence masking)
   pub dub_stab_kick_band_high_hz: f32,     // 120.0
   pub dub_stab_kick_mask_window_ms: f32,   // 30.0
   pub dub_stab_template_threshold: f32,    // 0.4
   pub dub_stab_decay_min_ms: f32,          // 50.0
   pub dub_stab_decay_max_ms: f32,          // 3000.0  (raised from draft's 1500)
   pub dub_stab_decay_optimum_ms: f32,      // 400.0   (raised from draft's 250)
   pub dub_stab_persistence_floor: f32,     // 0.3
   ```
6. **Public API:** `pub use features::dub_stab::DubStabResult;` in `lib.rs` if downstream code wants the full result; otherwise the `Option<f32>` on `AnalysisResult` is enough.

### reklawdbox side

7. **`StratumResult`:** `src/audio.rs:30–48` — add `pub dub_stab_score: Option<f32>` and `pub dub_stab_period: Option<u8>`.
8. **Schema version:** `src/audio.rs:60` — bump `STRATUM_SCHEMA_VERSION` from `"4"` to `"5"`. This auto-evicts cached results without the new field on next load.
9. **Mapping:** `src/tools/classify_handler.rs:706` — extract from stratum JSON.
10. **`AudioFeatures`:** `src/classify.rs:117` (struct definition; line 143 in the original draft was one specific field, not the struct) — add fields, thread through.
11. **Classification consumption:** see "Classification wiring" below — gated on validation.

## Validation (gates everything else)

The prior research doc (`genre-classification-improvements.md`) has a clear precedent: features that look obvious from theory have failed empirical validation before. `harmonic_proportion`, `mod_centroid`, `bpm_confidence`, `grid_stability` all looked like they should work and showed total overlap across genres in 17 ear-verified tracks. **The detector ships and runs against the validation set before being wired into classification.**

### Fixture set

Build a small ear-verified WAV fixture set for offline validation. Stored separately from the integration test fixtures (which test correctness of detection); these test discriminative power.

| Bucket | Target N | Examples |
|---|---|---|
| Dub Techno (positive) | 10 | Basic Channel, Vladislav Delay, Echocord roster, Deepchord, Convextion, Burger/Ink "Elvism" (already a known case study). **Must include ≥3 canonical reverb-soaked positives (Basic Channel / Echocord) to lock the decay upper bound at 3000 ms.** |
| Deep Techno (negative) | 10 | Marcel Dettmann era, Klockworks, Norman Nodge, Sandwell District, Prologue |
| Drone Techno (negative) | 10 | Voices From The Lake, Donato Dozzy reductive, Spazio Disponibile |
| Tech House (negative) | 10 | Hot Creations roster, Solid Grooves, Cuttin' Headz |
| Off-beat hi-hat tracks (negative) | 10 | Any House track with prominent off-beat hats but no chord stab |
| Sustained-pad tracks (negative) | 10 | Ambient Techno, Future Sound of London, anything with long pads |

Total: ~60 tracks. N=10 per bucket is the minimum for honestly estimating a 90th-percentile statistic; N=4 (original draft) only gives a max. If sourcing falls short on a bucket, validation can pass/fail on the buckets that *do* meet N — but **do not** ship classification wiring against undertested buckets.

### Validation acceptance criteria

The detector ships into classification only if:

1. **Dub Techno median score ≥ 2× the next-highest bucket's median score** (separation criterion). Median rather than mean — within-Dub-Techno variance is high (Basic Channel ≠ Convextion) and means get pulled around by outliers.
2. **No overlap at the 90th percentile** (primary criterion): the lowest 10th-percentile Dub Techno score must exceed the highest 90th-percentile score from any negative bucket. With N=10 per bucket this is honest; with the original draft's N=4 it was unestimable.
3. **At least 7/10 Dub Techno tracks score above 0.5**, and at least 9/10 score above 0.3.
4. **Max-based false-positive criterion** (replaces the original draft's "no more than 2/32" rate, since the rate was meaningless at N=4 per bucket): **no negative-bucket track scores above 0.6**, and no more than 3/50 negatives score above 0.4.

Failure modes and recourse:
- If criterion (1) fails by more than 50% — fundamental algorithm issue. Iterate on the band choice, template set, or decay window.
- If criterion (2) fails — likely a band-choice or template-set issue. Try widening to 150–2500 Hz or adding more templates.
- If criterion (3) passes but (4) fails — probably an issue with one specific negative class (e.g. Tech House). Add a guard or veto for that class.
- If only marginal failure — ship with a higher threshold (e.g. require score > 0.7 instead of > 0.5 to count as positive) and accept more false negatives.

### Validation harness

```
stratum-dsp/tests/dub_stab_validation.rs (new, gated behind a feature flag or env var
since it's expensive and depends on local fixtures):

  - Iterate over fixture WAV files, decode, run analyze_audio, capture dub_stab_score.
  - Group by bucket (from filename prefix or a manifest TOML).
  - Print per-track scores, per-bucket mean/median/stddev.
  - Assert acceptance criteria.
  - Output: a markdown report committed alongside the implementation PR.
```

This is offline, run-on-demand. The standard `cargo test` integration tests should not depend on it.

## Classification Wiring (post-validation)

Once validation passes, the feature wires in at three layers, each independently testable:

1. **Tree-side flag:** Add `CharFlag::StabHeavy` set when `audio.dub_stab_score > 0.5`. (Naming: existing `CharFlag` variants are all adjectives — `Ambient`, `Atmospheric`, `Broken`, `Irregular`, `Fast`, `Slow` — so `StabHeavy` keeps the pattern. The original draft's `CharFlag::ChordStab` was a noun and would have broken naming consistency.) Wire in `compute_audio_profile` at `src/classify.rs:280`.
2. **Same-family resolver:** In `resolve_same_family_specificity` at `src/classify.rs:914`, if `StabHeavy` is set and the family is Techno-family, prefer Dub Techno over Deep Techno / Techno regardless of energy bucket.
3. **Conjunctive template C2** from the parent ideas doc — incorporate into a Deep Techno→Dub Techno override.

Default to a moderate vote weight (similar to `AFFINITY_CAP = 0.5` defined in `src/audio_profile.rs:20`, *not* in `classify.rs` as the original draft claimed) to start, since this is a single-feature signal. After it proves itself in production, raise to a stronger override.

## Risks and Open Questions

1. **Band-choice sensitivity.** Pilot listens suggest 200–2000 Hz, but some atmospheric Dub Techno (Deepchord territory) has stabs reaching 3 kHz. The validation set will tell us if the default band misses too many positives. Mitigation: per-band-choice config so it can be tuned without code changes.

2. **Tech House sidechain pumping.** Sidechain creates mid-band amplitude *modulation* but not transient *onsets* — the spectral flux step should reject it. But if any Tech House tracks slip through, the stab-vs-pump discrimination requires the separate `sidechain_depth` feature (A5 in the parent doc). Validation will surface this.

3. **Half-time vs double-time confusion.** If the BPM detector reports half-time (e.g. 62 instead of 125), the beat grid is half as dense and "off-beat 8ths" become "on-beat quarters." Existing BPM detection is robust here per the metrical-agreement logic at `lib.rs:819–892`, but worth checking against fixtures with known halftime-ambiguity.

4. **Tracks with unreliable beat grids.** If the beat grid is unreliable, beat-relative offset histograms are noise. **Do not gate on `grid_stability`** — that field is on the empirically-invalidated list. Use structural gates instead: short-circuit and return `None` for `dub_stab_score` if `beat_grid.beats.len() < 8` (fewer than 2 bars detected) or if `total_bars < 4`. This matches the gating choice in the kick-pattern detector plan and avoids reintroducing an invalid metric. Better than reporting a confidently-wrong number.

5. **Are the v1 templates exhaustive?** v1 ships with T1 + T2 only. Idiosyncratic patterns (Vladislav Delay 7-beat phrases, certain Convextion bar-level singles) are deliberately out of scope. T1 + T2 captures the dominant canon. Mitigation: a low-weight "any off-beat presence" fallback can be added if validation surfaces specific misses — and T3/T4 can be added later as a strict extension once the per-bar histogram pipeline is in place.

6. **Cost estimate accuracy.** Original claim was <5%. With the kick-masking second band-onset pass added, plus per-bar sub-histogram scoring, the realistic budget is **8–12%**. Acceptable but worth measuring; not a blocker.

7. **Stereo ping-pong delay.** Heavy stereo delay (signature Dub Techno) can smear the temporal pattern when the pipeline sums to mono before STFT. v1 accepts this; if validation surfaces specific affected tracks, follow-up work runs the detector per-channel and ORs the matched-onset sets.

8. **Beat-grid micro-drift over long tracks.** A 0.5% tempo drift across a 7-minute track shifts late onsets by half a bin (in 32-bin terms). Soft binning (Stage 2) absorbs this gracefully — a single onset's weight spreads over neighbouring bins — but worth verifying against long-form fixtures.

9. **Half-time confusion (symmetric case).** Risk #3 above covers half-tempo BPM detection. The symmetric case — a 132 BPM track classified as 66 — produces an apparent "8 stabs/bar" pattern that T2 already covers. The dominant template would still match, just labeled with the wrong period. Acceptable.

## Suggested Implementation Order

A single PR is too big. Break it up:

1. **PR 1 — `detect_band_onsets` primitive.** New `stratum-dsp/src/features/onset/band.rs`. Mirror `detect_spectral_flux_onsets` (`features/onset/spectral_flux.rs:69`) but parameterised to a frequency band. Synthetic TDD: empty input, band-restricted detection (onset only in low band → no detection in high band), bin computation correctness, parameter validation. **Coordinated with kick-pattern detector plan (A2): whichever ships first owns this PR; the other consumes it.**
2. **PR 2 — `dub_stab` module skeleton with synthetic tests written first.** New module, stages 1 and 2 (band onsets + kick-masking + beat-relative per-bar soft histograms). Tests written *before* implementation: synthetic click train at off-beat 8ths → assert peak at bin 16; kicks + stabs together → assert score > 0.5 (catches the bin-0 veto bug); stabs in first half only → assert persistence ≈ 0.5; sidechain pump (no transients) → assert score ≈ 0. Ship behind a feature flag, no `AnalysisResult` integration yet.
3. **PR 3 — Stages 3 and 4.** Template matching (T1+T2 only), masked cosine similarity, log-space decay confirmation. Still gated behind feature flag. Synthetic TDD for each stage's edge cases.
4. **PR 4 — Result-struct integration + schema bump.** Wire into `AnalysisResult`, `StratumResult`, schema version 4 → 5. Re-analysis of cached tracks happens automatically (verified: `cli/mod.rs:476-495` evicts on schema mismatch).
5. **PR 5 — Validation harness and report.** Run against the 60-track fixture set, commit the report. **STOP HERE if validation fails.**
6. **PR 6 — Classification wiring.** Only if PR 5 passes. Add `CharFlag::StabHeavy`, wire into `AudioFeatures`, `compute_audio_profile`, same-family resolver, conjunctive template C2.

Each PR is independently revertable. PR 6 is gated on PR 5's report.

## Cost Estimate

| Stage | Effort |
|---|---|
| PR 1 (band onsets primitive + synthetic tests) | 0.5 day |
| PR 2 (skeleton + stages 1–2 with TDD) | 1.5 days (design fixes added Stage-1 kick masking + per-bar histograms) |
| PR 3 (stages 3–4) | 1.5 days |
| PR 4 (result-struct integration) | 0.5 day |
| PR 5 (validation harness + 60-track run) | 1.5 days (more fixtures than the original 32) |
| PR 6 (classification wiring) | 0.5 day |
| **Total** | ~6 days |

Budget for 7–8 days realistically. Validation may surface algorithmic issues requiring iteration on stages 1–4 (band edges, template set, decay window). The expanded fixture set (N=10 per bucket) is the largest cost increase relative to the original draft.
