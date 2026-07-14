# Genre Classification: Implementation Spec

> **Historical implementation snapshot:** This document predates the
> 2026-07-14 Beatport removal, source-group confidence rules, and versioned
> profile state. Preserve it as research context, but use Plans 034–035 and the
> live classifier as the current contract.

**Date:** 2026-04-10
**Prereq:** Read [genre-classification-improvements.md](genre-classification-improvements.md) for full research context and empirical data.

### Design Validation: Fisher Dry-Run Results

Fisher discriminant scoring was computed against the full 574-track verified playlist (5 features: danceability, spectral_centroid_mean, dynamic_complexity, rhythm_regularity, BPM). Key findings:

**Works well for clearly distinct genres:**
- House: centroid (w=0.46) + danceability (0.24) + BPM (0.23)
- Ambient: danceability (w=0.62) + centroid (0.25)
- Techno: dynamic_complexity (w=0.52) + BPM (0.46)
- Dancehall: BPM (w=0.78) — very slow (83 vs 117)
- Hip Hop: BPM (w=0.70) — slow (95 vs 118)

**Weaker for subtle boundaries:**
- Dub Techno: primarily danceability (w=0.67). Centroid is not a strong one-vs-all discriminator because Ambient also has low centroid, pulling the "other" mean down.
- Elvism test case: Dub Techno ranks **5th** (vote weight 0.219). Techno (0.302), Electro (0.300), Deep House (0.284), Deep Techno (0.280) score higher because Elvism's danceability (1.37) is far from the Dub Techno mean (2.2).

**Implications:**
- Fisher one-vs-all works as a **supplementary signal**, not a primary classifier. It correctly identifies the strongest features per genre and produces useful votes for well-separated genres.
- For subtle boundaries (Dub Techno vs Deep Techno), audio profiles nudge but don't drive. Enrichment data and LLM subagent knowledge remain essential.
- Wiring `decay_mid_tau` (the strongest validated Dub Techno discriminator not yet available) should improve the Dub Techno boundary specifically.
- `AFFINITY_CAP` should stay conservative at 0.5. Audio profile votes should augment, not override.
- Future refinement: one-vs-nearest-neighbor Fisher scores would better separate similar genres but add complexity. Defer until one-vs-all proves insufficient in practice.

---

## Overview

Six work items, ordered by dependency. Each is independently testable.

| # | Work Item | Files | Complexity |
|---|---|---|---|
| 1 | Deterministic tie-breaking | `classify.rs` | Small |
| 2 | Candidates array UX | `classify.rs`, `classify_handler.rs` | Small |
| 3 | Ambient veto expansion | `classify.rs` | Small |
| 4 | Family tiebreak + centroid | `classify.rs` | Medium |
| 5 | Feature wiring | `classify.rs`, `classify_handler.rs` | Medium |
| 6 | Genre Audio Profiles | `classify.rs`, `audio_profile.rs` (new), `store.rs`, tool handler | Large |

Items 1-3 can be done in parallel. Item 4 should precede 5 (both touch AudioProfile). Item 6 depends on 5.

---

## Item 1: Deterministic Tie-Breaking

### What

The HashMap→Vec→sort in `find_consensus` and `build_candidates` produces non-deterministic genre ordering when scores are tied. Fix with a stable 3-key sort.

### Where

`src/classify.rs` — two sites:
- `find_consensus` (line ~549-555): widen tally to `HashMap<&str, (f32, bool)>`, 3-key sort
- `build_candidates` (line ~1145-1149): add tiebreak keys to sort

### Changes

**`find_consensus`** — replace:
```rust
let mut tally: HashMap<&'static str, f32> = HashMap::new();
for v in votes { *tally.entry(v.genre).or_default() += v.weight; }
let mut ranked: Vec<(&'static str, f32)> = tally.into_iter().collect();
ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
```

With:
```rust
let mut tally: HashMap<&'static str, (f32, bool)> = HashMap::new();
for v in votes {
    let entry = tally.entry(v.genre).or_insert((0.0, true));
    entry.0 += v.weight;
    if !v.bpm_plausible { entry.1 = false; }
}
let mut ranked: Vec<(&'static str, f32, bool)> = tally
    .into_iter()
    .map(|(g, (w, p))| (g, w, p))
    .collect();
ranked.sort_by(|a, b| {
    b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.2.cmp(&a.2))   // plausible before implausible
        .then_with(|| a.0.cmp(b.0))    // alphabetical
});
```

Update all downstream tuple destructures in `find_consensus` (~6 sites):
- `ranked[0]`: `(mut top_genre, top_score)` → `(mut top_genre, top_score, _)`
- `ranked.get(1)` destructure: `(second_genre, _)` → `(second_genre, _, _)`
- `total_weight` map: `|(_, w)| w` → `|(_, w, _)| w`
- BPM-override find: `|(g, _)| ...` → `|(g, _, _)| ...`
- `alt_genre` destructure: same pattern
- Shallower check: `|(g, _)| *g == shallower` → `|(g, _, _)| *g == shallower`

**`build_candidates`** — add tiebreak keys:
```rust
candidates.sort_by(|a, b| {
    b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.bpm_plausible.cmp(&a.bpm_plausible))
        .then_with(|| a.genre.cmp(b.genre))
});
```

### Tests

Add test: two genres tied at same score → result is deterministic across 10 runs. Clean up the `collection_pod_ghost_4way_split_insufficient` test assertion message that acknowledges the bug.

---

## Item 2: Candidates Array UX

### What

Include the chosen genre in the candidates array with a `chosen: bool` marker. Rename dispatch format's `suggested_genre` to `genre`. Add candidates to dispatch output.

### Where

- `src/classify.rs` — `GenreCandidate` struct, `build_candidates` function
- `src/tools/classify_handler.rs` — dispatch format JSON builder

### Changes

**`GenreCandidate`** — add field:
```rust
#[serde(skip_serializing_if = "is_false")]
pub(crate) chosen: bool,
```

With helper: `fn is_false(b: &bool) -> bool { !*b }`

**`build_candidates`** — stop filtering out `top_genre`. Instead mark it `chosen: true`. Sort chosen first, then by score desc. Truncate to 4 (chosen + 3 alternatives).

**Dispatch format** in `classify_handler.rs`:
- Rename `"suggested_genre"` to `"genre"`
- Add `"candidates": r.candidates` to the dispatch track JSON

### Tests

Verify chosen genre appears with `chosen: true`, alternatives without. Verify BPM-override case shows chosen genre with lower score than a non-chosen bpm-implausible entry.

---

## Item 3: Ambient Veto Expansion

### What

The current ambient veto requires `NonDancefloor + Ambient` (dc > 10.0). All 5 verified ambient tracks have dc between 5.49-7.86, below this threshold. Add a second veto branch using the existing `Atmospheric` flag (dc > 5.0).

### Where

`src/classify.rs` — `check_audio_vetoes` function (line ~296)

### Changes

Add after the existing `NonDancefloor + Ambient` veto:
```rust
// Expanded: NonDancefloor + Atmospheric (dc > 5.0) — catches ambient tracks
// below the dc > 10.0 threshold. Lower confidence than the Ambient veto.
if profile.bucket == EnergyBucket::NonDancefloor
    && has_flag(profile, CharFlag::Atmospheric)
    && !has_flag(profile, CharFlag::Ambient) // don't double-fire
{
    // → Ambient, Low confidence
}
```

Also fix `rhythm_regularity` default: change `unwrap_or(0.85)` to `unwrap_or(0.0)` in `compute_audio_profile`. Validate against existing tests carefully — this affects Broken/Irregular flag assignment.

### Tests

Track with danceability 0.9, dc 6.0 (NonDancefloor + Atmospheric but not Ambient) → should now veto to Ambient, Low. Previously would have fallen through.

---

## Item 4: Family Tiebreak + Centroid on AudioProfile

### What

Promote `spectral_centroid_mean` to `AudioProfile`. Expand `audio_clearly_favors_family` with centroid-based checks. Fix the both-candidates correctness bug.

### Where

`src/classify.rs` — `AudioProfile` struct, `compute_audio_profile`, `audio_clearly_favors_family`, `find_consensus` tiebreak call site

### Changes

**AudioProfile** — add field:
```rust
centroid: Option<f64>,
```

Populate in `compute_audio_profile`:
```rust
centroid: audio.spectral_centroid_mean,
```

**`audio_clearly_favors_family`** — expand two families:

Downtempo:
```rust
GenreFamily::Downtempo => {
    let very_low_centroid = profile.centroid.is_some_and(|c| c < CENTROID_VERY_LOW);
    (profile.bucket == EnergyBucket::LowEnergy && has_flag(profile, CharFlag::Atmospheric))
        || (profile.bucket == EnergyBucket::LowEnergy && very_low_centroid)
        || (profile.bucket == EnergyBucket::NonDancefloor && very_low_centroid)
}
```

Techno:
```rust
GenreFamily::Techno => {
    let dark_timbre = profile.centroid.is_some_and(|c| c < CENTROID_DARK);
    (profile.bucket >= EnergyBucket::Dancefloor && !has_flag(profile, CharFlag::Broken) && profile.bpm >= 125.0)
        || (profile.bucket == EnergyBucket::LowEnergy && !has_flag(profile, CharFlag::Broken)
            && profile.bpm >= 118.0 && profile.bpm <= 132.0 && dark_timbre)
}
```

Constants (align with audio-only inference D.3):
```rust
const CENTROID_VERY_LOW: f64 = 600.0;
const CENTROID_DARK: f64 = 1200.0;
```

**Critical: Both-candidates check** — replace the tiebreak call site (line ~695-706):
```rust
} else {
    if let Some(profile) = audio_profile.as_ref() {
        let top_favored = audio_clearly_favors_family(profile, top_genre);
        let second_genre = second.expect("second exists").0;  // 3-tuple from Item 1
        let second_favored = audio_clearly_favors_family(profile, second_genre);
        if top_favored && !second_favored {
            flags.push("audio-assisted-tiebreak".into());
            ClassificationConfidence::Low
        } else if second_favored && !top_favored {
            top_genre = second_genre;
            flags.push("audio-assisted-tiebreak".into());
            ClassificationConfidence::Low
        } else {
            ClassificationConfidence::Insufficient
        }
    } else {
        ClassificationConfidence::Insufficient
    }
}
```

### Tests

- Elvism-like evidence (Ambient vs Minimal tie, centroid 264) → both families pass → Insufficient (both-candidates guard)
- Dub Techno vs Deep House tie (centroid 900, LowEnergy, 124bpm) → Techno family passes, House doesn't → Dub Techno wins
- Track with no centroid data → falls back to existing behavior (centroid checks return false)

---

## Item 5: Feature Wiring

### What

Plumb cached audio features into `AudioFeatures` for Genre Audio Profiles and evidence strings. Feature selection informed by correlation analysis and statistical validation (see research doc).

**This item must complete before Item 6 calibration.**

### Feature Selection Rationale

Statistical analysis of ~52 available features identified:
- **Dead features** (drop): `intensity_mean` (always -1.0), `intensity_var` (always 0.0)
- **Redundant features** (drop): `average_loudness` (redundant with `loudness_integrated`), `loudness_range` (redundant with `dynamic_complexity`)
- **Diminishing returns** (drop): MFCC coefficients 9-12, MFCC std coefficients 6-12, spectral contrast bands 1/3/5
- **Independent features** (keep): danceability/onset_rate/rhythm_regularity are genuinely independent rhythm features

MFCCs and spectral contrast are multi-dimensional (13+13+6 = 32 dims). At sample sizes of 5-17 tracks per genre, using them as independent features is statistically unsafe. Instead, collapse to **timbral distances** (computed at scoring time in Item 6, not here).

### Scalar Features to Add to AudioFeatures

| Feature | Source | Independent of | Why |
|---|---|---|---|
| `decay_mid_tau` | Stratum | All Essentia features | Reverb decay — validated Ambient separator |
| `decay_high_tau` | Stratum | Partially corr. with mid_tau | High-freq reverb — ratio to mid encodes reverb color |
| `onset_rate` | Essentia | danceability, rhythm_reg | Event density — independent rhythm dimension |
| `loudness_integrated` | Essentia | dynamic_complexity | Overall level (LUFS) — mastering style |
| `spectral_centroid_cv` | Essentia | spectral_centroid_mean | Brightness variability — independent |
| `spectral_flux_mean` | Essentia | All others | Rate of spectral change |
| `dissonance_mean` | Essentia | Most others | Harmonic roughness |
| `key_clarity` | Stratum | Most others | Tonalness |

### Vector Features to Add to AudioFeatures (for timbral distance in Item 6)

| Feature | Source | Dimensions | Used as |
|---|---|---|---|
| `mfcc_mean` | Essentia | 13 (use indices 1-8, drop 0 and 9-12) | Timbral centroid distance |
| `mfcc_std` | Essentia | 13 (use indices 1-5, drop 0 and 6-12) | Timbral variability distance |
| `spectral_contrast_mean` | Essentia | 6 (use bands 0, 2, 4) | Spectral shape distance |

These are stored as `Option<Vec<f64>>` on AudioFeatures. They are NOT used as independent Fisher features — they are collapsed to per-genre distance scalars at scoring time (Item 6).

### Where

- `src/classify.rs` — `AudioFeatures` struct
- `src/tools/classify_handler.rs` — `extract_audio_features` function

### Changes

**AudioFeatures** — add scalar fields:
```rust
pub(crate) decay_mid_tau: Option<f64>,
pub(crate) decay_high_tau: Option<f64>,
pub(crate) onset_rate: Option<f64>,
pub(crate) loudness_integrated: Option<f64>,
pub(crate) spectral_centroid_cv: Option<f64>,
pub(crate) spectral_flux_mean: Option<f64>,
pub(crate) dissonance_mean: Option<f64>,
pub(crate) key_clarity: Option<f64>,
```

**AudioFeatures** — add vector fields (for timbral distances):
```rust
pub(crate) mfcc_mean: Option<Vec<f64>>,      // 13 coefficients
pub(crate) mfcc_std: Option<Vec<f64>>,        // 13 coefficients
pub(crate) spectral_contrast_mean: Option<Vec<f64>>,  // 6 bands
```

**`extract_audio_features`** — read from cache:
```rust
// Scalar features from Stratum
decay_mid_tau: stratum_json.as_ref()
    .and_then(|sj| sj.get("decay_mid_tau")).and_then(Value::as_f64),
decay_high_tau: stratum_json.as_ref()
    .and_then(|sj| sj.get("decay_high_tau")).and_then(Value::as_f64),
key_clarity: stratum_json.as_ref()
    .and_then(|sj| sj.get("key_clarity")).and_then(Value::as_f64),

// Scalar features from Essentia
onset_rate: essentia_data.as_ref().and_then(|e| e.onset_rate),
loudness_integrated: essentia_data.as_ref().and_then(|e| e.loudness_integrated),
spectral_centroid_cv: essentia_data.as_ref().and_then(|e| e.spectral_centroid_cv),
spectral_flux_mean: essentia_data.as_ref().and_then(|e| e.spectral_flux_mean),
dissonance_mean: essentia_data.as_ref().and_then(|e| e.dissonance_mean),

// Vector features from Essentia (for timbral distances in Item 6)
mfcc_mean: essentia_data.as_ref().and_then(|e| e.mfcc_mean.clone()),
mfcc_std: essentia_data.as_ref().and_then(|e| e.mfcc_std.clone()),
spectral_contrast_mean: essentia_data.as_ref()
    .and_then(|e| e.spectral_contrast_mean.clone()),
```

Update all `AudioFeatures` struct literals in tests to include the new fields with `None`.

---

## Item 6: Genre Audio Profiles (Fisher Discriminant Scoring)

### What

Build a scoring engine that computes per-genre audio affinity from calibrated prototypes. Prototypes are built from a verified playlist using Fisher discriminant weights. Scores inject as votes into the existing `gather_votes` pipeline.

### Statistical Constraints (from validation)

These constraints are derived from analysis of feature correlations, sample sizes, and Fisher score reliability. See research doc for full detail.

| Constraint | Rule | Rationale |
|---|---|---|
| Feature count per genre | Max N/5 (N = verified tracks) | Curse of dimensionality |
| Fisher scoring minimum | N >= 10 | Fisher scores unreliable below this |
| Variance regularization | Always when N < 50 | Prevents overfitting on small samples |
| Fisher weight cap | Max 0.4 per feature | Prevents single-feature dominance |
| Correlated features | Do not use as independent dimensions | Double-counts information |
| MFCCs | Collapse to timbral distances, not 13 independent features | 13 correlated dims with N=17 is unsafe |

### Two-Tier Feature Design

**Tier 1 — Scalar features (all genres with N >= 5):**

13 independent scalar features. Used directly in Fisher scoring.

| Feature | Source | Correlation group | Independent of |
|---|---|---|---|
| `rekordbox_bpm` | DB | Rhythm | Everything except onset_rate (weak) |
| `danceability` | Essentia | Rhythm | onset_rate, rhythm_regularity (confirmed independent) |
| `onset_rate` | Essentia | Rhythm | danceability (confirmed independent) |
| `rhythm_regularity` | Essentia | Rhythm | danceability, onset_rate (confirmed independent) |
| `spectral_centroid_mean` | Essentia | Brightness | centroid_cv (independent) |
| `spectral_centroid_cv` | Essentia | Brightness variability | centroid_mean (independent) |
| `dynamic_complexity` | Essentia | Dynamics | loudness_integrated (independent) |
| `loudness_integrated` | Essentia | Energy level | dynamic_complexity (independent) |
| `decay_mid_tau` | Stratum | Reverb | Most Essentia features |
| `decay_high_tau` | Stratum | Reverb (high band) | Moderate corr with mid_tau |
| `spectral_flux_mean` | Essentia | Spectral change | Most others |
| `dissonance_mean` | Essentia | Harmonic roughness | Most others |
| `key_clarity` | Stratum | Tonalness | Most others |

**Tier 2 — Timbral distances (genres with N >= 10):**

3 additional features computed as per-genre Euclidean distances to genre centroids. Plus 2 genre-independent aggregate stats.

| Feature | Computed from | What it captures |
|---|---|---|
| `mfcc_timbral_dist` | MFCC mean coefficients 1-8 | Overall timbral similarity to genre |
| `mfcc_variability_dist` | MFCC std coefficients 1-5 | Timbral dynamics match |
| `spectral_contrast_dist` | Spectral contrast bands 0, 2, 4 | Spectral shape match |
| `mfcc_slope` | Linear regression across MFCC mean 1-13 | Spectral tilt (genre-independent) |
| `mfcc_std_mean` | Mean of MFCC std values | Timbral stability (genre-independent) |

The 3 distance features are genre-specific (different value per candidate genre). The 2 aggregate stats are genre-independent (computed once per track). Total Tier 2: 5 additional scoring dimensions.

**Per-genre feature budget:**

| Genre size (N) | Tier | Features used | Examples |
|---|---|---|---|
| N >= 50 | 1 + 2 | Up to 18 | House (145), Deep House (140), Ambient (87), Techno (50) |
| 10 <= N < 50 | 1 + 2 | Up to 18 | Electro (32), Hip Hop (31), Deep Techno (20), Dub Techno (17) |
| 5 <= N < 10 | 1 only | Up to 13 (cap at N/5) | Disco (11), Dancehall (10), D&B (5) |
| N < 5 | None | No prototype | Trance (3), Garage (3) — too few |

### Architecture

```
genre_verified playlist (574 tracks, verified by ear)
        ↓
calibrate_audio_profiles() MCP tool
        ↓
Per-genre prototypes stored in SQLite:
    Scalar: {genre, feature, mean, stddev, fisher_weight, n_verified}
    Timbral centroids: {genre, mfcc_centroid[8], mfcc_std_centroid[5], contrast_centroid[3]}
        ↓
At classification time:
    score_audio_profiles(track_features, registry) → Vec<AudioAffinity>
        ↓
    Top N affinities → GenreVote entries in gather_votes()
```

### New File: `src/audio_profile.rs`

Core types:
```rust
pub(crate) struct FeatureStat {
    pub(crate) mean: f64,
    pub(crate) stddev: f64,
    pub(crate) fisher_weight: f64,  // auto-computed, not manual
    pub(crate) n: u32,
}

pub(crate) struct GenrePrototype {
    pub(crate) genre: &'static str,
    pub(crate) features: HashMap<&'static str, FeatureStat>,
    /// MFCC mean centroid (coefficients 1-8) for timbral distance
    pub(crate) mfcc_centroid: Option<Vec<f64>>,
    /// MFCC std centroid (coefficients 1-5) for variability distance
    pub(crate) mfcc_std_centroid: Option<Vec<f64>>,
    /// Spectral contrast centroid (bands 0, 2, 4) for shape distance
    pub(crate) contrast_centroid: Option<Vec<f64>>,
    pub(crate) total_n: u32,
}

pub(crate) struct ProfileRegistry {
    pub(crate) prototypes: HashMap<&'static str, GenrePrototype>,
    /// Global stats for regularization fallback
    pub(crate) global_stats: HashMap<&'static str, (f64, f64)>,  // (mean, std)
}

pub(crate) struct AudioAffinity {
    pub(crate) genre: &'static str,
    pub(crate) distance: f64,
    pub(crate) vote_weight: f32,
    pub(crate) contributions: Vec<FeatureContribution>,
}

pub(crate) struct FeatureContribution {
    pub(crate) name: &'static str,
    pub(crate) track_value: f64,
    pub(crate) proto_mean: f64,
    pub(crate) z_score: f64,
    pub(crate) fisher_weight: f64,
}
```

### Calibration Algorithm

```rust
fn calibrate(verified_tracks: &[(genre, AudioFeatures)]) -> ProfileRegistry {
    // 1. Normalize all features globally (zero mean, unit variance)
    // 2. Group tracks by genre
    // 3. For each genre with N >= 5:
    //    a. For each scalar feature: compute mean and population stddev
    //    b. Regularize variance: var_reg = alpha(N) * var_sample + (1-alpha) * var_global
    //       where alpha(N) = (N-1) / (N-1 + num_features/2)
    //    c. Fisher score = (mean_genre - mean_global)^2 / (var_genre_reg + var_global)
    //    d. Cap Fisher weights at 0.4, floor at 0.02, normalize to sum to 1.0
    //    e. If N >= 10: compute timbral centroids (MFCC, contrast)
    //    f. Limit active features to min(N/5, total_available)
    // 4. Store prototypes + global stats
}
```

Minimum 5 verified tracks per genre (raised from 3). Below this, variance/mean estimates are too unreliable even with regularization.

### Scoring Function

```rust
fn score_track(audio: &AudioFeatures, proto: &GenrePrototype, global: &GlobalStats) -> AudioAffinity {
    // Tier 1: Scalar features
    // For each scalar feature with fisher_weight > 0 and both values present:
    //   z = (track_value - proto.mean) / max(proto.stddev, global_std * 0.1)
    //   contribution = fisher_weight * z^2

    // Tier 2: Timbral distances (if genre has centroids)
    //   mfcc_dist = euclidean_distance(track.mfcc_mean[1..8], proto.mfcc_centroid)
    //   Normalize by within-genre mean distance, treat as a z-score
    //   Same for mfcc_std_dist and contrast_dist
    //   Plus genre-independent: mfcc_slope, mfcc_std_mean

    // Combined distance = sqrt(sum of all contributions)
    // Confidence penalty for small genres: distance += 0.5 * sqrt(num_features / N)
    // vote_weight = max(0.0, AFFINITY_CAP * (1.0 - distance / SCALE))
}
```

Constants:
- `AFFINITY_CAP: f32 = 0.5` — below Beatport (1.0) and label (0.6). Conservative.
- `SCALE: f64 = 2.5` — tracks > 2.5 weighted stddev from prototype score 0.
- `MIN_TRACKS: u32 = 5` — minimum verified tracks to generate a prototype.
- `FISHER_WEIGHT_CAP: f64 = 0.4` — no single feature dominates.
- `FISHER_WEIGHT_FLOOR: f64 = 0.02` — mild regularization, no feature fully ignored.

### Vote Injection

In `gather_votes`, after existing vote sources:
```rust
if let Some(registry) = audio_profile::global_registry() {
    let affinities = audio_profile::score_all(audio, registry);
    for a in affinities {
        if a.vote_weight < 0.05 { continue; }
        votes.push(GenreVote {
            genre: a.genre,
            weight: a.vote_weight,
            source: "audio-profile",
            bpm_plausible: bpm_plausible(a.genre, effective_bpm),
        });
    }
}
```

### Evidence Strings

```
audio-profile: Dub Techno 0.42 (centroid=536~573 [F=3.2], dance=2.5~2.7 [F=2.1])
```

Only emit for genres with vote_weight >= 0.1. Include top 3 features by contribution.

### SQLite Schema

Add to `store.rs` migration (bump schema version):
```sql
CREATE TABLE IF NOT EXISTS genre_audio_profiles (
    genre         TEXT NOT NULL,
    feature       TEXT NOT NULL,
    mean          REAL NOT NULL,
    stddev        REAL NOT NULL,
    fisher_weight REAL NOT NULL,
    n_verified    INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (genre, feature)
);
```

### MCP Tool: `calibrate_audio_profiles`

```
calibrate_audio_profiles(playlist_id: "genre_verified")
→ Reads playlist tracks
→ Pulls audio features from cache
→ Computes prototypes with Fisher weights
→ Stores in SQLite
→ Returns: per-genre summary (n tracks, top discriminating features, prototype confidence)
```

### Registry Initialization

Use `OnceLock<ProfileRegistry>` pattern (same as `genre_alias_map` in `genre.rs`). Loaded from SQLite at startup. Recalibration requires restart or explicit `reload_registry()`.

### Known Limitations (from dry-run validation)

**Fisher one-vs-all is weak for similar genres.** The Fisher score compares each genre against the global average. When two genres share a distinctive feature (e.g., Dub Techno and Ambient both have low centroid), neither gets a high Fisher weight for that feature because the "other" distribution includes the similar genre.

**Practical impact:** Dub Techno's Fisher weights are dominated by danceability (0.67). Centroid, which cleanly separates Dub Techno from Minimal in our 17-track validation, gets low Fisher weight because Ambient (87 tracks) also has low centroid. The Elvism test case ranks Dub Techno 5th at vote weight 0.219.

**Mitigations built into the design:**
- `AFFINITY_CAP = 0.5` means audio profiles augment but never override enrichment
- `decay_mid_tau` (wired in Item 5) adds a feature where Dub Techno genuinely differs from both Ambient and other Techno variants
- The existing decision tree (Fixes 1, 3, 4, 5) handles the tiebreaking that Fisher scores are weak on
- LLM subagent reasoning fills the gap for edge cases like Elvism where artist context matters

**Future refinement:** One-vs-nearest-neighbor Fisher scores would compute discriminative weights relative to each genre's closest competitors rather than the global average. This would give centroid high weight for Dub Techno (because its nearest neighbor Minimal has high centroid) even though Ambient also has low centroid. Defer this complexity until one-vs-all proves insufficient in production use.

### Verification Feedback (future)

A `suggest_verification_candidates(n=50)` tool that picks tracks where:
- Audio-profile score disagrees most with current genre tag
- Genres with fewest verified examples
- Tracks near prototype boundaries

Not in the MVP — implement after the core scoring works.

---

## Portable Genre Model: The Core Value Proposition

### Why This Matters

The genre audio profiles are not just for one user's collection. They're a **portable genre model** that ships in the reklawdbox binary. The target use case:

> A DJ with a large untagged collection runs `analyze_audio_batch` + `classify_tracks` and gets 70-80%+ accurate genre tags with zero manual work. They correct the few mistakes manually. This saves hours of tedious tagging work.

This is the core value of reklawdbox.

### What Gets Shipped

```
Compiled into binary (or shipped as data file, ~50KB):
├── Genre prototypes: 40 genres × ~15 features × (mean, std, fisher_weight)
├── Timbral centroids per genre (MFCC, spectral contrast vectors)
├── Global normalization parameters (mean/std per feature from training set)
└── BPM ranges per genre (already in genre.rs)
```

### Normalization for Portability

Prototypes are trained on high-quality WAV/FLAC files. Other users may have MP3s, different mastering, different loudness. Features must be collection-independent:

- Normalize features to z-scores using the **training set's** global mean/std
- Ship those normalization parameters alongside the prototypes
- At classification time, apply the same normalization to the user's features
- Relative distances are preserved even with systematic shifts (e.g., all tracks 3dB louder)

### Training Data Strategy

**Phase 1: Verified collection (current)**
User ear-verifies tracks in Rekordbox, adds to `genre_verified` playlist. System calibrates from this.

**Phase 2: Canonical downloads (for gap genres)**
Download 50-100 tracks per genre that define the genre. Analyze, verify, add to training set. Priority genres: Minimal, IDM, Dubstep, Jungle.

**Phase 3: Convergence loop**
Use the model to classify the full collection. User reviews and corrects. Confirmed tracks join the training set. Repeat — each round improves the model.

**Phase 4: Ship**
Prototypes compiled into binary. All users benefit. Users who disagree with specific genre boundaries correct manually — most will be happy with 90%+ accuracy.

### Model Versioning

Prototypes are versioned alongside the code. When the training set improves:
1. Recalibrate locally (SQLite)
2. Validate accuracy against the verified set
3. Export prototypes to source (e.g., `src/audio_profile_data.rs` or `data/genre_prototypes.toml`)
4. Ship in next release

### Two Storage Modes

| Mode | Location | Purpose |
|---|---|---|
| **Development** | SQLite (`genre_audio_profiles` table) | Local calibration, iterative refinement |
| **Release** | Compiled into binary (static data) | Shipped to all users, read-only |

At runtime: if SQLite has calibrated prototypes, use those (user's local refinements). Otherwise fall back to compiled prototypes (shipped defaults).

---

## Verified Playlist: `genre_verified`

597 tracks verified by ear (as of 2026-04-10). Genre distribution:

| Genre | Count | Prototype viable? |
|---|---|---|
| House | ~145 | Strong |
| Deep House | ~140 | Strong |
| Ambient | ~87 | Strong |
| Techno | ~50 | Strong |
| Electro | ~32 | Strong |
| Hip Hop | ~31 | Strong |
| Deep Techno | ~20 | Good |
| Breakbeat | ~19 | Good (newly added) |
| Dub Techno | ~17 | Good |
| Disco | ~11 | Adequate |
| Dancehall | ~10 | Adequate |
| Trance | ~6-12 | Adequate (newly expanded) |
| Drum & Bass | ~5 | Minimum |
| Downtempo | ~5 | Minimum |
| Minimal | ~4-7 | Minimum (newly expanded, needs canonical downloads) |
| Garage | ~3 | Below minimum |
| Others | ~15 | Sparse |

### Gaps Requiring Canonical Downloads

| Genre | Current verified | Target | Action |
|---|---|---|---|
| Minimal | ~5 | 30+ | Download Perlon, Minus, Kompakt classics |
| IDM | ~2 | 30+ | Download Warp, Rephlex, Planet Mu classics |
| Dubstep | ~2 | 20+ | Download Tempa, DMZ, Deep Medi classics |
| Jungle | ~3 | 15+ | Download Moving Shadow, Reinforced classics |
| Garage | ~3 | 15+ | Download Ghost, Tectonic classics |

---

## Empirical Reference Data

### Validated Feature Ranges (17 ear-verified tracks)

| Feature | Dub Techno (n=5) | Deep Techno (n=5) | Minimal (n=2) | Ambient (n=5) |
|---|---|---|---|---|
| **Danceability** | 1.99–3.36 | 1.65–1.97 | 1.91–1.92 | 0.84–1.39 |
| **Centroid (Hz)** | 375–1060 | 708–1890 | 1074–1521 | 263–1432 |
| **Dyn complexity** | 3.78–6.06 | 2.22–5.35 | 3.58–3.96 | 5.49–7.86 |
| **decay_mid_tau (ms)** | 68–226 | 117–323 | 47–82 | 172–1045 |
| **Rhythm reg** | 0.61–1.27 | 0.44–1.10 | 0.63–1.10 | 0.79–1.14 |
| **Onset rate (/s)** | 3.80–4.90 | 4.70–8.34 | 4.87–6.18 | 1.22–3.94 |

### Clean Separators

| Pair | Feature | Gap |
|---|---|---|
| Ambient vs Dub Techno | Danceability | 0.60 (1.39 max ambient → 1.99 min dub techno) |
| Dub Techno vs Minimal | Centroid | 274 Hz (771 max dub techno* → 1074 min minimal) |
| Ambient vs dancefloor | decay_mid_tau | Near-clean at 300ms threshold |

*Excluding Monolake outlier at 1060

### Invalidated Features (do not use)

`mod_centroid`, `harmonic_proportion`, `bpm_confidence`, `grid_stability` — total overlap across all tested genres. See research doc Appendix A for full data.
