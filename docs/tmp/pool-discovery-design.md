# Pool Discovery Design

Design spec for track pool discovery features (issue #25).

## Terminology

**Pool** — an unordered set of tracks with high mutual compatibility. The raw
material. Created by `expand_pool` from seeds, or surfaced by `discover_pools`
later. A pool has no sequence, no commitment. It's a working scratchpad.

**Chapter** — a locked pool that the DJ has validated. Two variants:

- **Sequenced chapter** — 2-12 tracks in a specific order. The DJ has tested
  the transitions and locked the sequence. Typically 2-6 tracks in practice.
- **Unordered chapter** — a tight cluster where any permutation works. The DJ
  trusts the internal compatibility enough to improvise the order live. Locked
  means "these tracks belong together," not "in this order."

**Bridge** — 1-3 tracks that connect two chapters. High compatibility with both
the tail of the outgoing chapter and the head of the incoming one (or any
member, for unordered chapters). The set builder finds these.

**Set plan** — the full structure: chapter order, bridges between them, overall
energy arc. Produced by the Chapter Set Planning SOP.

## Architecture

### Symmetric pool-compatibility kernel

A new `score_pool_compatibility` function in `scoring.rs` that answers "do these
tracks belong in the same crate?" — as opposed to `score_transition` which
answers "should track B follow track A at this point in the set?"

Key properties:

- **Symmetric**: `score(A, B) == score(B, A)` always. Tracks in a pool can be
  played in either direction — there is no "from" and "to."
- **No sequential context**: no energy phases, no genre streaks, no BPM drift
  budget. The optional `reference_bpm` is a pool-level parameter (affects how
  keys are computed), not positional context within a set.
- **Reuses `TrackProfile`**: same profile struct, extended with timbral fields.

### What's different from the transition scorer

| Concern | Transition scorer | Pool kernel |
|---------|-------------------|-------------|
| Directionality | A→B ≠ B→A (phase, streaks) | Symmetric always |
| Energy | Phase-conditioned (warmup wants +delta, build wants increasing, etc.) | Absolute distance (similar energy band = high score) |
| Genre | Run-length streak bonuses/penalties | Simple match only (same=1.0, same family=0.7, different=0.3) |
| BPM drift | Cumulative penalty from set start BPM | None — only pairwise BPM distance |
| Key pitch correction | Rounds to integer semitone, transposes, no detuning penalty (has cliff bug) | Continuous detuning model with smooth penalty (see below) |
| Harmonic gate | Post-composite penalty that crushes bad key matches | None — key is just one weighted axis |
| Timbral similarity | Not used | Concatenated feature vector distance (new axis) |

### BPM source

Pool tools always use the **Rekordbox BPM** (`track.bpm`), never
Essentia/stratum-dsp analysis values. The existing `build_track_profile`
already prefers Rekordbox BPM and only falls back to stratum-dsp when
Rekordbox BPM is below 30.0 (i.e. missing). The pool kernel inherits this
behavior unchanged — the DJ's corrected BPM is the source of truth.

### Axis design

**BPM** — Reuse existing Gaussian decay on percentage difference:
`exp(-0.019 × pct²)`. Already symmetric.

| BPM diff | Score |
|----------|-------|
| 0% | 1.00 |
| 2% | 0.93 |
| 4% | 0.74 |
| 6% | 0.50 |
| 8% | 0.30 |

BPM scoring is always pairwise between two tracks. In `expand_pool`'s
iterative greedy algorithm, each candidate is scored against every current
pool member individually. The min-compatibility ranking naturally catches bad
BPM pairs — a candidate whose BPM is 8% from any single pool member will have
a low min-score regardless of proximity to other members.

**Key** — Reuse Camelot distance scoring with a continuous detuning model.
Master-tempo-aware:

- **Master tempo ON**: score native Camelot keys directly. No pitch correction.
- **Master tempo OFF**: compute each track's effective key at a reference BPM
  using the continuous detuning model (see below), then score.

**Energy** — Replace phase-conditioned scoring with absolute energy distance,
Gaussian decay. Tracks at similar energy levels score high. No directional
delta.

**Genre** — Drop streak bonuses/penalties. Same genre = 1.0, same family = 0.7,
different family = 0.3.

**Brightness** — Reuse spectral centroid delta thresholds. Already symmetric.

**Rhythm** — Reuse rhythm regularity delta thresholds. Already symmetric.

**Timbral (new)** — Euclidean distance on a concatenated, z-score-normalized
feature vector:

- MFCC mean (13-dim): overall spectral envelope / timbral color
- MFCC std (13-dim): timbral variation within the track (uniform vs diverse
  sections). Already stored in Essentia cache as `mfcc_std`.
- Spectral contrast mean (per-band): frequency band energy distribution
- Spectral centroid CV: brightness variation over time
- Dissonance mean: harmonic roughness / consonance

Total: ~35 dimensions. Each dimension z-score-normalized across the library
before computing distance. Normalization stats recomputed when library changes
significantly (>10% new tracks).

Excluded from composite if either track lacks Essentia data (same graceful
degradation pattern brightness/rhythm use).

onset_rate and rhythm_regularity are NOT included in the timbral axis — they
are rhythmic features handled by the existing brightness and rhythm axes.

**Partial analysis coverage:** tracks without Essentia data are scored on
non-timbral axes only (BPM, key, genre). The composite is renormalized, so
they are not penalized for missing data. This means unanalyzed tracks can
rank higher than they deserve by avoiding potential low timbral scores — a
tradeoff, not a bug. The `analysis_coverage` field in `describe_pool`
surfaces this. The agent should recommend running full Essentia analysis
before pool building for best results.

### Continuous detuning model (master tempo off)

The existing transition scorer has a cliff bug in key scoring when
master_tempo=false: it rounds pitch shifts to the nearest integer semitone,
then transposes the Camelot key. Because 1 chromatic semitone = 7 positions on
the Camelot wheel (circle of fifths), this creates a brutal discontinuity:

- 2.9% BPM diff → 0.49 semitones → rounds to 0 → "Perfect" (1.0)
- 3.1% BPM diff → 0.51 semitones → rounds to 1 → 7-position jump → "Clash" (0.1)

The pool kernel fixes this with bilinear interpolation between bracketing
integer transpositions:

1. Compute the fractional semitone shift for each track:
   `shift = 12 × log2(reference_bpm / native_bpm)`
2. Find the two bracketing integer transpositions (floor and ceil) with
   interpolation weights based on the fractional part
3. Transpose the Camelot key by each integer shift using
   `transpose_camelot_key` (semitones × 7 mod 12 on the Camelot wheel)
4. Score all four key combinations (from-floor × to-floor, from-floor ×
   to-ceil, from-ceil × to-floor, from-ceil × to-ceil) via `score_key_axis`
5. Blend scores via bilinear interpolation using the fractional weights

This directly models perceptual ambiguity: a 25-cent detuning makes the key
genuinely ambiguous between two Camelot positions, so the score blends between
the two possible key relationships. For a same-key pair at 25 cents:

    0.75 × score(same_key) + 0.25 × score(7-position jump)
    = 0.75 × 1.0 + 0.25 × 0.1 = 0.775

| Cents | Same-key pair score | Rationale |
|-------|---------------------|-----------|
| 0 | 1.00 | No ambiguity |
| 10 | 0.91 | Mostly resolved to native key |
| 25 | 0.775 | Significant ambiguity |
| 40 | 0.46 | Nearly equal weight on both keys |
| 50 | 0.55 | Maximum ambiguity (equal blend) |

The implementation uses `score_key_with_pitch_shifts` and `bracketed_keys`
(see `scoring.rs`). The pool kernel reuses the same model already used by
the transition scorer.

### Reference BPM and harmonic stability (master tempo off)

With master tempo off, pitching a track to match BPM shifts its key
proportionally — just like vinyl. The shift is
`12 × log2(target_bpm / native_bpm)` semitones:

| BPM diff | Pitch shift |
|----------|-------------|
| 1% | ~0.17 semitones (inaudible) |
| 3% | ~0.51 semitones (noticeable detuning) |
| 5% | ~0.85 semitones (key relationship degrading) |
| 8% | ~1.33 semitones (key relationship broken) |

**Why a reference BPM matters:**

Without a reference BPM, each pair is evaluated in isolation — "if A matches
B's BPM, what happens to A's key?" But in a pool, the DJ plays tracks in
sequence, and each track is pitched to match whatever is currently playing.
The reference BPM shifts as the DJ moves through the pool, making harmonic
relationships unstable.

Example — 3 tracks all tagged 8A:

| Track | Native BPM | Key |
|-------|-----------|-----|
| A | 126 | 8A |
| B | 130 | 8A |
| C | 134 | 8A |

Pairwise, A↔B and B↔C each have ~0.5 semitone shifts (manageable). But A↔C
has a 1.06 semitone shift — effectively a different key. The pool looks
harmonically coherent pairwise but breaks for the widest pair.

With a locked reference BPM of 130:
- A at 130: +0.54 semitones sharp of 8A
- B at 130: native 8A
- C at 130: -0.52 semitones flat of 8A
- A↔C effective gap: 1.06 semitones — immediately visible

More importantly, the choice of reference BPM can change WHICH harmonic
relationships exist. Tracks with different native keys may converge or diverge
harmonically depending on the reference BPM. A pool that looks like a clean
Camelot progression at native BPMs might collapse into a tight cluster at a
locked BPM, or vice versa.

**How the tools handle this:**

All pool tools accept `master_tempo: bool` (default `false`).

When `master_tempo = true`: reference BPM is irrelevant. Keys are fixed. No
pitch correction needed.

When `master_tempo = false`:

- `score_pool_compatibility` accepts an optional `reference_bpm`. When
  provided, all tracks' effective keys are computed at that BPM using the
  continuous detuning model. When omitted, defaults to the **median BPM** of
  the tracks being scored.
- `expand_pool` accepts `reference_bpm`. Defaults to median BPM of seeds.
  Candidates' effective keys are evaluated at the reference BPM before
  scoring.
- `describe_pool` accepts `reference_bpm` and additionally reports the
  **optimal reference BPM** — sweeps BPMs across the pool's range and finds
  the one that maximizes overall key compatibility (using the continuous
  detuning model, so the sweep is smooth with no discontinuous jumps).
  The sweep is constrained so that no track's shift exceeds 1 semitone
  (~6%) from the reference. If the pool's BPM range is too wide for any
  reference BPM to satisfy this constraint, report it: "pool spans too wide
  a BPM range for reliable harmonic evaluation at a single reference BPM."
  Output includes: `optimal_reference_bpm` and `key_stability_at_optimal` vs
  `key_stability_at_median`.
- For sequenced chapters, the first track's BPM is the natural reference.
- For unordered chapters, the optimal BPM from `describe_pool` is the
  recommendation.

### Weight table

Default "balanced" preset:

| Axis | Weight | Rationale |
|------|--------|-----------|
| BPM | 0.25 | Defines the pool's tempo identity |
| Energy | 0.20 | Pools should be a coherent energy band |
| Timbral | 0.18 | The discovery axis — "sounds like it belongs" |
| Key | 0.12 | Less important — enough key variety in any Camelot neighborhood |
| Genre | 0.10 | Low to avoid restating existing tags |
| Brightness | 0.08 | Timbral detail |
| Rhythm | 0.07 | Timbral detail |

A "timbral" priority preset upweights the timbral axis for discovering sonic
relationships the DJ didn't tag. Exact weights TBD via manual tuning session
(see open questions).

Note: with master tempo off, BPM differences degrade both the BPM score AND
the key score (via pitch shift). This means the effective weight of BPM
proximity is higher than the nominal 0.25 — pools naturally become tighter
when master tempo is off. This is correct behavior.

Key and genre are intentionally downweighted relative to the transition scorer.
If they dominate, pools just restate existing tags. BPM, energy, and timbre
define sonic coherence.

### TrackProfile extension

Add optional timbral fields (already in Essentia cache, just not pulled through
to the profile yet):

```rust
mfcc_mean: Option<Vec<f64>>,              // 13-dim
mfcc_std: Option<Vec<f64>>,               // 13-dim (timbral variation)
spectral_contrast_mean: Option<Vec<f64>>, // per-band
spectral_centroid_cv: Option<f64>,        // brightness variation
dissonance_mean: Option<f64>,             // harmonic roughness
```

### Z-score normalization for timbral axis

The timbral feature vector requires z-score normalization (subtract mean,
divide by stddev per dimension) computed across the library. Without this,
dimensions on different scales would dominate the distance calculation (e.g.,
spectral centroid in Hz vs MFCC coefficients in arbitrary units).

Storage: normalization stats (mean + stddev per dimension) stored in the
internal SQLite cache. Recomputed when >10% of tracks are added or when
explicitly triggered. Cheap computation — one pass over all cached Essentia
results.

## New tools

### `score_pool_compatibility`

The symmetric kernel, exposed as an MCP tool with three modes:

All modes accept `master_tempo: bool` (default `false`) and optional
`reference_bpm`. When `master_tempo=false` and no reference BPM is given,
defaults to the median BPM of the tracks being scored.

**Mode 1 — Pairwise**: Score two tracks for pool compatibility.

```
score_pool_compatibility(track_a: "id", track_b: "id",
  master_tempo: false, reference_bpm: null)
→ { composite, per_axis_scores }
```

**Mode 2 — One-vs-pool**: Score a candidate track against an existing pool.
Used by `expand_pool` internally and by the agent during SOP-driven pool
building.

```
score_pool_compatibility(track_id: "id", pool_track_ids: [...],
  master_tempo: false, reference_bpm: null)
→ { min_score, mean_score, per_member_scores }
```

Ranks by minimum compatibility to any pool member (the improvisation guarantee:
"this track works no matter which pool member you play before or after it").
Mean compatibility as tiebreaker.

**Mode 3 — Pool cohesion**: Analyze an entire pool's internal compatibility.
Used by `describe_pool` internally.

```
score_pool_compatibility(pool_track_ids: [...],
  master_tempo: false, reference_bpm: null)
→ { mean_pairwise, min_pairwise, weakest_member, medoid, per_pair_scores }
```

### `expand_pool`

Multi-seed pool expansion. Given anchor tracks, find more tracks compatible
with the entire pool.

```
expand_pool(
  seed_track_ids: [...],
  additions: 3,             // how many tracks to add (default 3)
  master_tempo: false,       // default false
  reference_bpm: null,       // optional; defaults to median seed BPM
  cross_genre: false,        // true to disable genre family prefilter
  // uses existing selector pattern for candidate universe:
  // playlist_id, or search filters + max_tracks
)
→ {
  additions: [
    { track, min_score, mean_score, rationale: { strongest_axes, weakest_axis, most_compatible_member } },
    ...
  ],
  pool_cohesion: { mean_pairwise, min_pairwise },
  stopped_early: bool,      // true if quality threshold not met for all requested additions
  candidates_scanned: usize
}
```

#### Candidate universe

The candidate universe is not capped at a fixed number. Instead, `expand_pool`
pre-filters aggressively in SQL, then scores everything that passes:

1. **BPM range**: lower bound = lowest seed BPM × 0.92, upper bound = highest
   seed BPM × 1.08. The Gaussian decay already penalizes non-linearly (0.50 at
   6%, 0.30 at 8%), so the 8% hard cutoff only excludes tracks that can't
   possibly rank well.
2. **Genre family** (when `cross_genre=false`, the default): same family as
   any seed (House, Techno, Bass, Downtempo). Tracks with
   `GenreFamily::Other` are included if any seed is also Other.
   When `cross_genre=true`: no genre filter. Enables cross-genre timbral
   discovery ("what in my library sounds like these regardless of tags").
   Increases the candidate universe but lets the timbral axis surface
   relationships that genre tags would hide.
3. **Exclude seeds**: don't re-suggest tracks already in the pool.

On a typical 2,000-track library, BPM + genre family filters reduce the
candidate universe to 100-500 tracks. All are scored — no artificial cap.

#### Expansion algorithm

**Iterative greedy expansion**, not "top N from a flat ranking":

1. Score all candidates against the seed pool (min-compatibility to any seed).
   Pick the single best. Pool is now seeds + 1.
2. Re-score remaining candidates against the expanded pool. Pick the best.
   Pool is now seeds + 2.
3. Repeat until `additions` reached or quality threshold not met.

Each addition is guaranteed compatible with the full pool at the time it's
added, including prior additions. This prevents the failure mode where
multiple additions are each compatible with the seeds but incompatible with
each other.

**Quality threshold**: if the best remaining candidate scores below a minimum
composite (e.g. 0.4), stop short and report `stopped_early: true`. Better to
return 2 strong additions than 5 where the last 3 are mediocre. The DJ can
widen search filters or adjust seeds rather than getting a weak pool.

#### Per-addition rationale

Each addition reports why it was chosen:
- Which axes scored highest (e.g. "strong timbral match")
- Which axis scored lowest (e.g. "key distance is moderate")
- Which seed it's most compatible with

This helps the DJ evaluate quickly without listening to every track.

### `describe_pool`

Analyze an existing playlist/pool for cohesion, coverage, and structure.
Single-pool analysis only — does not know about other pools.

```
describe_pool(
  pool_track_ids: [...],  // or playlist_id
  master_tempo: false,    // default false
  reference_bpm: null     // optional; if omitted, uses median BPM
)
→ {
  cohesion: { mean_pairwise, min_pairwise },
  medoid_track_id,
  weak_members: [...],       // tracks with low min-compatibility to rest
  energy_band: [low, high],
  bpm_center, bpm_spread,
  key_neighborhood: [...],   // effective keys at reference BPM if master_tempo=false
  dominant_genre,
  analysis_coverage,         // % of tracks with full Essentia data
  // master_tempo=false only:
  reference_bpm_used,
  optimal_reference_bpm,     // BPM that maximizes key compatibility
  key_stability_at_optimal,  // key cohesion score at optimal BPM
  key_stability_at_median    // key cohesion score at median BPM
}
```

Bridge-finding is handled separately by the Chapter Set Planning SOP, not by
`describe_pool`. See the bridge-finding section below.

## Bridge-finding

Bridge-finding is a between-chapters concern, handled in the Chapter Set
Planning SOP rather than in any single tool. The process:

1. Define boundary tracks: for sequenced chapters, the tail 2-3 tracks of the
   outgoing chapter and head 2-3 tracks of the incoming chapter. For unordered
   chapters, all members.

2. Find bridge candidates from three sources:
   - **Library search**: `expand_pool` with seeds from both chapters' boundary
     tracks, small `additions` (3-5). Tracks compatible with both sets of
     seeds are natural bridges.
   - **Edge members**: `describe_pool` flags weak members (low
     min-compatibility within their own pool). A track that's an edge member
     of chapter A but also compatible with chapter B is a natural bridge.
   - **BPM/energy interpolation**: if chapters differ in BPM or energy, seek
     tracks that sit between them on those axes.

3. Score bridge candidates using `score_pool_compatibility` mode 2 against both
   chapters' boundary tracks. Rank by minimum of the two min-scores (must work
   with both sides).

4. Present 3-5 bridge options to the DJ with context about why each works.

## Workflow

### Pool Building SOP (with new tools)

1. Collect seed tracks from DJ
2. `expand_pool` with seeds, `additions` = 2-5
3. Present additions with rationale and pool cohesion stats
4. DJ removes/adds tracks, may run `expand_pool` again with updated seeds
5. `describe_pool` on the refined pool
6. DJ locks as chapter (sequenced or unordered), saved as mini-playlist

### Chapter Set Planning SOP

1. DJ presents locked chapters (playlist IDs) + sequenced/unordered flags
2. `describe_pool` on each chapter — energy band, BPM center, genre
3. Agent proposes chapter order based on energy arc across the night
4. DJ approves or reorders chapters
5. For each chapter boundary: find bridge tracks (see bridge-finding section)
6. DJ auditions and approves bridges
7. For unordered chapters: propose internal sequence via `build_set` or
   pairwise `score_transition`
8. Present full set plan, allow swaps
9. Export via `write_xml`

## `discover_pools` algorithm (future)

Research evaluated Louvain, spectral clustering, DBSCAN, k-means, and
clique-based approaches. Recommendation: **maximal clique enumeration on a
thresholded compatibility graph** (Bron-Kerbosch with pivoting).

Why not the alternatives:
- **Louvain/Leiden**: resolution limit problem — produces large communities
  (30-100+), not the 2-12 track pools needed. Disjoint only.
- **Spectral clustering**: requires pre-specifying k, disjoint, no size
  control.
- **DBSCAN/HDBSCAN**: designed for density-separated clusters. Our
  compatibility graph is dense (most pairs have nonzero scores) — DBSCAN
  either puts everything in one cluster or fragments into noise.
- **K-means on feature vectors**: throws away the domain-specific compatibility
  scores. If we've already computed pairwise compatibility, use it directly.

### Bron-Kerbosch algorithm

1. Build weighted adjacency matrix from pairwise pool compatibility scores
2. Threshold the graph (keep edges where compatibility >= t, e.g. 0.7)
3. Enumerate maximal cliques via Bron-Kerbosch with pivoting
4. Filter cliques to size [2, 12]
5. Rank by mean internal compatibility × size bonus (reward 4-8 over 2-3)
6. Greedy selection: take highest-scoring clique, then next highest that isn't
   a subset of an already-selected clique. Allow overlap (a track can appear
   in multiple pools).
7. Compute core members (above-median internal compatibility) vs edge members
8. Bridge tracks = nodes appearing in 2+ selected pools

Implementation: ~200 lines of Rust, zero external graph library dependencies.
The pivoting variant (Tomita et al., 2006) is essential for performance. For
N=500 with threshold 0.7, expect tens of thousands of maximal cliques,
enumerable in under a second.

### Validation

Pool quality must beat a random baseline convincingly:
- Generate random pools (same number, same size distribution)
- Compare min/mean internal compatibility, internal/external density ratio
- If algorithm pools aren't significantly better, they're noise

Additional validation:
- **Stability testing**: perturb compatibility scores ±5%, re-run. Same pools
  should emerge. Fragile pools indicate noise, not structure.
- **Known-good test**: check if algorithm recovers track groups the DJ has
  actually played together (from Rekordbox session history).
- **Planted-cluster eval**: synthetic tests with known cluster structure in
  `eval_scoring.rs`.

## Implementation order

1. Continuous detuning model for key scoring (fixes cliff bug, needed by pool
   kernel, worth backporting to transition scorer separately)
2. Symmetric `score_pool_compatibility` kernel in `scoring.rs`
3. Extend `TrackProfile` with timbral descriptors from Essentia
4. Z-score normalization infrastructure for timbral axis
5. `expand_pool` tool (iterative greedy, quality threshold)
6. `describe_pool` tool
7. Planted-cluster and bridge-track eval tests in `eval_scoring.rs`
8. Pool Building SOP
9. Chapter Set Planning SOP
10. `discover_pools` (Bron-Kerbosch on thresholded compatibility graph,
    overlapping pools, core/edge/bridge output)

## Documentation (site/)

After the implementation is locked, add documentation to the Starlight site.
Uses the existing patterns: Astro components for SVG visualizations, MDX
content pages, Starlight `Aside`/`Card` components, scoped CSS with Starlight
theme vars.

### New pages

**`concepts/pool-discovery.mdx`** — explains the feature for technically
curious DJs:

- What pools, chapters, bridges, and set plans are (terminology)
- How pool compatibility differs from transition scoring (the "same crate" vs
  "what comes next" distinction)
- The symmetric kernel — what axes it scores and why the weights differ
- Interactive CamelotWheel showing key neighborhoods for a pool
- The reference BPM concept: why pitch shifting affects harmony when master
  tempo is off, with concrete examples

**`reference/pool-scoring.md`** — full technical reference (the nerdy
breakdown):

- Axis-by-axis formulas: BPM Gaussian decay, Camelot distance categories,
  energy distance, genre matching, timbral vector distance
- The continuous detuning model: formula, derivation from psychoacoustic JND
  thresholds, comparison to the old integer-rounding cliff
- Weight tables for balanced and timbral presets
- The iterative greedy expansion algorithm with worked example
- Bron-Kerbosch clique enumeration for `discover_pools`: what a clique is,
  why it's the right primitive, the threshold-enumerate-filter-rank pipeline
- Z-score normalization for the timbral axis

**`workflows/pool-building.mdx`** — user-facing workflow guide:

- Building pools from seed tracks
- Validating and refining pools
- Locking chapters (sequenced vs unordered)
- Chapter set planning with bridges

**`agent/pool-building.mdx`** + **`agent/chapter-set-planning.mdx`** — agent
SOPs (reusing partials from `src/partials/sops/`).

**`mcp-tools/mixing.mdx`** — update to add the new tools:
`score_pool_compatibility`, `expand_pool`, `describe_pool`.

### New components

**`PoolGraph.astro`** — SVG visualization of a small example pool as a
compatibility graph. Tracks as nodes, edges colored/weighted by pairwise
compatibility score. Shows:
- A tight pool (all edges green/high)
- A weak member (one node with thin/yellow edges)
- A bridge track connecting two pools

**`DetuningCurve.astro`** — SVG line chart plotting the detuning factor
`1.0 - 0.5 × (cents / 50)²` from 0 to 50 cents. Annotated with JND
threshold, "noticeable" zone, and "ambiguous key" zone. Compared against the
old cliff behavior (step function at ±0.5 semitones) on the same chart.

**`PoolWeights.astro`** — variant of the existing `PriorityWeights.astro`
showing pool kernel weight presets (balanced, timbral) side by side with
transition scorer weights. Makes the weight differences visually obvious.

**`GreedyExpansion.astro`** — step-by-step diagram showing iterative greedy
pool expansion: seed pool grows by one track per step, candidate scores
update, quality threshold line shown.

### Implementation note

All site work happens AFTER the tools are implemented and tested. The
documentation describes the shipped behavior, not the design intent. Diagrams
use real data from the test suite where possible.

## Open questions

- Pool weight presets: "balanced" (default) and "timbral" are planned. Exact
  timbral preset weights TBD via manual tuning session where DJ creates pools
  with different weight configurations and assesses results.
- Quality threshold for `expand_pool` early stopping — 0.4 composite is a
  starting point. May need per-priority-preset thresholds.
- Detuning factor curve: `1.0 - 0.5 × (cents / 50)²` fits the perceptual
  table well. May need refinement based on listening tests with real mixes.
- MFCC limitation: within-genre discrimination is weak (the "Aucouturier
  problem"). For DJs working within a single genre, the timbral axis may not
  distinguish subtle stylistic differences. The multi-axis approach mitigates
  this, but worth monitoring during tuning.
- Threshold `t` for `discover_pools` Bron-Kerbosch: 0.7 is a starting point.
  Could offer threshold sweep (try t=0.8 for tight pools, t=0.6 for looser
  groupings) and let the DJ choose.
