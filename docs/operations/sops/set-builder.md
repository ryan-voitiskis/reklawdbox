# Set Builder — Agent SOP

Standard operating procedure for building DJ set sequences from a Rekordbox collection. Agents must follow this document step-by-step.

## Overview

The agent helps the user build ordered tracklists for DJ sets. It uses cached audio analysis and enrichment data to score transitions, sequence tracks, and export the result as a Rekordbox-importable playlist.

**Prerequisites:** Phase 1 genre classification should be substantially complete. Cache coverage should be > 80% across stratum-dsp and enrichment providers. Run `cache_coverage` to verify before starting.

**New tools needed:**

| Tool | Purpose |
|------|---------|
| `score_transition` | Deterministic Camelot/BPM/energy/genre/brightness/rhythm math for a track pair. |
| `build_set` | Greedy sequencing from a candidate pool. Returns ordered candidates with per-transition scores. |
| XML playlist export | Extend `write_xml` to emit `<PLAYLISTS>` section with ordered track references. |

**Existing tools used:**

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
```

### Defaults (if user doesn't specify)

| Parameter | Default |
|-----------|---------|
| Duration | 60 minutes |
| Genre focus | Match user's most common genres |
| BPM range | ±15 BPM from genre median |
| Energy curve | warmup → build → peak → release |
| Priority axis | balanced |
| Starting track | Agent picks based on energy curve (low-energy opener) |
| Exclude | None |

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
  candidates=3
)
```

### How `build_set` works internally (tool spec below)

The tool uses greedy heuristic sequencing:

1. Pick a starting track (user-specified, or lowest-energy track in pool for warmup curves).
2. Score every possible next track from the remaining pool using `score_transition` logic.
3. Apply priority weighting to the composite score.
4. Pick the top-scoring next track.
5. Repeat until target track count or duration is reached.
6. For multiple candidates: vary the starting track or use second/third-best transitions at branch points.

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

| Command | Action |
|---------|--------|
| `swap #5 TrackID` | Replace track at position 5 with a specific track from the pool. Agent re-scores adjacent transitions. |
| `move #5 to #8` | Reorder: move track from position 5 to position 8. Agent re-scores affected transitions. |
| `remove #5` | Remove track at position 5. Agent re-scores the new adjacent pair. |
| `insert TrackID after #5` | Insert a track from the pool after position 5. Agent scores both new transitions. |
| `suggest #5` | Agent suggests the best replacement for position 5 from the pool, scored against positions 4 and 6. |
| `details #5` | Show full data for the track at position 5 (Rekordbox metadata, audio analysis, enrichment). |
| `check` | Re-score and re-display the full set with current edits. |
| `done` | Finalize the set and proceed to export. |

### After each edit, use `score_transition` to validate

```
score_transition(from_track_id="...", to_track_id="...")
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

| Relationship | Score | Label |
|-------------|-------|-------|
| Same key (e.g., 6A → 6A) | 1.0 | Perfect |
| +1 position, same letter (e.g., 6A → 7A) | 0.9 | Energy boost |
| -1 position, same letter (e.g., 6A → 5A) | 0.9 | Energy drop |
| Same number, A↔B (e.g., 6A → 6B) | 0.8 | Mood shift |
| +2 positions, same letter (e.g., 6A → 8A) | 0.5 | Acceptable |
| -2 positions, same letter (e.g., 6A → 4A) | 0.5 | Acceptable |
| +1 position, different letter (e.g., 6A → 7B) | 0.4 | Rough |
| Everything else | 0.1 | Clash |

Camelot wraps: 12A → 1A is +1 (adjacent).

### BPM Compatibility

| Delta | Score | Label |
|-------|-------|-------|
| 0-2 BPM | 1.0 | Seamless |
| 2-4 BPM | 0.8 | Comfortable pitch adjust |
| 4-6 BPM | 0.5 | Noticeable |
| 6-8 BPM | 0.3 | Needs creative transition |
| > 8 BPM | 0.1 | Likely jarring |

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

| Relationship | Score |
|-------------|-------|
| Same canonical genre | 1.0 |
| Related genres (same broad family — see table below) | 0.7 |
| Different families | 0.3 |

Genre families for scoring purposes:

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

| Absolute delta | Score | Label |
|-------------|-------|-------|
| < 300 Hz | 1.0 | Similar brightness |
| 300-800 Hz | 0.7 | Noticeable shift |
| 800-1500 Hz | 0.4 | Large timbral jump |
| > 1500 Hz | 0.2 | Jarring jump |

If either track lacks centroid data, score is reported as `0.5` (neutral unknown).

### Rhythm Compatibility

Rhythm uses Essentia `rhythm_regularity`:

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

| Priority | key_weight | bpm_weight | energy_weight | genre_weight | brightness_weight | rhythm_weight |
|----------|-----------|-----------|---------------|-------------|-------------------|---------------|
| Balanced | 0.30 | 0.20 | 0.18 | 0.17 | 0.08 | 0.07 |
| Harmonic | 0.48 | 0.18 | 0.12 | 0.08 | 0.08 | 0.06 |
| Energy | 0.12 | 0.18 | 0.42 | 0.12 | 0.08 | 0.08 |
| Genre | 0.18 | 0.18 | 0.12 | 0.38 | 0.08 | 0.06 |

---

## Energy Curves

Predefined curves that map track position (as fraction of total) to a target energy phase.

### warmup_build_peak_release (default)

| Position | Phase |
|----------|-------|
| 0% – 15% | Warmup |
| 15% – 45% | Build |
| 45% – 75% | Peak |
| 75% – 100% | Release |

### flat

All positions = Peak (stable high energy throughout).

### peak_only

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
  "priority": "balanced"
}
```

`energy_phase` and `priority` are optional (default: no energy direction preference, balanced weights).

**Response:**

```json
{
  "from": { "track_id": "...", "title": "...", "artist": "...", "key": "6A", "bpm": 122.0, "energy": 0.45, "genre": "Deep House" },
  "to": { "track_id": "...", "title": "...", "artist": "...", "key": "7A", "bpm": 123.5, "energy": 0.52, "genre": "Deep House" },
  "scores": {
    "key": { "value": 0.9, "label": "Energy boost (+1)" },
    "bpm": { "value": 1.0, "label": "Seamless (delta 1.5)" },
    "energy": { "value": 1.0, "label": "Rising (build phase)" },
    "genre": { "value": 1.0, "label": "Same genre" },
    "brightness": { "value": 0.7, "label": "Noticeable brightness shift (delta 420 Hz)" },
    "rhythm": { "value": 1.0, "label": "Matching groove (delta 0.06)" },
    "composite": 0.92
  }
}
```

### `build_set`

Generate candidate set orderings from a track pool.

**Request:**

```json
{
  "track_ids": ["...", "..."],
  "target_tracks": 12,
  "priority": "balanced",
  "energy_curve": "warmup_build_peak_release",
  "start_track_id": "optional",
  "candidates": 3
}
```

**Response:**

```json
{
  "candidates": [
    {
      "id": "A",
      "tracks": [
        { "track_id": "...", "title": "...", "artist": "...", "key": "6A", "bpm": 122.0, "energy": 0.45, "genre": "Deep House" }
      ],
      "transitions": [
        {
          "from_index": 0,
          "to_index": 1,
          "scores": {
            "key": { "value": 0.9, "label": "Energy boost (+1)" },
            "bpm": { "value": 1.0, "label": "Seamless" },
            "energy": { "value": 1.0, "label": "Rising" },
            "genre": { "value": 1.0, "label": "Same genre" },
            "brightness": { "value": 0.7, "label": "Noticeable brightness shift" },
            "rhythm": { "value": 1.0, "label": "Matching groove" },
            "composite": 0.92
          }
        }
      ],
      "set_score": 8.4,
      "estimated_duration_minutes": 68
    }
  ],
  "pool_size": 87,
  "tracks_used": 12
}
```

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
- If the candidate pool is too small (< 1.5x target tracks), warn the user before generating.
- If cache coverage for the pool is low, warn before proceeding.

## Success Criteria

Set builder is complete when:

- [ ] `score_transition` tool implemented with Camelot, BPM, energy, genre, brightness, and rhythm scoring.
- [ ] `build_set` tool implemented with greedy sequencing and multiple candidate support.
- [ ] `write_xml` extended with playlist export.
- [ ] Agent can execute this full SOP end-to-end: parameters → pool → candidates → refine → export.
- [ ] At least one exported playlist successfully imported into Rekordbox.
- [ ] User can swap/reorder/remove tracks and see re-scored transitions interactively.
