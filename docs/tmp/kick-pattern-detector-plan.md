# Kick-Pattern Classifier: Implementation Plan

**Date:** 2026-04-26
**Status:** Detector implemented, classifier wiring still gated on validation pass.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) — this is feature A2 from that doc, the discriminator that pulls Electro and broken-beat genres out of the Techno-family decision space. Sibling of [chord-stab-detector-plan.md](chord-stab-detector-plan.md), with which it shares a `detect_band_onsets` primitive.

## Goal

Add a stratum-dsp feature that classifies the kick-drum metric pattern of a track and reports it as a discrete enum plus a confidence score. Output `kick_pattern: KickPattern` and `kick_pattern_confidence: f32 ∈ [0, 1]`. Cost target: under 5% of total analysis time, reusing the existing shared STFT.

Implementation note: the shipped cache output is intentionally a little richer
than the original proposal so validation can inspect failure modes:
`kick_pattern`, `kick_pattern_confidence`, `kick_kicks_per_bar`,
`kick_onset_count`, `kick_rate_basis`, and a flattened 4x16 `kick_histogram`.
As of schema 17, `kick_kicks_per_bar` and `kick_onset_count` are computed from
deduplicated beat-level kick anchors — at most one event per bar/beat — rather
than raw low-band onset count or dense subdivision activity. The reported
`kick_kicks_per_bar` is hard-capped at four beat anchors per bar.

As of schema 18, `BrokenBeat` includes both eighth-offbeat and early-16th
syncopation templates, and `Halftime` cannot win when the detector sees dense,
near-four-anchor-per-bar low-end activity.

Real-library validation widened the default kick band from 40–120 Hz to
40–200 Hz after the narrow fundamental band produced confident false `Sparse`
labels on acoustic/disco kicks.

```rust
pub enum KickPattern {
    FourOnFloor, // kick on every beat (Techno, House, Trance, Disco)
    BrokenBeat,  // kicks on 1 + 3 with off-beat hits between (Electro, Breakbeat, UK Garage)
    Halftime,    // kick on beat 1 of every other beat-pair (some Dubstep, Halftime DnB)
    Sparse,      // <1 kick per bar on average (Drone, Ambient, Beatless)
    Irregular,   // kicks present but not aligned to any of the above templates
}
```

This is the cleanest single discriminator separating Electro from Techno-family genres. Tonight's POM POM Untitled 42433 (deep-techno-classification-ideas.md table row 4) was tagged Deep House by an automated pass; a kick-pattern classifier flagging it as `BrokenBeat` would have vetoed every 4/4 House and Techno candidate.

## Signal Definition

A kick drum onset, for the purpose of this detector, has these properties:

1. **Frequency band**: dominant energy in 40–200 Hz (kick fundamental plus upper punch). The original default started narrow at 40–120 Hz to avoid bassline pluck contamination, but validation showed missed acoustic/disco kicks. The detector now relies on beat-anchor deduplication and template scoring to keep the wider band from turning dense bass motion into impossible kick counts.
2. **Onset envelope**: short attack (under ~20 ms typical), with a transient spectral flux step in the kick band. Sustained sub-bass with no transient is *not* a kick.
3. **Phase relative to beat grid**: the *position* on the beat grid is the discriminative signal (not just presence). The histogram structure is the classifier input.
4. **Persistence**: kicks present across most of the track, not a brief intro flourish. Sparse-by-design tracks (drone) are detected by the *absence* of dense kick onsets, not by a positive template.

Negative classes the detector must reject as kicks:
- **Snare/clap on backbeats 2 and 4** (mostly wrong band — the 200 Hz ceiling keeps most snare energy out, but validation must watch for bass-heavy claps leaking into the widened band).
- **Bassline pluck onsets** (overlap with kick band but typically have longer decay and less transient flux step). Mitigated by transient-step thresholding rather than long-window energy; risk discussed in #2 below.
- **Sub-bass drone** (no transient onset — fails stage 1 spectral flux threshold).
- **Hi-hats and cymbals** (wrong band — frequency cutoff at 200 Hz hard-rejects).

Negative outputs (cases the detector should *not* over-confidently label):
- Tracks with no detectable kick at all: report `Sparse` if onset count is below a per-bar threshold, *not* `FourOnFloor` with low confidence.
- Tracks with kick onsets in band but no metric alignment: report `Irregular`, not a forced template fit.

## Algorithm

A four-stage pipeline. Each stage outputs an intermediate signal; stage 4 selects the winning enum variant by template scoring.

### Stage 1 — Kick-band onset detection

Reuses the existing shared STFT (`stratum-dsp/src/lib.rs:166`, computed once for all detectors). This is the same primitive proposed in [chord-stab-detector-plan.md §Stage 1](chord-stab-detector-plan.md) — extracted into a shared helper.

```
fn detect_band_onsets(
    fft_magnitudes: &[Vec<f32>],
    sample_rate: u32,
    band_hz: (f32, f32),
    threshold_percentile: f32,
) -> Result<Vec<usize>, AnalysisError>
```

Implementation: compute spectral flux limited to bins in `band_hz`, apply percentile thresholding, and peak-pick. Mirrors `detect_spectral_flux_onsets` at `stratum-dsp/src/features/onset/spectral_flux.rs:69` but parameterised to a band.

For the kick detector, call with `band_hz = (40.0, 200.0)` (configurable) and `threshold_percentile = config.kick_onset_threshold_percentile` (default 0.85, slightly higher than the global `onset_threshold_percentile = 0.80` since kick onsets dominate the kick band's flux distribution and we want only the strong ones).

Output: vector of frame indices where a kick-band onset was detected.

Cost: O(n_frames × bins_in_band). For 40–200 Hz at sr=44100, frame_size=2048, the band covers only a few FFT bins — vanishingly cheap relative to the existing STFT cost.

**Note:** PR 1 from the chord-stab plan extracts this primitive. If the chord-stab detector ships first, this kick-pattern PR is a pure consumer. If kick-pattern ships first, PR 1 of this plan does the extraction; chord-stab consumes it later.

### Stage 2 — Beat-relative offset histogram

Same structure as the chord-stab detector's stage 2, but the *interesting* peaks are at different bins:

| Pattern         | Histogram peak bins (16ths/beat) |
|-----------------|----------------------------------|
| 4-on-floor      | bin 0 only                       |
| Broken-beat     | mix of bin 0 and bin 8           |
| Halftime        | bin 0 *every other beat* (bar-relative) |
| Sparse          | low total count regardless of bin |
| Irregular       | high total count, smeared distribution |

The chord-stab detector cares about bin 8 (off-beat); this detector cares primarily about bin 0 (on-beat) and the *bar-relative* distribution.

```
For each kick-band onset frame f:
    onset_time = f * hop_size / sample_rate                       // seconds
    nearest_beat_idx = binary_search(beat_grid.beats, onset_time)
    if nearest_beat_idx is at the head or tail beyond grid: skip
    beat_period = beat_grid.beats[nearest_beat_idx + 1]
                - beat_grid.beats[nearest_beat_idx]
    beat_offset = (onset_time - beat_grid.beats[nearest_beat_idx])
                  / beat_period   // ∈ [0.0, 1.0)
    beat_in_bar = nearest_beat_idx % 4   // assuming 4/4 — see Risk #5

    histogram_per_beat_position[beat_in_bar][round(beat_offset * 16)] += 1
```

`beat_grid.beats` and `beat_grid.bars` come from the existing `BeatGrid` struct at `stratum-dsp/src/analysis/result.rs:144–154`. Both are `Vec<f32>` in seconds.

Output: a 4×16 histogram (4 beat positions in bar × 16 sixteenth-note offsets per beat) plus the total onset count and the duration in bars (`beat_grid.bars.len() - 1`). This 4×16 shape is what enables halftime and broken-beat detection: 4-on-floor concentrates in column 0 of all 4 rows; halftime concentrates in column 0 of rows 0 and 2 only; broken-beat shows column 0 in rows 0 and 2 (kicks) plus syncopated hits in rows 1 and 3, currently around early-16th columns 3–4 or eighth-offbeat column 8.

Onsets outside the beat grid (before the first detected beat or after the last) are discarded.

### Stage 3 — Template scoring

Define five reference templates as 4×16 normalized weight matrices. Scoring is template correlation × persistence. Templates:

| Template | Description                                         | Non-zero cells (row, col)      |
|----------|-----------------------------------------------------|--------------------------------|
| T_FOUR   | 4-on-floor: kick on every beat                      | (0,0), (1,0), (2,0), (3,0)     |
| T_BROKEN | Electro: kick on 1 + 3, off-beat on 2 + 4            | (0,0), (2,0), (1,8), (3,8)     |
| T_HALF   | Halftime: kick on beat 1 of every two beat-pairs     | (0,0), (2,0) — half weight     |
| T_SPARSE | (no template; reported by total-count threshold)     | n/a                            |
| T_IRREG  | (no template; reported as fallback when no template wins) | n/a                       |

Each template is normalized so its non-zero cells sum to 1.0.

```
For each template t in {FOUR, BROKEN, HALF}:
    template_corr[t] = sum over cells (i,j) of
        normalized_histogram[i][j] * template[t][i][j]

    leakage[t] = sum of histogram weight in cells where template[t] is zero
                 / total histogram weight

    template_score[t] = template_corr[t] * (1 - leakage[t])
```

Persistence: `kicks_per_bar = total_onsets / total_bars`. Used for sparse detection and for downweighting templates when too many bars are silent.

```
If total_onsets / total_bars < 0.5:
    pattern = Sparse
    confidence = 1.0 - (total_onsets / total_bars).clamp(0, 1)
    return

best_template = argmax(template_score)
best_score = max(template_score)

If best_score < 0.4:
    pattern = Irregular
    confidence = 1.0 - best_score   // higher confidence the worse the fits
    return
```

Disambiguation rules between the three winning templates require care because T_FOUR and T_HALF share column 0 in rows 0 and 2:

- **T_FOUR vs T_HALF**: ratio of (rows 1+3 column 0 mass) to (rows 0+2 column 0 mass). If > 0.4, the kick is hitting all 4 beats → T_FOUR. If < 0.15, only beats 1 and 3 are firing → T_HALF. Between 0.15 and 0.4, low-confidence T_FOUR (the user may have a missing-kick variation).

- **T_FOUR vs T_BROKEN**: ratio of (column 8 mass in rows 1+3) to (column 0 mass in rows 0+2). If > 0.5, the off-beat in 2 and 4 is loud enough to read as broken-beat → T_BROKEN. The on-beat 1+3 mass should also be present (broken-beat is *not* the absence of beats 1+3, only the presence of off-beats on 2+4).

- **Broken-beat veto on FourOnFloor**: if T_BROKEN's column-8 mass in rows 1+3 exceeds 30% of total kick mass, never report FourOnFloor — even if T_FOUR's correlation is higher, the off-beat presence is diagnostic of Electro and a 4-on-floor with strong off-beat hi-hats is rare in the kick band.

```
final_pattern = match (template_score, ratios) {
    (T_BROKEN > 0.4 && col8_ratio > 0.5) => BrokenBeat,
    (T_HALF > 0.4 && rows13_ratio < 0.15) => Halftime,
    (T_FOUR > 0.4) => FourOnFloor,
    _ => Irregular,
}

confidence = best_score
            * persistence_factor
            * (1.0 - second_best_score / best_score) // margin

where persistence_factor = (kicks_per_bar / 4.0).clamp(0.0, 1.0)
                           // 1.0 when ≥4 kicks/bar (4-on-floor density),
                           // less when sparser
```

### Stage 4 — Halftime sanity check

Halftime is the most error-prone class. Two specific failure modes:

1. **Real-tempo confusion**: tracks at 70–90 BPM where the kick *is* once per beat (slow Techno, slow Dubstep) get tagged Halftime by the histogram alone. Counter: only report `Halftime` if the BPM is ≥ 100. Below that, halftime and FourOnFloor are indistinguishable from the histogram, and the BPM detector has likely already chosen the right metrical level.
2. **Halftime-felt at double tempo**: a 140 BPM Dubstep track with a halftime kick reads as "kick on every other beat" only if the BPM detector picked 140 (not 70). The metrical-agreement logic at `stratum-dsp/src/lib.rs:819–892` is generally robust on the modern Dubstep canon, so this should be rare. If validation surfaces it, mitigation is to additionally check half-BPM templates against the histogram.

```
If pattern == Halftime && bpm < 100.0:
    pattern = FourOnFloor    // re-classify; the kick IS once per beat at this BPM
    confidence *= 0.7        // slight downgrade; original detection was ambiguous
```

### Final output

```rust
pub struct KickPatternResult {
    pub pattern: KickPattern,
    pub confidence: f32,
    pub kicks_per_bar: f32,
    pub debug: KickPatternDebug,  // raw histogram, template scores, ratios
}
```

## Integration Points

### stratum-dsp side

1. **Shared primitive (PR 1):** `stratum-dsp/src/features/onset/band.rs` (new file, ~150 LOC). Defines `pub fn detect_band_onsets(stft, sample_rate, band_hz, threshold_percentile) -> Result<Vec<usize>, AnalysisError>`. Tests for synthetic band-localised onsets. *Coordinated with chord-stab plan: whichever lands first owns this PR; the other consumes it.*

   **Implemented:** this primitive already existed from the chord-stab work and
   A2 consumes it directly.

2. **New module:** `stratum-dsp/src/features/kick_pattern.rs` (~300 LOC). Contains:
   - `pub fn detect_kick_pattern(stft, beat_grid, sample_rate, config) -> KickPatternResult`
   - `pub enum KickPattern { FourOnFloor, BrokenBeat, Halftime, Sparse, Irregular }`
   - `pub struct KickPatternResult { pattern, confidence, kicks_per_bar, debug }`
   - Helpers: `compute_beat_relative_histogram`, `score_templates`, `disambiguate`

   **Implemented:** the enum/result live in `analysis::result`; the detector
   module returns `KickPatternAnalysis` and includes synthetic tests for
   four-on-floor, broken-beat, halftime, sparse, irregular, low-BPM halftime
   collapse, and MainGroove bar counting.

3. **Module registration:** `stratum-dsp/src/features/mod.rs` — add `pub mod kick_pattern;`.

4. **Pipeline wiring:** `stratum-dsp/src/lib.rs:1591–1632`. After `decay` is computed (line 1605), call `detect_kick_pattern(&magnitude_spec_frames, &beat_grid, sample_rate, &config)`. Slot in alongside `mod_centroid`, `harmonic_proportion`, `decay`. Skip detection (return `Sparse`-by-default) if `beat_grid.beats.len() < 8` — too few beats for a meaningful histogram.

5. **Result struct:** `stratum-dsp/src/analysis/result.rs:185–232` — add fields:
   ```rust
   #[serde(skip_serializing_if = "Option::is_none")]
   pub kick_pattern: Option<KickPattern>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub kick_pattern_confidence: Option<f32>,
   ```
   `KickPattern` is `serde::{Serialize, Deserialize}` via standard derive. Stored as the enum variant name (`"FourOnFloor"`, etc.) in cache JSON.

6. **Config:** `stratum-dsp/src/config.rs` — add fields to `AnalysisConfig` (around the existing onset detection block at line 23–43, defaults near line 666):
   ```rust
   /// Kick-band lower frequency cutoff (Hz). Default: 40.0.
   pub kick_band_low_hz: f32,
   /// Kick-band upper frequency cutoff (Hz). Default: 200.0.
   pub kick_band_high_hz: f32,
   /// Threshold percentile for kick-band onset detection. Default: 0.85.
   pub kick_onset_threshold_percentile: f32,
   /// Minimum template score for non-Irregular classification. Default: 0.4.
   pub kick_template_threshold: f32,
   /// Minimum kicks-per-bar for non-Sparse classification. Default: 0.5.
   pub kick_sparse_threshold: f32,
   /// Minimum BPM at which Halftime is allowed (below this, fall back to FourOnFloor). Default: 100.0.
   pub kick_halftime_min_bpm: f32,
   ```

7. **Public API:** `pub use features::kick_pattern::{KickPattern, KickPatternResult};` in `stratum-dsp/src/lib.rs`.

### reklawdbox side

8. **`StratumResult`:** `src/audio.rs:30–48` — add `pub kick_pattern: Option<String>` and `pub kick_pattern_confidence: Option<f64>`. String rather than enum on the Rust side to avoid coupling reklawdbox to stratum-dsp's enum (the cache JSON already contains the variant name; reklawdbox parses it back into its own enum if needed).

9. **Schema version:** `src/audio.rs:60` — bump `STRATUM_SCHEMA_VERSION` from `"11"` to `"18"`. This auto-evicts cached results without the new field on next load, avoids mixing raw-onset / subdivision-dedup detector density with beat-anchor detector density, and avoids mixing the first single-template `broken_beat` pass with the broader syncopation templates.

10. **Mapping:** `src/tools/classify_handler.rs:647–722` (`extract_audio_features`). Add field extraction to `AudioFeatures`:
    ```rust
    kick_pattern: stratum_json
        .as_ref()
        .and_then(|sj| sj.get("kick_pattern"))
        .and_then(serde_json::Value::as_str)
        .map(|s| s.to_string()),
    kick_pattern_confidence: stratum_json
        .as_ref()
        .and_then(|sj| sj.get("kick_pattern_confidence"))
        .and_then(serde_json::Value::as_f64),
    ```

11. **`AudioFeatures`:** `src/classify.rs:117–155` — add fields:
    ```rust
    #[allow(dead_code)]
    pub(crate) kick_pattern: Option<String>,
    #[allow(dead_code)]
    pub(crate) kick_pattern_confidence: Option<f64>,
    ```
    Stays `#[allow(dead_code)]` until validation passes and it is wired into `compute_audio_profile`.

12. **Classification consumption:** see "Classification wiring" below — gated on validation.

## Validation (gates everything else)

The prior research doc ([genre-classification-improvements.md](genre-classification-improvements.md)) documents the empirical invalidation of `harmonic_proportion`, `mod_centroid`, `bpm_confidence`, and `grid_stability`. **Do not propose using those features in this detector or in classification rules that consume its output.** Patterns that look obvious from theory have failed empirical validation before. The detector ships and runs against the validation set before being wired into classification.

### Fixture set

Build a small ear-verified WAV fixture set for offline validation, separate from the existing integration test fixtures.

| Bucket          | Target N | Examples |
|-----------------|----------|----------|
| FourOnFloor (Techno/House) | 8 | Marcel Dettmann, Ben Klock, classic Detroit Techno; Loco Dice, classic Chicago House |
| BrokenBeat (Electro)       | 6 | Drexciya, DMX Krew, Dopplereffekt, Andrea, Helena Hauff, Untitled 42433 (POM POM) |
| BrokenBeat (Breakbeat / UK Garage) | 4 | Anything with non-Electro broken-beat — Burial, MJ Cole, classic 90s breakbeat |
| Halftime (Dubstep / DnB)   | 4 | Halftime DnB (Ivy Lab, Halogenix), 140 BPM dubstep with halftime kick |
| Sparse (Drone/Ambient)     | 4 | Voices From The Lake, Donato Dozzy reductive, Stars Of The Lid |
| Irregular                  | 2 | Free-jazz percussion, Aphex Twin Drukqs, IDM with shifting time |

Total: ~28 tracks. The user's collection should cover most of these.

Source the fixtures from the user's `genre_verified` Rekordbox playlist where possible. Halftime and BrokenBeat-as-non-Electro buckets are the trickiest to source from a Techno-heavy library; if N falls below 4 in a bucket, validation passes/fails on the buckets that *do* meet N and the missing buckets are deferred to a later top-up round. **Do not** ship classification wiring against undertested buckets.

### Early listening notes from `genre_verified`

First-pass listening on the schema 17 detector output changed the intended
classifier wiring:

- Shifted — "She Dressed In Grey (Static Mix)" and Qemist — "Peaking (OG Mix)"
  are both Techno despite being labelled `broken_beat`. Treat `broken_beat` as
  syncopated kick-pattern evidence, not as a genre veto.
- Forest Drive West — "Phosphenes" was labelled `halftime` with low confidence
  (`0.405`) but is plausibly Breakbeat by listening judgement. This boundary
  needs a dedicated Halftime-vs-Breakbeat fixture set before any classifier
  consumption. A corrected Rekordbox beat grid showed near-four-anchor density
  with early 16th syncopation, so schema 18 treats this as detector logic rather
  than a taxonomy decision.
- Fred P — "Portal 5" is a correct `sparse` ambient/no-kick case.
- Sister Sledge — "Pretty Baby" is classic Disco with significant beat drift;
  `irregular` here means the detector failed to find a stable template, not
  that Disco should be downweighted.

### Validation acceptance criteria

The detector ships into classification only if:

1. **Per-bucket accuracy ≥ 80%**: at least 80% of fixture tracks in each bucket are classified into the correct enum variant.
2. **No FourOnFloor → BrokenBeat confusion above 10%**: at most 10% of fixture FourOnFloor tracks are classified as BrokenBeat. (This is the highest-cost confusion — see Risk #1.)
3. **No BrokenBeat → FourOnFloor confusion above 10%**: as above, in the other direction.
4. **Confidence calibration**: in correctly-classified tracks, the mean confidence is ≥ 0.6; in *incorrectly* classified tracks, the mean confidence is ≤ 0.5. (We want low confidence on errors so the classifier can downweight them.)
5. **Sparse correctly identifies all drone fixtures**: 4/4 Sparse-bucket tracks classified as `Sparse`. Zero tolerance — this is a low-density signal that should be unambiguous.

Failure modes and recourse:
- If criterion (1) fails on the Halftime bucket — likely the BPM-floor sanity check at stage 4 needs adjustment, or the test fixtures are mostly low-BPM. Iterate on `kick_halftime_min_bpm`.
- If criterion (2) or (3) fails — the templates or disambiguation thresholds need tuning. Inspect histograms for the misclassified tracks and adjust ratios.
- If criterion (4) fails (over-confident errors) — multiplicative confidence factors are wrong. The `(1 - second_best/best)` margin term may be too lenient; tighten or add a sigmoid.
- If criterion (5) fails — `kick_sparse_threshold` is too low; raise to 0.7 or 1.0 kicks-per-bar.

### Validation harness

```
stratum-dsp/tests/kick_pattern_validation.rs (new, gated behind a feature flag or env
var since it's expensive and depends on local fixtures, mirroring the chord-stab
validation harness):

  - Iterate over fixture WAV files, decode, run analyze_audio, capture
    kick_pattern + confidence + raw histogram.
  - Group by bucket from filename prefix (e.g. `four_on_floor__dettmann_*.wav`,
    `broken_beat__drexciya_*.wav`) or a manifest TOML.
  - Print per-track classification, per-bucket accuracy, confusion matrix.
  - Assert acceptance criteria.
  - Output: a markdown report committed alongside PR 5.
```

This is offline, run-on-demand. Standard `cargo test` integration tests should not depend on it.

## Classification Wiring (post-validation)

Once validation passes, the feature wires in at three layers, each independently testable:

1. **Tree-side flag:** Extend `enum CharFlag` at `src/classify.rs:181` with new variants:
   ```rust
   FourOnFloor,
   BrokenBeat,
   Halftime,
   SparseKick,
   ```
   In `compute_audio_profile` at `src/classify.rs:280`, set the appropriate flag based on `audio.kick_pattern` *only if* `audio.kick_pattern_confidence > 0.5`. Below that threshold, no flag is set — classification falls back to existing logic.

2. **Soft rhythm evidence for BrokenBeat:** Do **not** add a hard veto against
   Techno, House, Tech House, Deep Techno, Trance, or Disco. Real-library
   validation found Techno tracks with legitimate syncopated/darker rhythmic
   movement that match the `broken_beat` template. Instead, use high-confidence
   `BrokenBeat` only as positive supporting evidence for Electro, Breakbeat, UK
   Garage, Jungle, and related non-straight rhythm candidates. It may reduce a
   candidate's score only when another evidence source already supports a
   non-4/4 family; it must not force a genre switch by itself.

3. **Conjunctive template C1 enrichment:** In the Deep Techno template, prefer
   `FourOnFloor` when present, but absence of `FourOnFloor` or presence of
   `BrokenBeat` is not enough to downgrade Deep Techno. Use it only as a small
   confidence adjustment after enrichment/audio profile evidence has already
   selected a Techno-family candidate.

4. **Halftime steering:** Keep `Halftime` as a rhythm flag, not a genre-taxonomy
   entry, until the library has a dedicated fixture set. The current detector is
   kick-band only: it sees sparse on-grid low-frequency anchors on a fast grid;
   it does not detect snare/backbeat placement. Use it only to support
   Dubstep/DnB/Breakbeat candidates when other evidence already points there.
   Do not downweight Techno-family candidates from `Halftime` alone.

5. **Sparse → Ambient**: If `SparseKick` is set with confidence > 0.7 and energy bucket is `NonDancefloor`, escalate the existing Ambient veto at `src/classify.rs:359` to high-confidence rather than its current low-medium.

Default vote weights: start at `0.2` to `0.3` for rhythm evidence and tune only
after listening validation. `SparseKick` may be stronger for non-dancefloor
Ambient because the first listening pass produced a clean positive example.

## Risks and Open Questions

1. **FourOnFloor + missed kicks vs BrokenBeat.** Some Techno tracks have intentional kick drops where 1–2 beats are silent. The histogram for these reads as fewer kicks but still on-beat — the disambiguation ratio (rows 1+3 / rows 0+2 column 0) handles this correctly because the *missing* kicks reduce both numerator and denominator. The risk is the *opposite*: a broken-beat track where the column-0 mass coincidentally dominates because the off-beat hits are quieter than the on-beat kicks. Mitigation: the broken-beat veto at stage 3 (column-8 mass > 30% of total) catches the case where off-beats are merely present, not necessarily loudest. Validation criterion (3) is explicitly about this.

2. **Sub-bass-layered kicks.** Tracks with a kick layered against a sustained sub-bass note in the same band (40–200 Hz) can confuse the spectral flux step — the sub-bass dominates the band magnitude and the kick transient becomes a smaller relative step. Mitigation 1: the percentile-thresholding peak-pick in `detect_band_onsets` is robust to a slowly-varying baseline by design (only fast changes pass). Mitigation 2: if validation surfaces this again, split into two bands (40–80 Hz sub vs 80–160 Hz kick attack) and require an onset in both to count.

3. **Snare/clap on backbeats 2 and 4 dominating the kick onsets.** This is the canonical concern: if the snare's lower harmonics leak into the 80–200 Hz range, the histogram can show column 0 in rows 1 and 3 as well — looking like FourOnFloor instead of "1+3 kicks + 2+4 snare". The widened band is a recall tradeoff after real-library disco false negatives. Verify empirically: the FourOnFloor-bucket fixtures all have backbeat snares, so if criterion (1) passes, this risk is empirically managed. If it fails on tracks with particularly bass-heavy claps (some Tech House), narrow the band or split kick body/attack bands.

4. **Halftime detection vs "real" tempo at 70–90 BPM.** Stage 4 `kick_halftime_min_bpm` (default 100) handles this, but it's an arbitrary cutoff. Real risk: a 95-BPM slow Techno track with a halftime feel could be misclassified either way. Acceptable: tag it FourOnFloor at 95 BPM (it functions as 4/4 in mixing context), accept the false-Halftime cost as small.

5. **Time signatures other than 4/4.** The histogram bins by `nearest_beat_idx % 4` — this assumes 4/4. The detector reads from `beat_grid.beats` and `beat_grid.bars`; the existing `time_signature` module at `stratum-dsp/src/features/beat_tracking/time_signature.rs` may already provide a better modulus. Open question: should the detector check the time signature first and bin by `% time_sig.numerator` instead of `% 4`? For the current verified-tracks set (overwhelmingly 4/4) this is moot. Mitigation: add a fallback to `Irregular` if the detected time signature isn't 4/4, and revisit if 3/4 or 6/8 fixtures matter later.

6. **Beat grid quality.** Per the chord-stab plan's Risk #4, if the beat grid is unreliable, beat-relative offset histograms are noise. **However: the chord-stab plan proposed gating on `grid_stability < 0.4`. That feature was empirically invalidated** ([genre-classification-improvements.md](genre-classification-improvements.md)). Use a different gate: skip detection (return `pattern = Irregular`, `confidence = 0.0`) if `beat_grid.beats.len() < 8` (too few beats to histogram) or if `total_bars < 4`. Both are structural rather than statistical and don't depend on the invalidated `grid_stability`.

7. **Tracks with kick variations across sections.** A track with a 4/4 main groove and a halftime breakdown averages across the full duration. The detector reports the mode — likely FourOnFloor — and silently ignores the breakdown. Acceptable for genre classification (the dominant pattern is what matters); but the `debug` output should include per-bar histograms so future validation can detect this.

8. **Cost estimate accuracy.** Claimed <5% overhead. Realistically 2–4% — the band onset detection is cheap (4–6 bins of an existing STFT), histogram construction is O(n_onsets), template scoring is O(64). No iterative loops. Worth measuring once implemented; budget 5% for safety.

## Suggested Implementation Order

A single PR is too big. Break it up:

1. **PR 1 — `detect_band_onsets` primitive.** Refactor existing spectral-flux onset detection to take an optional band, since it's reusable beyond kick detection (also used by chord-stab A1). New `stratum-dsp/src/features/onset/band.rs`. Tests against synthetic band-localised onsets. *Skip if chord-stab plan's PR 1 has already landed.*
2. **PR 2 — `kick_pattern` module skeleton.** New module, stages 1 and 2 (kick onsets + 4×16 histogram). Ship behind a feature flag, no integration into `AnalysisResult` yet. Tests against synthetic kick patterns (4-on-floor, halftime, broken-beat synthetic kicks at known BPM).
3. **PR 3 — Stages 3 and 4.** Template scoring, disambiguation, halftime BPM check. Still gated behind feature flag. Tests against synthetic input.
4. **PR 4 — Result-struct integration + schema bump.** Wire into `AnalysisResult`, `StratumResult`, schema version. Re-analysis of cached tracks happens automatically on next load.
5. **PR 5 — Validation harness and report.** Run against the fixture set, commit the report. **STOP HERE if validation fails.** Iterate on PR 3 thresholds if any failures are tuning issues; deeper failures route back to PR 2 (band/template choice).
6. **PR 6 — Classification wiring.** Only if PR 5 passes. Add `CharFlag::{FourOnFloor, BrokenBeat, Halftime, SparseKick}`, wire into `AudioFeatures`, `compute_audio_profile`, `check_audio_vetoes`, conjunctive templates C1 and C4.

Each PR is independently revertable. PR 6 is gated on PR 5's report.

## Cost Estimate

| Stage | Effort |
|---|---|
| PR 1 (band onsets refactor) | 0.5 day — *0 days if chord-stab PR 1 already landed* |
| PR 2 (skeleton + stages 1–2) | 1 day |
| PR 3 (stages 3–4) | 1.5 days |
| PR 4 (result-struct integration) | 0.5 day |
| PR 5 (validation harness + run) | 1.5 days (mostly fixture sourcing, esp. Halftime/Breakbeat) |
| PR 6 (classification wiring) | 0.5 day |
| **Total** | ~5.5 days (4.5 days if PR 1 already landed) |

Budget for 6–7 days realistically. Validation may surface algorithmic issues requiring iteration on stages 2–3 (template selection, disambiguation ratios). The Halftime bucket is the highest-risk validation case; if its fixture sourcing is harder than expected, it can ship without Halftime support (return `FourOnFloor` for halftime tracks at first) and Halftime can be added in a follow-up.
