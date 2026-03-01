---
title: Transition Scoring
description: The math behind key, BPM, energy, genre, brightness, and rhythm compatibility scoring.
sidebar:
  order: 3
---

## Overview

Transitions between tracks are scored on six independent axes, each producing a value between 0 and 1. A weighted composite combines all available scores based on the chosen priority mode.

Both `score_transition` and `build_set` use this scoring system. `score_transition` returns the full breakdown for a single A-to-B pair. `build_set` uses it internally to evaluate thousands of candidate transitions during greedy sequencing.

The six axes are:

1. **Key** — harmonic compatibility via the Camelot wheel
2. **BPM** — tempo difference
3. **Energy** — energy level direction relative to the set's energy curve phase
4. **Genre** — genre family relationships
5. **Brightness** — spectral centroid similarity (requires Essentia)
6. **Rhythm** — onset regularity similarity (requires Essentia)

Axes 5 and 6 require Essentia audio analysis. When that data is missing, those axes are excluded from the composite without penalizing the score.

---

## Key compatibility (Camelot wheel)

Key relationships are evaluated on the Camelot wheel, where adjacent positions represent harmonically compatible keys.

| Relationship               | Score | Label        |
| -------------------------- | ----- | ------------ |
| Same key (6A to 6A)        | 1.0   | Perfect      |
| +1 same letter (6A to 7A)  | 0.9   | Energy boost |
| -1 same letter (6A to 5A)  | 0.9   | Energy drop  |
| Same number A/B (6A to 6B) | 0.8   | Mood shift   |
| +2 same letter (6A to 8A)  | 0.5   | Acceptable   |
| -2 same letter (6A to 4A)  | 0.5   | Acceptable   |
| +1 other letter (6A to 7B) | 0.4   | Rough        |
| Everything else            | 0.1   | Clash        |

Camelot wraps at the boundary: 12A to 1A is +1 (adjacent), not +11.

The +1/-1 moves are the bread and butter of harmonic mixing. Same-number A/B shifts switch between major and minor (or vice versa) — useful for mood changes within the same harmonic center. The +2/-2 moves are usable but require more careful phrasing.

---

## BPM compatibility

BPM delta is the absolute difference in tempo between two tracks.

| Delta   | Score | Label                     |
| ------- | ----- | ------------------------- |
| 0-2 BPM | 1.0   | Seamless                  |
| 2-4 BPM | 0.8   | Comfortable pitch adjust  |
| 4-6 BPM | 0.5   | Noticeable                |
| 6-8 BPM | 0.3   | Needs creative transition |
| > 8 BPM | 0.1   | Likely jarring            |

reklawdbox uses stratum-dsp analyzed BPM when available, which is more accurate than Rekordbox's built-in analysis. Falls back to Rekordbox metadata BPM if no stratum-dsp analysis is cached.

The thresholds assume both tracks will be pitch-adjusted to match. A 4 BPM gap means one or both decks need noticeable pitch fader movement. Beyond 8 BPM, you typically need to use techniques like looping, echo-out, or hard cuts rather than a smooth blend.

---

## Energy compatibility

Energy is a composite value estimated from Essentia audio features.

### Energy calculation

```
normalized_dance    = clamp(danceability / 3.0, 0, 1)
normalized_loudness = clamp((loudness_integrated + 30) / 30, 0, 1)
onset_rate_norm     = clamp(onset_rate / 10.0, 0, 1)

energy = (0.4 * normalized_dance)
       + (0.3 * normalized_loudness)
       + (0.3 * onset_rate_norm)
```

When Essentia data is not available, a BPM-based proxy is used:

```
energy_proxy = clamp((bpm - 95) / 50, 0, 1)
```

### Phase-aware scoring

Energy scoring depends on where you are in the energy curve. The desired energy direction changes by phase:

| Phase   | Desired direction     | Score if met | Score if wrong |
| ------- | --------------------- | ------------ | -------------- |
| Warmup  | Stable or slight rise | 1.0          | 0.5            |
| Build   | Rising                | 1.0          | 0.3            |
| Peak    | High and stable       | 1.0          | 0.5            |
| Release | Dropping              | 1.0          | 0.3            |

### Loudness-range bonuses

Additional bonuses apply at phase boundaries based on the destination track's `loudness_range` value:

- **Phase boundary** with destination `loudness_range > 8.0`: +0.1 bonus (capped at 1.0). Tracks with wide dynamic range create natural-sounding transitions at phase boundaries.
- **Peak phase** with destination `loudness_range < 4.0`: +0.05 bonus (capped at 1.0). Compressed, loud tracks sustain energy during the peak.

---

## Genre compatibility

Genres are grouped into families. Compatibility depends on whether two genres share the same family.

| Relationship                 | Score |
| ---------------------------- | ----- |
| Same canonical genre         | 1.0   |
| Related genres (same family) | 0.7   |
| Different families           | 0.3   |

### Genre families

| Family    | Genres                                                                                                  |
| --------- | ------------------------------------------------------------------------------------------------------- |
| House     | House, Deep House, Tech House, Afro House, Gospel House, Progressive House, Garage, Speed Garage, Disco |
| Techno    | Techno, Deep Techno, Minimal, Dub Techno, Ambient Techno, Hard Techno, Drone Techno, Acid, Electro      |
| Bass      | Drum & Bass, Jungle, Dubstep, Breakbeat, UK Bass, Grime, Bassline, Broken Beat                          |
| Downtempo | Ambient, Downtempo, Dub, Dub Reggae, IDM, Experimental                                                  |
| Other     | Hip Hop, Trance, Psytrance, Pop, R&B, Reggae, Dancehall, Rock, Synth-pop, Highlife, Jazz                |

Genres within the "Other" family do **not** receive the 0.7 related-genre bonus with each other. They score 0.3 against all other genres, including other "Other" entries.

Genre matching uses the canonical genre assigned by reklawdbox's classification system, not the raw genre string from Rekordbox metadata. If a track hasn't been classified yet, the raw metadata genre is normalized to the closest canonical genre.

---

## Brightness compatibility (spectral centroid)

Brightness is derived from the spectral centroid frequency, measured in Hz. A higher spectral centroid means more high-frequency content — think bright, crispy hi-hats versus deep, muffled pads. Large jumps in brightness create timbral clashes that are especially noticeable in long blends.

| Delta (Hz) | Score | Label              |
| ---------- | ----- | ------------------ |
| < 300      | 1.0   | Similar brightness |
| 300-800    | 0.7   | Noticeable shift   |
| 800-1500   | 0.4   | Large timbral jump |
| > 1500     | 0.2   | Jarring            |

When brightness data is missing for either track, this axis scores 0.5 (neutral) and is excluded from the composite denominator so it does not penalize the overall score.

---

## Rhythm compatibility (regularity)

Rhythm regularity measures how "on the grid" a track feels — a four-on-the-floor techno beat has high regularity, while a broken-beat track or live-drummer recording has low regularity. Delta is the absolute difference in regularity between two tracks.

| Delta     | Score | Label             |
| --------- | ----- | ----------------- |
| < 0.10    | 1.0   | Matching groove   |
| 0.10-0.25 | 0.7   | Manageable shift  |
| 0.25-0.50 | 0.4   | Challenging shift |
| > 0.50    | 0.2   | Groove clash      |

When rhythm data is missing for either track, this axis scores 0.5 (neutral) and is excluded from the composite denominator.

---

## Composite score

The final transition score is a weighted sum normalized by the weights of available axes:

```
composite = sum(weight_i * score_i) / sum(weight_i)    for available axes
```

Axes with missing data (brightness, rhythm) are excluded from both the numerator and denominator. This means a transition between two tracks without Essentia analysis is scored purely on key, BPM, energy (proxy), and genre — without penalty.

### Weight presets

Four priority modes control how axes are weighted:

| Priority | Key  | BPM  | Energy | Genre | Brightness | Rhythm |
| -------- | ---- | ---- | ------ | ----- | ---------- | ------ |
| Balanced | 0.30 | 0.20 | 0.18   | 0.17  | 0.08       | 0.07   |
| Harmonic | 0.48 | 0.18 | 0.12   | 0.08  | 0.08       | 0.06   |
| Energy   | 0.12 | 0.18 | 0.42   | 0.12  | 0.08       | 0.08   |
| Genre    | 0.18 | 0.18 | 0.12   | 0.38  | 0.08       | 0.06   |

- **Balanced** — general-purpose. Good default for most sets.
- **Harmonic** — prioritizes key compatibility. Best for melodic genres where clashing keys are obvious.
- **Energy** — prioritizes energy flow. Best when building a set around a specific energy curve.
- **Genre** — prioritizes genre cohesion. Best for sets that should stay within a genre family.

Note that brightness and rhythm always have low weights (0.06-0.08) across all presets. They act as tiebreakers when the primary axes are similar, not as dominant factors.

### Interpreting scores

| Range     | Quality                                                   |
| --------- | --------------------------------------------------------- |
| 0.85-1.0  | Excellent transition — mix with confidence                |
| 0.70-0.84 | Good transition — minor compromises on one or two axes    |
| 0.50-0.69 | Acceptable — requires skill or creative mixing techniques |
| 0.30-0.49 | Difficult — likely noticeable to the audience             |
| < 0.30    | Avoid unless intentionally jarring                        |

These ranges are guidelines. A 0.65 score with a perfect key match and a genre clash plays differently than a 0.65 with mediocre scores across the board. Use `score_transition` to see the per-axis breakdown when a composite score surprises you.

---

## Data requirements

The scoring system degrades gracefully based on available data:

| Data available          | Axes used                                  |
| ----------------------- | ------------------------------------------ |
| Rekordbox metadata only | Key, BPM, genre                            |
| + stratum-dsp analysis  | Key, BPM (improved), energy (proxy), genre |
| + Essentia analysis     | All six axes                               |

Run `analyze_audio_batch` on your tracks before building sets for the best scoring accuracy. Without Essentia, brightness and rhythm are excluded entirely, and energy falls back to the BPM-based proxy.
