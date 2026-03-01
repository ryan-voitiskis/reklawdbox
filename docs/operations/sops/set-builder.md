# Set Builder — Agent SOP

Standard operating procedure for building DJ set sequences from a Rekordbox collection. Agents must follow this document step-by-step.

## Overview

The agent helps the user build ordered tracklists for DJ sets. It uses cached audio analysis and enrichment data to score transitions, sequence tracks, and export the result as a Rekordbox-importable playlist.

**Prerequisites:** Phase 1 genre classification should be substantially complete. Cache coverage should be > 80% across stratum-dsp and enrichment providers. Run `cache_coverage` to verify before starting.

**New tools needed:**

<!-- dprint-ignore -->
| Tool | Purpose |
|------|---------|
| `score_transition` | Deterministic Camelot/BPM/energy/genre/brightness/rhythm math for a track pair. |
| `build_set` | Beam search sequencing from a candidate pool. Returns ordered candidates with per-transition scores. |
| `query_transition_candidates` | Rank pool tracks as transition candidates from a reference. Context-aware (BPM target, energy phase, harmonic style). |
| XML playlist export | Extend `write_xml` to emit `<PLAYLISTS>` section with ordered track references. |

**Existing tools used:**

<!-- dprint-ignore -->
| Tool | Purpose |
|------|---------|
| `search_tracks` | Find candidate pool by genre, BPM, key, playlist, etc. |
| `resolve_tracks_data` | Get full cached evidence for pool tracks. |
| `get_playlists` / `get_playlist_tracks` | Use existing playlists as seed pools. |
| `cache_coverage` | Verify data readiness before set building. |

## Constraints

- **Read-only.** Set building never modifies track metadata (genre, comments, rating, color). It only creates playlist orderings.
- **Human controls the result.** Agent proposes candidates with rationale. User picks, reorders, swaps. Agent exports only what the user approves.
- **Cache-first.** All transition scoring uses cached audio analysis and enrichment. No external API calls during set building.
- **Multiple candidates.** Always present at least 2 candidate orderings unless the pool is too small. Never silently pick a single "best" set.

---

## Step 1: Define Set Parameters

**Goal:** Collect user constraints before building.

### Ask the user

```
Set parameters:
  Duration target?     [e.g., 60 min, 90 min, 2 hours]
  Genre focus?         [e.g., "Deep House + Techno", "any", or a playlist name]
  BPM range?           [e.g., 120-135, or "flexible"]
  Energy curve?        [warmup → peak → cooldown / flat / peak-only / user-defined]
  Priority axis?       [balanced / harmonic / energy / genre]
  Starting track?      [optional — seed track to open with]
  Tracks to exclude?   [optional — IDs or "already played" list]
  Master Tempo?        [on (default) / off — affects key transposition when pitching]
  Harmonic style?      [conservative / balanced (default) / adventurous]
  BPM drift tolerance? [default 6% — max BPM wander from opening track]
  BPM trajectory?      [optional — e.g., "start 122, peak at 130" → bpm_range]
```

### Defaults (if user doesn't specify)

<!-- dprint-ignore -->
| Parameter | Default |
|-----------|---------|
| Duration | 60 minutes |
| Genre focus | Match user's most common genres |
| BPM range | ±15 BPM from genre median |
| Energy curve | warmup → build → peak → release |
| Priority axis | balanced |
| Starting track | Agent picks based on energy curve (low-energy opener) |
| Exclude | None |
| Master Tempo | on |
| Harmonic style | balanced |
| BPM drift tolerance | 6% |
| BPM trajectory | None (no trajectory planning) |

### Configuration notes

- **Master Tempo on** (default) = CDJ/controller pitch-locks the key. BPM changes don't affect harmonic compatibility. Modern default for all digital setups.
- **Master Tempo off** = pitching a track changes its key. Harmonic scoring accounts for the transposition automatically using `round(12 × log₂(target_bpm / native_bpm))` semitone shift.
- **Conservative** harmonic style: only Perfect (1.0), Adjacent ±1 (0.9), and Mood shift A↔B (0.8) pass without heavy penalty (0.1× composite for anything below 0.8). Effectively blocks clashes.
- **Balanced** harmonic style: threshold at 0.45 — Extended (±2) and Energy diagonal pass. Clashes get 0.5× composite penalty.
- **Adventurous** harmonic style: loosens harmonic gates during Build and Peak phases (threshold drops to 0.1). Warmup and Release still enforce 0.45 threshold with 0.5× penalty.
- **BPM trajectory** enables phase-aware BPM ramp: Warmup holds start_bpm, Build ramps linearly to end_bpm, Peak holds end_bpm, Release ramps back toward start_bpm.

### Confirm with user

After collecting parameters, summarize the interpreted constraints back to the user before proceeding to Step 2:

```
Confirmed set parameters:
  Duration:        60 min (~10-12 tracks)
  Genre:           Deep House + Tech House
  BPM:             120-132
  Energy curve:    warmup → build → peak → release
  Priority:        balanced
  Master Tempo:    on
  Harmonic style:  balanced
  BPM drift:       6%
  BPM trajectory:  start 122, peak at 130
  Starting track:  (agent picks)
  Excludes:        none

Proceed? [yes / adjust]
```

### Estimate track count

Assume ~5-7 minutes per track average. For a 60-minute set, target 10-12 tracks. For 2 hours, 18-24.

---

## Step 2: Build Candidate Pool

**Goal:** Assemble a pool of tracks that match the constraints, with full cached data.

### Tool calls

```
search_tracks(genre="Deep House", bpm_min=118, bpm_max=132, limit=200)
```

Or for playlist-based pools:

```
get_playlist_tracks(playlist_id="...")
```

Then resolve full data for the pool:

```
resolve_tracks_data(track_ids=[...all pool IDs...], max_tracks=200)
```

### Filter the pool

Remove tracks from the resolved data where:

- `data_completeness.stratum_dsp` is false (no key/BPM analysis — can't score transitions).
- Track is in the user's exclude list.
- BPM is outside the user's range (after checking `audio_analysis.stratum_dsp.bpm` for the analyzed value).

### Present pool summary to user

```
Candidate pool: 87 tracks

  Genre breakdown:
    Deep House     42
    House          23
    Tech House     12
    Techno         10

  BPM range:  118.0 – 131.5
  Key spread: 11 of 24 Camelot positions represented

  Tracks without Essentia data: 12 (energy scoring will use BPM/loudness proxies)
  Tracks without enrichment: 8 (genre from Rekordbox metadata only)

Proceed with this pool? [yes / adjust filters / add tracks from another playlist]
```

---

## Step 3: Generate Set Candidates

**Goal:** Use the `build_set` tool to produce 2-3 candidate tracklists.

### Tool call

```
build_set(
  track_ids=[...pool IDs...],
  target_tracks=12,
  priority="balanced",
  energy_curve="warmup_build_peak_release",
  start_track_id="optional",
  beam_width=3,
  master_tempo=true,
  harmonic_style="balanced",
  bpm_drift_pct=6.0,
  bpm_range=[122, 130]
)
```

### How `build_set` works internally

The tool uses beam search sequencing:

- `beam_width=1`: greedy single-path (fast, good baseline). Picks the top-scoring next track at each step.
- `beam_width≥2`: beam search exploring N parallel paths, keeping top N by mean composite at each step, deduplicating identical sequences. Default is 3.
- Guidance: use `beam_width=1` for quick previews, 3-5 for final candidates.

Steps:

1. Pick starting track(s) (user-specified, or lowest-energy tracks in pool for warmup curves).
2. For each beam path, score every possible next track from the remaining pool.
3. Apply priority weighting, harmonic gate penalties, and BPM drift penalties to the composite.
4. Expand all beams, sort by mean composite, keep top `beam_width` paths.
5. Repeat until target track count is reached.
6. Deduplicate identical sequences across starting tracks.

The agent does NOT run this loop itself. The tool handles it and returns complete sequences.

### Present candidates to user

```
CANDIDATE A — 12 tracks, ~68 min, score: 8.4/10
  Energy curve: ▁▂▃▄▅▆▇█▇▆▄▂

  #   KEY   BPM     ARTIST              TITLE                    GENRE          TRANSITION
  ─────────────────────────────────────────────────────────────────────────────────────────────
  1   6A    122.0   Artist Name         Track Title              Deep House     (opener)
  2   7A    123.5   Artist Name         Track Title              Deep House     key:+1 bpm:+1.5 ✓
  3   7A    124.0   Artist Name         Track Title              House          key:= bpm:+0.5 ✓
  4   8A    126.0   Artist Name         Track Title              Tech House     key:+1 bpm:+2.0 ✓
  5   8B    127.0   Artist Name         Track Title              Techno         key:A→B bpm:+1.0 ✓
  6   9A    128.5   Artist Name         Track Title              Techno         key:+1 bpm:+1.5 ✓
  7   9A    130.0   Artist Name         Track Title              Techno         key:= bpm:+1.5 ✓
  8   10A   131.0   Artist Name         Track Title              Techno         key:+1 bpm:+1.0 ✓
  9   9A    129.0   Artist Name         Track Title              Tech House     key:-1 bpm:-2.0 ✓
  10  8A    127.5   Artist Name         Track Title              House          key:-1 bpm:-1.5 ✓
  11  7A    125.0   Artist Name         Track Title              Deep House     key:-1 bpm:-2.5 ✓
  12  6A    122.0   Artist Name         Track Title              Deep House     key:-1 bpm:-3.0 ✓

  Transition summary: 10/11 harmonic ✓, 1 mood shift (A→B), avg BPM delta: 1.6

─────────────────────────────────────────────────────────────────────────────────────────────

CANDIDATE B — 12 tracks, ~65 min, score: 7.9/10
  Energy curve: ▁▂▃▅▆▇█▇▆▅▃▂
  [... same table format ...]

Actions:
  pick A              — select candidate A for refinement
  pick B              — select candidate B for refinement
  compare #5          — show transition details for position 5 across candidates
  regenerate          — build new candidates with different seeds
  adjust              — change parameters and rebuild
```

---

## Step 4: Refine Selected Set

**Goal:** Let the user edit the selected candidate interactively.

### Available edit operations

<!-- dprint-ignore -->
| Command | Action |
|---------|--------|
| `swap #5 TrackID` | Replace track at position 5 with a specific track from the pool. Agent re-scores adjacent transitions. |
| `move #5 to #8` | Reorder: move track from position 5 to position 8. Agent re-scores affected transitions. |
| `remove #5` | Remove track at position 5. Agent re-scores the new adjacent pair. |
| `insert TrackID after #5` | Insert a track from the pool after position 5. Agent scores both new transitions. |
| `suggest #5` | Use `query_transition_candidates` to find the best replacement for position 5 from the pool, scored against positions 4 and 6. |
| `details #5` | Show full data for the track at position 5 (Rekordbox metadata, audio analysis, enrichment). |
| `check` | Re-score and re-display the full set with current edits. |
| `done` | Finalize the set and proceed to export. |

### Using `query_transition_candidates` for suggestions

For `suggest #N`, call `query_transition_candidates` with:

- `from_track_id` = track at position N-1
- `pool_track_ids` = remaining pool (minus tracks already in set)
- `energy_phase` = phase at position N
- `target_bpm` = trajectory BPM at position N (if bpm_range set)
- `master_tempo` and `harmonic_style` from session config

Then filter results to also score well against position N+1 (if exists).

### After each edit, use `score_transition` to validate

```
score_transition(
  from_track_id="...",
  to_track_id="...",
  master_tempo=true,
  harmonic_style="balanced"
)
```

### Present edit result

After a swap, show the affected transitions:

```
Swapped #5:
  Before: Track A → [Old Track] → Track C
    key: 8A → 9B → 9A (8A→9B: ✗ non-adjacent)
  After:  Track A → [New Track] → Track C
    key: 8A → 8B → 9A (8A→8B: ✓ mood shift, 8B→9A: ✓ +1)

Set score: 8.4 → 8.6 (+0.2)
```

After `suggest #N`, show top 3 replacement options:

```
Best replacements for position 5 (between 8A@126 and 9A@128.5):

  #  ARTIST              TITLE                    KEY   BPM     FIT SCORE
  ────────────────────────────────────────────────────────────────────────
  1  Artist Name         Track Title              8B    127.0   9.2   ← best harmonic + BPM fit
  2  Artist Name         Track Title              9A    127.5   8.8   ← key match to next, slight BPM jump
  3  Artist Name         Track Title              8A    126.5   8.5   ← same key, smooth BPM

Pick a replacement? [1 / 2 / 3 / keep current]
```

---

## Step 5: Export

**Goal:** Write the finalized set as a Rekordbox playlist in XML.

### Tool call

```
write_xml(
  playlists=[{
    "name": "Set 2026-02-21 — Deep House to Techno",
    "track_ids": ["...", "...", "..."]
  }]
)
```

The XML writer emits both:

- `<COLLECTION>` with all referenced tracks (metadata from Rekordbox DB).
- `<PLAYLISTS>` with a `<NODE Type="1">` containing ordered `<TRACK Key="..."/>` references.

### Present to user

```
Exported: Set 2026-02-21 — Deep House to Techno (12 tracks)
File: ./rekordbox-exports/reklawdbox-20260221-143022.xml

Import in Rekordbox:
  File → Import Collection → select the XML file
  The set will appear under [Imported Library] as a playlist.
```

---

## Transition Scoring Logic

This is the deterministic math that `score_transition` and `build_set` implement. Documented here so the agent understands what the scores mean.

### Key Compatibility (Camelot Wheel)

Camelot positions are numbered 1-12 with A (minor) and B (major) variants.

<!-- dprint-ignore -->
| Relationship | Score | Label |
|-------------|-------|-------|
| Same key (e.g., 6A → 6A) | 1.0 | Perfect |
| +1 position, same letter (e.g., 6A → 7A) | 0.9 | Camelot adjacent (+1) |
| -1 position, same letter (e.g., 6A → 5A) | 0.9 | Camelot adjacent (-1) |
| Same number, A↔B (e.g., 6A → 6B) | 0.8 | Mood shift (A↔B) |
| ±1 position, different letter (e.g., 6A → 7B) | 0.55 | Energy diagonal |
| ±2 positions, same letter (e.g., 6A → 8A) | 0.45 | Extended (+/-2) |
| Everything else | 0.1 | Clash |

Camelot wraps: 12A → 1A is +1 (adjacent).

### BPM Compatibility

BPM scoring uses a continuous exponential curve based on percentage difference:

```
score = exp(-0.019 × pct²)
```

where `pct = |from_bpm - to_bpm| / from_bpm × 100`.

This gives a smooth "closer is better" curve with no artificial step boundaries:

<!-- dprint-ignore -->
| pct | Score | Label |
|-----|-------|-------|
| 0.0% | 1.000 | Seamless |
| 1.0% | 0.981 | Seamless |
| 2.0% | 0.927 | Comfortable |
| 3.0% | 0.843 | Comfortable |
| 5.0% | 0.621 | Noticeable |
| 8.0% | 0.297 | Creative transition needed |
| 12.0% | 0.064 | Jarring |

Label brackets: < 2% → Seamless, < 4% → Comfortable, < 6% → Noticeable, < 9% → Creative transition needed, ≥ 9% → Jarring.

Use the stratum-dsp analyzed BPM (more accurate than Rekordbox metadata). Fall back to Rekordbox BPM if no analysis cached.

### Energy Compatibility

Energy is estimated from cached Essentia features when available:

```
normalized_dance = clamp(danceability / 3.0, 0, 1)
normalized_loudness = clamp((loudness_integrated + 30) / 30, 0, 1)
onset_rate_normalized = clamp(onset_rate / 10.0, 0, 1)
energy_score = (0.4 * normalized_dance) + (0.3 * normalized_loudness) + (0.3 * onset_rate_normalized)
```

Where:

- `danceability`: raw Essentia value (typical range 0 to ~3).
- `normalized_loudness`: `(loudness_integrated + 30) / 30`, clamped to 0-1 (maps typical -30..0 LUFS range).
- `onset_rate_normalized`: `onset_rate / 10.0`, clamped to 0-1 (typical range 0-10 onsets/sec).

If Essentia is not available, fall back to a BPM-based energy proxy:

```
energy_proxy = (bpm - 95) / 50, clamped to 0-1
```

This is crude but directionally correct (higher BPM ≈ higher energy for dance music).

Energy transition scoring depends on the requested energy curve position:

<!-- dprint-ignore -->
| Curve phase | Desired direction | Score if met | Score if wrong direction |
|-------------|-------------------|-------------|------------------------|
| Warmup | energy stable or slight rise | 1.0 | 0.5 |
| Build | energy rising | 1.0 | 0.3 |
| Peak | energy high and stable | 1.0 | 0.5 |
| Release | energy dropping | 1.0 | 0.3 |

Phase-aware loudness-range bonuses:

- If the set crosses a phase boundary and destination track `loudness_range > 8.0`, add `+0.1` to the energy axis (cap 1.0).
- If phase stays in Peak and destination track `loudness_range < 4.0`, add `+0.05` (cap 1.0).

### Genre Compatibility

<!-- dprint-ignore -->
| Relationship | Score |
|-------------|-------|
| Same canonical genre | 1.0 |
| Related genres (same broad family — see table below) | 0.7 |
| Different families | 0.3 |

Genre families for scoring purposes:

<!-- dprint-ignore -->
| Family | Genres |
|--------|--------|
| House | House, Deep House, Tech House, Afro House, Garage, Speed Garage |
| Techno | Techno, Deep Techno, Minimal, Dub Techno, Ambient Techno, Hard Techno, Acid, Electro |
| Bass | Drum & Bass, Jungle, Dubstep, Breakbeat, UK Bass, Grime, Bassline, Broken Beat |
| Downtempo | Ambient, Downtempo, Dub, IDM, Experimental |
| Other | Hip Hop, Disco, Trance, Psytrance, Pop, R&B, Reggae, Dancehall, Rock, Synth-pop |

Tracks within the "Other" family don't get the 0.7 related-genre bonus with each other (too diverse).

### Brightness Compatibility

Brightness uses Essentia `spectral_centroid_mean` (Hz):

<!-- dprint-ignore -->
| Absolute delta | Score | Label |
|-------------|-------|-------|
| < 300 Hz | 1.0 | Similar brightness |
| 300-800 Hz | 0.7 | Noticeable shift |
| 800-1500 Hz | 0.4 | Large timbral jump |
| > 1500 Hz | 0.2 | Jarring jump |

If either track lacks centroid data, score is reported as `0.5` (neutral unknown).

### Rhythm Compatibility

Rhythm uses Essentia `rhythm_regularity`:

<!-- dprint-ignore -->
| Absolute delta | Score | Label |
|-------------|-------|-------|
| < 0.10 | 1.0 | Matching groove |
| 0.10-0.25 | 0.7 | Manageable shift |
| 0.25-0.50 | 0.4 | Challenging shift |
| > 0.50 | 0.2 | Groove clash |

If either track lacks regularity data, score is reported as `0.5` (neutral unknown).

### Composite Score

```
weighted_sum = Σ(weight_i * score_i for available axes)
composite = weighted_sum / Σ(weight_i for available axes)
```

For brightness/rhythm specifically: when descriptor data is missing, the axis remains visible in output as `0.5` + `Unknown ...`, but that axis is excluded from the composite denominator so missing Essentia data does not penalize the transition score.

Weights by priority axis:

<!-- dprint-ignore -->
| Priority | key_weight | bpm_weight | energy_weight | genre_weight | brightness_weight | rhythm_weight |
|----------|-----------|-----------|---------------|-------------|-------------------|---------------|
| Balanced | 0.30 | 0.20 | 0.18 | 0.17 | 0.08 | 0.07 |
| Harmonic | 0.48 | 0.18 | 0.12 | 0.08 | 0.08 | 0.06 |
| Energy | 0.12 | 0.18 | 0.42 | 0.12 | 0.08 | 0.08 |
| Genre | 0.18 | 0.18 | 0.12 | 0.38 | 0.08 | 0.06 |

### Master Tempo and Pitch Shift

When **Master Tempo is off**, pitching a track to match BPMs also shifts its key. The scoring system accounts for this automatically.

**Pitch shift formula:**

```
semitones = round(12 × log₂(target_bpm / native_bpm))
```

**Camelot transposition:** Each semitone = +7 positions mod 12 on the Camelot wheel. The letter (A/B) is unchanged.

**Worked example:**

- Track at 135 BPM, key 8A. Played at 128 BPM.
- `semitones = round(12 × log₂(128/135)) = round(12 × -0.076) = round(-0.91) = -1`
- `-1 semitone = -7 Camelot positions = (8-1-7) mod 12 + 1 = 0 mod 12 + 1 = 1` → **1A**
- Harmonic scoring uses the effective key 1A instead of native 8A.

**Master Tempo on** (default): key is unchanged regardless of pitch. `effective_key` is always the native key.

### Post-Composite Adjustments

After the weighted composite is computed, several context-dependent adjustments may apply. These are surfaced via the `adjustments` array on each transition's scores (only present when non-empty).

**Harmonic gate** (style-dependent penalty on composite):

<!-- dprint-ignore -->
| Style | Threshold | Penalty when key < threshold |
|-------|-----------|------------------------------|
| Conservative | 0.8 | 0.1× composite |
| Balanced | 0.45 | 0.5× composite |
| Adventurous (Warmup/Release) | 0.45 | 0.5× composite |
| Adventurous (Build/Peak) | 0.1 | 0.5× composite |

**BPM drift penalty** (0.7× composite): Applied in `build_set` when a candidate's BPM drift from the opening track exceeds the position-proportional budget. Budget = `bpm_drift_pct × (position / max_position)`.

**Genre stickiness** (axis-level, reflected in composite):

- **Streak bonus** (+0.1 on genre axis): staying in the same genre family for 1-4 consecutive transitions.
- **Early switch penalty** (-0.1 on genre axis): switching genre family after only 1 transition in the current family.

**Phase-aware energy bonuses** (axis-level, reflected in composite):

- **Phase boundary boost** (+0.1 on energy axis): destination track has `loudness_range > 8.0` at a phase boundary.
- **Sustained peak boost** (+0.05 on energy axis): staying in Peak phase with destination track `loudness_range < 4.0` (tight, controlled energy).

Each adjustment entry includes:

- `kind`: identifier (e.g., `"harmonic_gate"`, `"bpm_drift"`, `"genre_streak"`)
- `delta`: signed impact on composite
- `composite_without`: what the composite would have been without this adjustment
- `reason`: human-readable explanation

---

## Energy Curves

Predefined curves that map track position (as fraction of total) to a target energy phase.

### warmup_build_peak_release (default)

<!-- dprint-ignore -->
| Position | Phase |
|----------|-------|
| 0% – 15% | Warmup |
| 15% – 45% | Build |
| 45% – 75% | Peak |
| 75% – 100% | Release |

### flat

All positions = Peak (stable high energy throughout).

### peak_only

<!-- dprint-ignore -->
| Position | Phase |
|----------|-------|
| 0% – 10% | Build |
| 10% – 85% | Peak |
| 85% – 100% | Release |

### User-defined

User specifies phases as a list: `["warmup", "warmup", "build", "build", "peak", "peak", "peak", "peak", "release", "release"]` — one entry per track position.

---

## New Tool Specs

### `score_transition`

Score a single transition between two tracks.

**Request:**

```json
{
  "from_track_id": "...",
  "to_track_id": "...",
  "energy_phase": "build",
  "priority": "balanced",
  "master_tempo": true,
  "harmonic_style": "balanced"
}
```

`energy_phase`, `priority`, `master_tempo`, and `harmonic_style` are optional. Defaults: no energy phase, balanced priority, master tempo on, balanced harmonic style.

**Response:**

```json
{
  "from": {
    "track_id": "...",
    "title": "...",
    "artist": "...",
    "key": "6A",
    "bpm": 122.0,
    "energy": 0.45,
    "genre": "Deep House"
  },
  "to": {
    "track_id": "...",
    "title": "...",
    "artist": "...",
    "key": "7A",
    "bpm": 123.5,
    "energy": 0.52,
    "genre": "Deep House"
  },
  "scores": {
    "key": { "value": 0.9, "label": "Camelot adjacent (+1)" },
    "bpm": { "value": 0.972, "label": "Seamless (1.2%, 1.5 BPM)" },
    "energy": { "value": 1.0, "label": "Rising (build phase)" },
    "genre": { "value": 1.0, "label": "Same genre" },
    "brightness": {
      "value": 0.7,
      "label": "Noticeable brightness shift (delta 420 Hz)"
    },
    "rhythm": { "value": 1.0, "label": "Matching groove (delta 0.06)" },
    "composite": 0.958
  },
  "key_relation": "Camelot adjacent (+1)",
  "bpm_adjustment_pct": 1.23,
  "effective_to_key": "3A",
  "pitch_shift_semitones": -1,
  "adjustments": [
    {
      "kind": "harmonic_gate",
      "delta": -0.328,
      "composite_without": 0.656,
      "reason": "Key score 0.10 below Conservative threshold 0.80 — 0.1x penalty"
    }
  ]
}
```

`effective_to_key` and `pitch_shift_semitones` only present when Master Tempo is off and pitch shift is non-zero. `adjustments` only present when non-empty.

### `query_transition_candidates`

Rank pool tracks as transition candidates from a reference track.

**Request:**

```json
{
  "from_track_id": "...",
  "pool_track_ids": ["...", "...", "..."],
  "energy_phase": "build",
  "priority": "balanced",
  "master_tempo": true,
  "harmonic_style": "balanced",
  "target_bpm": 126.0,
  "limit": 10
}
```

Alternatively, use `playlist_id` instead of `pool_track_ids` to pull candidates from a playlist. `target_bpm` is optional — when set, BPM scoring uses `target_bpm` as the reference instead of the source track's BPM. `limit` defaults to 10 (max 50).

**Response:**

```json
{
  "from": {
    "track_id": "...",
    "title": "...",
    "artist": "...",
    "native_bpm": 124.0,
    "key": "8A",
    "energy": 0.55,
    "genre": "House"
  },
  "reference_bpm": 126.0,
  "master_tempo": true,
  "candidates": [
    {
      "track_id": "...",
      "title": "...",
      "artist": "...",
      "native_bpm": 125.5,
      "native_key": "9A",
      "bpm_difference_pct": 0.4,
      "key_relation": "Camelot adjacent (+1)",
      "scores": {
        "key": { "value": 0.9, "label": "Camelot adjacent (+1)" },
        "bpm": { "value": 0.997, "label": "Seamless (0.4%, 0.5 BPM)" },
        "energy": { "value": 1.0, "label": "Rising (build phase)" },
        "genre": { "value": 1.0, "label": "Same genre" },
        "brightness": { "value": 0.5, "label": "Unknown brightness" },
        "rhythm": { "value": 0.5, "label": "Unknown groove" },
        "composite": 0.953
      },
      "play_at_bpm": 126.0,
      "pitch_adjustment_pct": 0.4
    }
  ],
  "total_pool_size": 45
}
```

### `build_set`

Generate candidate set orderings from a track pool using beam search.

**Request:**

```json
{
  "track_ids": ["...", "..."],
  "target_tracks": 12,
  "priority": "balanced",
  "energy_curve": "warmup_build_peak_release",
  "start_track_id": "optional",
  "beam_width": 3,
  "master_tempo": true,
  "harmonic_style": "balanced",
  "bpm_drift_pct": 6.0,
  "bpm_range": [122, 130]
}
```

`beam_width` controls search breadth (default 3, max 8). `bpm_range` is optional `[start_bpm, end_bpm]` for trajectory planning.

**Response:**

```json
{
  "candidates": [
    {
      "id": "A",
      "tracks": [
        {
          "track_id": "...",
          "title": "...",
          "artist": "...",
          "key": "6A",
          "bpm": 122.0,
          "energy": 0.45,
          "genre": "Deep House",
          "play_at_bpm": 122.0,
          "pitch_adjustment_pct": 0.0
        }
      ],
      "transitions": [
        {
          "from_index": 0,
          "to_index": 1,
          "scores": {
            "key": { "value": 0.9, "label": "Camelot adjacent (+1)" },
            "bpm": { "value": 0.972, "label": "Seamless (1.2%, 1.5 BPM)" },
            "energy": { "value": 1.0, "label": "Rising (build phase)" },
            "genre": { "value": 1.0, "label": "Same genre" },
            "brightness": {
              "value": 0.7,
              "label": "Noticeable brightness shift"
            },
            "rhythm": { "value": 1.0, "label": "Matching groove" },
            "composite": 0.958,
            "adjustments": []
          },
          "key_relation": "Camelot adjacent (+1)",
          "bpm_adjustment_pct": 1.23
        }
      ],
      "set_score": 8.4,
      "estimated_duration_minutes": 68,
      "bpm_trajectory": [
        122.0,
        122.0,
        123.6,
        125.3,
        127.0,
        128.6,
        130.0,
        130.0,
        130.0,
        128.0,
        126.0,
        124.0
      ]
    }
  ],
  "pool_size": 87,
  "tracks_used": 12,
  "beam_width": 3,
  "bpm_trajectory": [
    122.0,
    122.0,
    123.6,
    125.3,
    127.0,
    128.6,
    130.0,
    130.0,
    130.0,
    128.0,
    126.0,
    124.0
  ]
}
```

Per-track fields `play_at_bpm`, `pitch_adjustment_pct`, and `effective_key` are only present when `bpm_range` is set. `effective_key` only appears when Master Tempo is off and pitch shift is non-zero.

### XML Playlist Export (extension to `write_xml`)

Extend the existing `write_xml` tool to accept an optional `playlists` parameter.

**Extended request:**

```json
{
  "output_path": "optional",
  "playlists": [
    {
      "name": "Set 2026-02-21 — Deep House to Techno",
      "track_ids": ["id1", "id2", "id3"]
    }
  ]
}
```

When `playlists` is present, the XML output includes:

```xml
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.10" Company="AlphaTheta"/>
  <COLLECTION Entries="12">
    <TRACK TrackID="..." ... />
    ...
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="1">
      <NODE Type="1" Name="Set 2026-02-21 — Deep House to Techno" Entries="12" KeyType="0">
        <TRACK Key="id1"/>
        <TRACK Key="id2"/>
        <TRACK Key="id3"/>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
```

Track order in the `<NODE>` matches the order in `track_ids`. The `<COLLECTION>` includes all tracks referenced by any playlist plus any tracks with staged metadata changes.

---

## Guardrails

- No metadata changes during set building. Set building is read-only on track data.
- Always present multiple candidates. Never auto-pick a single "best" set.
- Always show per-transition scores so the user understands why tracks are adjacent.
- If any transition has `adjustments`, narrate them to the user (e.g., "transition 4→5 was penalized for key clash").
- If the candidate pool is too small (< 1.5x target tracks), warn the user before generating.
- If cache coverage for the pool is low, warn before proceeding.

## Success Criteria

Set builder is complete when:

- [x] `score_transition` tool implemented with Camelot, BPM, energy, genre, brightness, and rhythm scoring.
- [x] `build_set` tool implemented with beam search sequencing and multiple candidate support.
- [x] `query_transition_candidates` tool implemented for interactive position replacement.
- [x] `write_xml` extended with playlist export.
- [x] Agent can execute this full SOP end-to-end: parameters → pool → candidates → refine → export.
- [ ] At least one exported playlist successfully imported into Rekordbox.
- [x] User can swap/reorder/remove tracks and see re-scored transitions interactively.
- [x] Master Tempo on/off with pitch-aware key transposition.
- [x] Harmonic style (conservative/balanced/adventurous) with per-style penalty factors.
- [x] BPM trajectory planning via `bpm_range` parameter.
- [x] Post-composite adjustments surfaced via `adjustments` array.
- [x] BPM scoring uses smooth exponential curve (no step artifacts).
- [x] Scoring evaluation harness with synthetic pool quality gates.
