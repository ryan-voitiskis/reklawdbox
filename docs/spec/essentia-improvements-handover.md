# Essentia Analysis Improvements — Handover Prompt

Use this document as a complete handover for improving how reklawdbox extracts, normalizes, and utilizes Essentia audio descriptors in genre classification and set building.

---

## Context

reklawdbox is a Rust MCP server for Rekordbox 7.x DJ library management. It runs Essentia (a Python audio analysis library) as a subprocess to extract perceptual audio features, caches them in a SQLite store, and uses them in two workflows:

1. **Genre classification** — an agent SOP (`docs/genre-classification-sop.md`) that classifies tracks using enrichment data (Discogs, Beatport) with an audio-only fallback (Step C) when enrichment is unavailable.
2. **Set building** — tools (`score_transition`, `build_set`) that sequence tracks using key, BPM, energy, and genre compatibility scoring.

The Essentia extraction script lives inline in `src/audio.rs` as the `ESSENTIA_SCRIPT` constant (lines 32-134). The energy computation that consumes the data lives in `src/tools.rs` in `compute_track_energy()` (lines 3412-3441). Transition scoring is in `score_transition_profiles()` (line 3185) and energy phase evaluation in `score_energy_axis()` (line 3301).

---

## Part 1: Bug Fixes in the Essentia Script

### Bug A: OnsetRate returns first onset timestamp, not onset rate

**File:** `src/audio.rs`, line 108

```python
features["onset_rate"] = first_scalar_or_none(es.OnsetRate()(audio))
```

**Problem:** `es.OnsetRate()(audio)` returns a tuple: `(onsets_vector, onset_rate_scalar)`. The `first_scalar_or_none()` helper iterates the tuple, finds the onsets vector first, and returns `float(arr.reshape(-1)[0])` — the timestamp of the first onset in seconds (e.g., 0.23). This is semantically wrong. The onset rate (onsets per second, e.g., 5.2) is at index `[1]`.

**Fix:** Extract the onset rate specifically:

```python
onset_result = es.OnsetRate()(audio)
if isinstance(onset_result, (tuple, list)) and len(onset_result) >= 2:
    features["onset_rate"] = first_scalar_or_none(onset_result[1])
else:
    features["onset_rate"] = first_scalar_or_none(onset_result)
```

**Impact:** The `onset_rate` field is one of three inputs to `compute_track_energy()` in `tools.rs:3426-3428`, weighted at 30%. With the bug, all tracks get near-zero "onset rates" (since first-onset timestamps are typically < 1.0 second), and the normalization `onset / 10.0` produces values near 0.0. This drags down every track's energy score.

### Bug B: Danceability normalization squashes discrimination

**File:** `src/tools.rs`, line 3434

```rust
(0.4 * dance.clamp(0.0, 1.0))
```

**Problem:** Essentia's Danceability algorithm (Detrended Fluctuation Analysis) outputs values in the range 0 to ~3, not 0-1. See [Essentia docs](https://essentia.upf.edu/reference/std_Danceability.html): "Normal values range from 0 to ~3." Clamping at 1.0 means most club-playable tracks (which score 1.0-2.5) all become 1.0. The 40%-weighted component becomes a constant for danceable music.

**Fix:** Normalize by dividing by 3.0 instead of clamping at 1.0:

```rust
let normalized_dance = (dance / 3.0).clamp(0.0, 1.0);
```

An alternative approach: compute the empirical max from the user's library at cache-coverage time and normalize against that. The `/3.0` approach is simpler and sufficient given the documented range.

**Impact:** Combined with Bug A, the current energy formula effectively reduces to `0.3 * normalized_loudness` for most tracks. Key/BPM dominate set building because the energy axis has almost no discriminative power.

### Non-bug note: LoudnessEBUR128 indexing is already correct

The script at lines 75-101 correctly indexes `ebu[2]` for integrated loudness and `ebu[3]` for loudness range, with fallback logic. This was fixed in a prior iteration. No action needed.

### Testing gap

There is already:

- A `run_essentia()` script-execution test with fake Essentia modules (`src/audio.rs` test module), and
- An ignored real-track Essentia cache round-trip test (`analyze_track_audio_essentia_cache_round_trip_real_track` in `src/tools.rs`).

What is still missing is a targeted integration assertion for the specific descriptor semantics in this change. Extend one of the existing ignored integration tests (or add a new one) to assert:

1. Runs the full `ESSENTIA_SCRIPT` on a real audio file from the Rekordbox backup.
2. Asserts `onset_rate` is > 1.0 (typical club tracks: 3-8 onsets/sec).
3. Asserts `danceability` can exceed 1.0 (validates the range assumption).
4. Asserts `loudness_integrated` is in a plausible LUFS range (-30 to 0).

---

## Part 2: Danceability Normalization in Energy Scoring

**File:** `src/tools.rs`, `compute_track_energy()` (line 3412)

Current formula:

```rust
energy = (0.4 * dance.clamp(0.0, 1.0))
       + (0.3 * ((loudness + 30.0) / 30.0).clamp(0.0, 1.0))
       + (0.3 * (onset / 10.0).clamp(0.0, 1.0))
```

After fixing Bug A (onset_rate) and Bug B (danceability), update to:

```rust
let normalized_dance = (dance / 3.0).clamp(0.0, 1.0);
let normalized_loudness = ((loudness + 30.0) / 30.0).clamp(0.0, 1.0);
let onset_rate_normalized = (onset / 10.0).clamp(0.0, 1.0);
((0.4 * normalized_dance) + (0.3 * normalized_loudness) + (0.3 * onset_rate_normalized))
    .clamp(0.0, 1.0)
```

The BPM proxy fallback (line 3415) remains unchanged — it's only used when Essentia data is unavailable.

### Test updates

After the normalization change, existing test fixtures that provide `"danceability": 0.82` as if it were 0-1 scale will produce lower energy scores (0.82/3.0 = 0.27 instead of 0.82). Either:

- Update fixture values to realistic 0-3 range (e.g., `"danceability": 2.46` for a very danceable track), or
- Add a comment explaining the test uses synthetic values and the math still exercises all branches.

Search `tools.rs` for `"danceability":` to find all fixture locations. At time of writing these include test functions near lines 4183, 5314, 6222, 6493, and 6503.

---

## Part 3: Utilize Unused Descriptors in Set Building

Four Essentia descriptors are extracted and cached but never consumed: `loudness_range`, `dynamic_complexity`, `rhythm_regularity`, and `spectral_centroid_mean`. Three of these have clear set-building value.

### 3a: Add brightness compatibility scoring (spectral_centroid_mean)

**Rationale:** Transitioning from a dark, sub-heavy track (low centroid, e.g. ~800 Hz) to a bright, sparkly track (high centroid, e.g. ~3500 Hz) sounds jarring even when key/BPM match. Matching timbral color makes transitions smoother.

**Implementation:** Add a `score_brightness_axis()` function alongside the existing axis scorers in `tools.rs`. Score based on the absolute delta of spectral centroid values between two tracks, normalized:

```
centroid_delta = abs(to_centroid - from_centroid)
if delta < 300 Hz:  1.0 (similar brightness)
if delta < 800 Hz:  0.7 (noticeable but acceptable)
if delta < 1500 Hz: 0.4 (quite different)
else:               0.2 (jarring)
```

This requires reading `spectral_centroid_mean` from the essentia cache in `build_track_profile()` and storing it on `TrackProfile`.

### 3b: Add rhythm compatibility scoring (rhythm_regularity)

**Rationale:** Transitioning from a straight four-on-the-floor track (rhythm_regularity ~1.0) to a broken-beat track (rhythm_regularity ~0.5) is one of the hardest DJ maneuvers. The set builder should penalize or flag this.

**Implementation:** Add a `score_rhythm_axis()`:

```
regularity_delta = abs(to_regularity - from_regularity)
if delta < 0.1:  1.0 (similar groove)
if delta < 0.25: 0.7 (manageable shift)
if delta < 0.5:  0.4 (challenging mix)
else:            0.2 (groove clash)
```

### 3c: Use loudness_range for phase-boundary track selection

**Rationale:** Tracks with high loudness range (LRA) have built-in dramatic structure — big breakdowns and builds. These are ideal at phase transition points (warmup→build, peak→release) because their internal dynamics carry the energy shift. Low-LRA tracks are better during sustained peak because they maintain consistent energy.

**Implementation:** In `score_energy_axis()`, add a bonus when a high-LRA track appears at a phase boundary:

- Define "phase boundary" as positions where the phase changes from the previous position.
- If the track's `loudness_range` exceeds a threshold (e.g., > 8.0 LU) and the position is a phase boundary, boost the energy score by 0.1 (clamped to 1.0).
- If the track's `loudness_range` is low (< 4.0 LU) and we're in sustained peak, boost by 0.05.

This requires passing phase context (previous phase vs current phase) into the scoring function, which is a minor structural change.

### 3d: Expand composite scoring to 6 axes

Current composite: `key + bpm + energy + genre` (4 axes, weights sum to 1.0).

Proposed composite: `key + bpm + energy + genre + brightness + rhythm` (6 axes).

**Weight redistribution:**

| Priority | key | bpm | energy | genre | brightness | rhythm |
|----------|-----|-----|--------|-------|------------|--------|
| Balanced | 0.30 | 0.20 | 0.18 | 0.17 | 0.08 | 0.07 |
| Harmonic | 0.48 | 0.18 | 0.12 | 0.08 | 0.08 | 0.06 |
| Energy | 0.12 | 0.18 | 0.42 | 0.12 | 0.08 | 0.08 |
| Genre | 0.18 | 0.18 | 0.12 | 0.38 | 0.08 | 0.06 |

The new axes have small weights — they act as tiebreakers and smoothing, not dominant factors. Key and BPM remain the primary drivers.

**Graceful degradation:** Returning 0.5 for unknown brightness/rhythm is a reasonable default, but with fixed 6-axis weights it still reduces composite score versus known values (roughly 0.07-0.08 depending on priority). If the goal is true "no penalty for missing Essentia," renormalize weights over the available axes per transition.

**Changes required:**
- Add `brightness: Option<f64>` and `rhythm_regularity: Option<f64>` to `TrackProfile` struct (line 2884).
- Read from essentia cache in `build_track_profile()` (line 3135).
- Add `score_brightness_axis()` and `score_rhythm_axis()` functions.
- Update `TransitionScores` struct to include the new axes.
- Update `composite_score()` to accept 6 values.
- Update `priority_weights()` to return 6-tuples.
- Update `score_transition_profiles()` to compute all 6 axes.
- Update the JSON output in `score_transition` and `build_set` tools to include the new axis scores.

---

## Part 4: Utilize Essentia in Genre Classification

The genre classification SOP (`docs/genre-classification-sop.md`) currently uses Essentia only as a last-resort fallback in Step C (audio-only inference) with a 7-row lookup table mapping BPM + rhythm_regularity + danceability → genre family.

### 4a: Use audio as tie-breaking evidence (new Step B.9b / Step 9b)

When Discogs and Beatport disagree (Step B.9, currently → "insufficient, present both to user"), audio features could break the tie and raise confidence to "low" instead of "insufficient."

**Decision logic to add after Step B.9 in the SOP:**

> 9b. **Discogs/Beatport disagree + audio features available:**
> Compare each candidate genre against audio feature expectations. Pick the candidate that better matches:
>
> | Discriminator | Favors genre A when | Favors genre B when |
> |---|---|---|
> | spectral_centroid_mean | Lower (darker) | Higher (brighter) |
> | rhythm_regularity | Higher (straighter) | Lower (more broken) |
> | dynamic_complexity | Lower (steadier) | Higher (more dynamic) |
> | BPM range | Within genre A's typical range | Within genre B's typical range |
>
> Common conflicts this resolves:
> - **Deep House vs Tech House:** Deep House = darker (lower centroid), fewer onsets. Tech House = brighter, busier.
> - **Techno vs Trance:** Trance = higher BPM (typically 135+), very high danceability (>2.0).
> - **House vs Garage:** Garage = lower rhythm regularity (swung beats), higher dynamic complexity.
> - **Techno vs Electro:** Electro = more syncopated (lower regularity), sparser onsets.
>
> If audio clearly favors one candidate: `suggested_genre` = that candidate. Confidence = **low** (audio-assisted). Note the reasoning.
> If audio is ambiguous: remain **insufficient**. Present both options to user.

This is an SOP change only — no tool code change needed, since `resolve_tracks_data` already surfaces the full essentia JSON to the agent.

### 4b: Expand the audio-only fallback table (Step C)

Replace the current 7-row table with a more discriminating version that uses a broader descriptor set (BPM, rhythm_regularity, danceability, spectral_centroid_mean, dynamic_complexity). The table should use correctly normalized values (for example, danceability on a 0-3 scale).

**Proposed expanded table:**

| BPM | Rhythm Reg | Danceability | Spectral Centroid | Dynamic Comp | Likely genre |
|---|---|---|---|---|---|
| 160-180 | any | > 1.5 | any | any | Drum & Bass |
| 135-150 | > 0.9 | > 2.0 | Mid-High | Mid-High | Trance |
| 128-145 | > 0.9 | > 1.8 | Mid | High | Techno (peak/driving) |
| 128-140 | > 0.85 | > 1.2 | Very Low | Low-Mid | Deep Techno / Dub Techno |
| 125-140 | > 0.9 | > 1.0 | Low-Mid | Low | Minimal |
| 125-135 | > 0.85 | > 1.5 | Low | Low-Mid | Deep House |
| 125-135 | > 0.85 | > 1.5 | Mid-High | Mid | Tech House |
| 118-130 | > 0.8 | > 1.5 | Mid | Mid | House |
| 120-135 | 0.6-0.8 | > 1.0 | Mid | Mid-High | Garage / UK Garage |
| 120-140 | < 0.7 | > 1.0 | any | Mid | Breakbeat |
| 80-115 | any | > 1.0 | any | any | Hip Hop or Downtempo |
| 60-110 | < 0.5 | < 0.8 | Low | Low | Ambient |
| 128-145 | > 0.85 | > 1.0 | Mid-High | High | Hard Techno |

All matches remain confidence = **low** with "audio-only inference" note.

**Centroid range guidelines** (these are rough and should be calibrated against the user's library):
- Very Low: < 600 Hz
- Low: 600-1200 Hz
- Mid: 1200-2500 Hz
- Mid-High: 2500-4000 Hz
- High: > 4000 Hz

This is an SOP document change. The agent applies the table at runtime — the tool just provides the raw data.

---

## Part 5: Future — Multi-dimensional energy profiles

This section describes a larger refactor that should NOT be done in the same PR as Parts 1-4. Document it for future reference.

### Concept

Instead of one scalar `energy` per track, compute three axes:

| Axis | Inputs | Captures |
|---|---|---|
| **Intensity** | loudness_integrated + onset_rate | How hard the track hits |
| **Drive** | danceability + rhythm_regularity | How propulsive the groove is |
| **Brightness** | spectral_centroid_mean | Timbral color (dark ↔ bright) |

The set builder could then shape independent curves per axis:
- **Intensity curve:** warmup → peak → release (standard energy arc)
- **Drive curve:** keep consistently high during peak, allowed to dip for transitions
- **Brightness curve:** dark opening → bright peak → dark closing (timbral journey)

### Why defer

This changes the `EnergyPhase` model, the `build_set` API surface (callers would need to specify multi-axis curves), and all the scoring functions. It's a clean breaking change that should happen after Parts 1-4 are validated and the current energy scoring is proven to work correctly with fixed inputs.

---

## File Reference

| File | Lines | What lives there |
|---|---|---|
| `src/audio.rs` | 32-134 | `ESSENTIA_SCRIPT` Python code (extraction) |
| `src/audio.rs` | 146-178 | `run_essentia()` subprocess runner |
| `src/tools.rs` | 2884-2893 | `TrackProfile` struct |
| `src/tools.rs` | 3135-3183 | `build_track_profile()` — reads caches, computes energy |
| `src/tools.rs` | 3185-3209 | `score_transition_profiles()` — 4-axis scoring |
| `src/tools.rs` | 3215-3268 | `score_key_axis()` |
| `src/tools.rs` | 3271-3299 | `score_bpm_axis()` |
| `src/tools.rs` | 3301-3352 | `score_energy_axis()` — phase-aware energy scoring |
| `src/tools.rs` | 3355-3390 | `score_genre_axis()` |
| `src/tools.rs` | 3392-3398 | `priority_weights()` — 4-tuple weight table |
| `src/tools.rs` | 3401-3410 | `composite_score()` — weighted sum |
| `src/tools.rs` | 3412-3441 | `compute_track_energy()` — Essentia → scalar energy |
| `docs/genre-classification-sop.md` | 381-399 | Step C audio-only fallback table |
| `docs/set-builder-sop.md` | 311-339 | Energy scoring documentation |

## Execution Order

1. **Part 1** — Fix OnsetRate indexing in `ESSENTIA_SCRIPT`. Extend integration assertions for descriptor semantics.
2. **Part 2** — Fix danceability normalization in `compute_track_energy()`. Update test fixtures.
3. **Part 2.5 (operational)** — Recompute Essentia cache entries so existing tracks pick up corrected descriptors (for example, run `analyze_audio_batch` with `skip_cached=false` across the intended scope).
4. **Part 3** — Add brightness + rhythm axes to transition scoring. Update composite weights.
5. **Part 4** — Update SOP documents (genre classification fallback table, tie-breaking logic).
6. **Part 5** — Deferred. Document only.

Parts 1+2 are small, safe, and can be a single commit. Part 3 is a medium-sized change touching the scoring pipeline. Part 4 is documentation only.

## Essentia Descriptor Reference

For verification, the authoritative docs:

| Descriptor | Essentia Algorithm | Output index | Range | Docs |
|---|---|---|---|---|
| danceability | `Danceability` | `[0]` (scalar) | 0 to ~3 | [std_Danceability](https://essentia.upf.edu/reference/std_Danceability.html) |
| loudness_integrated | `LoudnessEBUR128` | `[2]` (scalar, LUFS) | ~-70 to 0 | [std_LoudnessEBUR128](https://essentia.upf.edu/reference/std_LoudnessEBUR128.html) |
| loudness_range | `LoudnessEBUR128` | `[3]` (scalar, LU) | 0 to ~20+ | [std_LoudnessEBUR128](https://essentia.upf.edu/reference/std_LoudnessEBUR128.html) |
| onset_rate | `OnsetRate` | `[1]` (scalar, Hz) | 0 to ~15 | [std_OnsetRate](https://essentia.upf.edu/reference/std_OnsetRate.html) |
| dynamic_complexity | `DynamicComplexity` | `[0]` (scalar) | 0 to ~15 | [std_DynamicComplexity](https://essentia.upf.edu/reference/std_DynamicComplexity.html) |
| average_loudness | `Loudness` | scalar | ≥ 0 (Stevens' power law) | [std_Loudness](https://essentia.upf.edu/reference/std_Loudness.html) |
| rhythm_regularity | Computed (not an algorithm) | — | 0 to ~2 | Ratio of downbeat to all-beat energy in lowest band |
| spectral_centroid_mean | `SpectralCentroidTime` | mean of frame values (Hz) | ~200 to ~8000 | [std_SpectralCentroidTime](https://essentia.upf.edu/reference/std_SpectralCentroidTime.html) |
| bpm_essentia | `RhythmExtractor2013` | `[0]` (scalar) | 30 to ~250 | [std_RhythmExtractor2013](https://essentia.upf.edu/reference/std_RhythmExtractor2013.html) |
