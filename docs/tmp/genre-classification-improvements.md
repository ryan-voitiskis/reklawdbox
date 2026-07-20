# Genre Classification: Research & Analysis

> **Historical research snapshot:** The provider weights and classifier paths
> below describe the pre-2026-07-14 implementation. Beatport is no longer a
> runtime provider; the live classifier and readiness regressions define the
> current source-aware confidence and readiness contracts.

**Date:** 2026-04-10
**Status:** Research complete. See [genre-classification-implementation.md](genre-classification-implementation.md) for the implementation spec.

**Context:** Full SOP test run on 200 ungenred + 72 unknown-genre tracks, followed by deep-dive analysis of the classification pipeline. The Burger/Ink "Elvism" track (correctly Dub Techno) served as the primary case study — the system classified it as Ambient or Minimal with Insufficient confidence, and Dub Techno was never even a candidate.

### Final Decisions Summary

- **Fix 1 (tie-breaking):** Implement. Deterministic 3-key sort. Correctness bug.
- **Fix 2 (audio augmentation):** **Subsumed** by Genre Audio Profiles (Section 11). Skip.
- **Fix 3 (family tiebreak + centroid):** Implement. Centroid on AudioProfile + both-candidates correctness fix.
- **Fix 4 (candidates UX):** Implement. Include chosen genre in candidates array.
- **Fix 5 (ambient veto):** Implement. Add `NonDancefloor + Atmospheric` veto branch. Original beat-presence scoring proposal was **invalidated** empirically.
- **Feature wiring:** Wire 6 validated features into AudioFeatures.
- **Genre Audio Profiles:** Implement Fisher discriminant scoring calibrated from 574 verified tracks.

### Empirically Invalidated Features

These were tested against 17 ear-verified tracks and show total overlap across genres. Do not use for classification:
- `mod_centroid` (Stratum) — total overlap across Dub Techno, Deep Techno, Minimal, Ambient
- `harmonic_proportion` (Stratum) — total overlap
- `bpm_confidence` (Stratum) — total overlap
- `grid_stability` (Stratum) — heavy overlap

---

## Table of Contents

1. [Background: How Classification Works Today](#1-background)
2. [The Elvism Case Study](#2-elvism)
3. [Fix 1: Deterministic Tie-Breaking](#3-fix-1)
4. [Fix 2: Audio-Augmented Candidates for Weak Consensus](#4-fix-2) *(subsumed by Section 11)*
5. [Fix 3: Expanding `audio_clearly_favors_family`](#5-fix-3)
6. [Fix 4: Candidates Array UX](#6-fix-4)
7. [Fix 5: Ambient / Beatless Detection](#7-fix-5)
8. [Dependencies and Build Order](#8-dependencies)
9. [Open Questions](#9-open-questions)
10. [Audio Feature Expansion: Untapped Signals](#10-feature-expansion)
11. [Genre Audio Profiles: Fisher Discriminant Scoring](#11-genre-profiles)

---

## 1. Background: How Classification Works Today {#1-background}

The decision tree in `src/classify.rs` has four phases:

### Phase 1: Audio Vetoes (lines 296-403)

Before any vote-based scoring, hard audio checks can short-circuit the entire pipeline:

- **NonDancefloor + Ambient** (danceability < 1.0, dynamic_complexity > 10.0) → Ambient, Medium
- **NonDancefloor + Slow** (danceability < 1.0, BPM < 115) → Downtempo family, Low
- **NonDancefloor (generic)** → Downtempo family, Low
- **Fast + Dancefloor** (BPM > 155) → Bass family, Medium
- **LowEnergy + Atmospheric + enrichment disagrees** → Downtempo family, Low

Vetoed results get `flags: ["audio-vetoed"]` and empty candidates arrays. No votes are gathered.

### Phase 2: Vote Gathering (`gather_votes`, lines 439-525)

When no veto fires, votes are collected from four sources:

| Source | Base Weight | BPM-implausible penalty | Notes |
|--------|------------|------------------------|-------|
| Beatport | 1.0 | halved to 0.5 | Track-level, strongest single source |
| Discogs | proportion × 0.9 × diversity_decay × confirmatory | halved | Album-level; decay = 1/(n^0.4) for n genres; confirmatory = 0.75 when Beatport exists |
| Label | 0.6 (0.4 if confirming) | halved | From static label→genre table |
| Current genre tokens | 0.5/n per token | halved | Only for non-canonical, non-alias genre strings |

The Discogs weighting deserves explanation. `diversity_decay` penalises albums with many genre tags — a 5-genre release gets decay 1/(5^0.4) = 0.525, reflecting that album-level data spread across many genres is less informative per-genre. When Beatport also provides a track-level genre, Discogs is further reduced by 0.75× (the `confirmatory` factor) because Discogs is now playing a supporting role rather than primary.

When Beatport returns a genre that doesn't map to any canonical genre (e.g. "Electronica"), `evidence.beatport_genre` is set to `None`. This means no Beatport vote is cast AND the confirmatory discount is not applied to Discogs (correctly — Beatport didn't provide useful data).

### Phase 3: Consensus Finding (`find_consensus`, lines 539-797)

Votes are tallied into a HashMap, ranked by total weight, and confidence is assigned:

- **Single genre, score >= 1.0**: High (or Medium if BPM-implausible)
- **2+ genres, margin/total > 0.40**: High/Medium
- **2+ genres, margin/total > 0.15**: Medium. Same-family ties get depth resolution (e.g. Techno vs Deep Techno resolved by audio energy).
- **2+ genres, margin/total <= 0.15** (close race):
  - Same family → Low, with depth resolution
  - Different family + audio clearly favours one → Low with `audio-assisted-tiebreak`
  - Different family, no clear audio signal → **Insufficient**
- **Single genre, score < 1.0**: Medium (or Low if BPM-implausible)

After initial genre/confidence selection, two post-processing steps run:

**BPM Override** (lines 723-751): If the winning genre is BPM-implausible but a runner-up is plausible, swap to it with a confidence downgrade.

**Depth Demotion** (lines 753-779): HighEnergy audio always demotes deep variants (Deep Techno → Techno). Dancefloor audio demotes only when the shallower variant also has votes.

### Phase 4: Audio-Only Inference (lines 906-1104)

When there are **zero votes** (no enrichment data maps to known genres), audio features alone drive classification through a rule cascade:

- **D.1**: Broad bucket by energy level and BPM
- **D.2**: Subgenre by BPM × rhythm regularity
- **D.3**: Spectral centroid refinement
- **D.4**: Confidence assignment (single candidate = Low, multiple = Insufficient)

This phase is completely unreachable when any enrichment vote exists — the `votes.is_empty()` gate at line 186 ensures mutual exclusion with `find_consensus`.

### Key Structures

**`AudioProfile`** (line 159): `{ bucket: EnergyBucket, flags: Vec<CharFlag>, bpm: f64 }`. Only carries computed abstractions, not raw feature values. The `CharFlag` enum has 6 variants: Ambient, Atmospheric, Broken, Irregular, Fast, Slow.

**`AudioFeatures`** (line 110): The bridge between raw analysis and the classifier. Currently passes through only 7 of the ~25 available features: `rekordbox_bpm`, `stratum_bpm`, `bpm_agreement`, `danceability`, `dynamic_complexity`, `rhythm_regularity`, `spectral_centroid_mean`. Rich Stratum features (`bpm_confidence`, `grid_stability`, `harmonic_proportion`, decay constants) and additional Essentia features (`onset_rate`, `spectral_flux`, `intensity_var`) are cached but never reach the classifier.

**`GenreFamily`** enum: `{ House, Techno, Bass, Hardcore, Downtempo, Other }`. Used for same-family detection, depth resolution, and audio tiebreaking. Notable mappings: Ambient → Downtempo, Minimal → Techno, Dub Techno → Techno (depth 6).

---

## 2. The Elvism Case Study {#2-elvism}

**Track:** Burger/Ink — "Elvism" from [Las Vegas] (1996)
**Actual genre:** Dub Techno (per the user, and consistent with Burger/Ink's output as an Atom Heart project from the golden era of dub techno)
**System result:** Insufficient confidence, randomly picks Ambient or Minimal

### Why the system fails

**Enrichment data is noisy and album-level:**
- Discogs: Trance, Ambient, Minimal Techno (album-level styles for a diverse album)
- Beatport: "Electronica" → maps to None (unknown), no vote
- Label: "Harvest" → no mapping
- Current genre: "Electronica" → `extract_genre_tokens` returns empty (no matching keyword)

**Vote computation:**
All three Discogs genres get equal weight: (1/3) × 0.9 × (1/3^0.4) × 1.0 = 0.193 each. Trance is BPM-implausible at 120bpm (range 131-150), halved to 0.097. Ambient and Minimal are BPM-plausible (neither has a defined range), remaining at 0.193 each.

**Consensus:**
Tally: `{Ambient: 0.193, Minimal: 0.193, Trance: 0.097}`. Ambient and Minimal are **tied**. The HashMap→Vec→sort pipeline at line 554 produces non-deterministic ordering for tied scores (HashMap iteration order is random in Rust, and the sort comparator only compares scores). Whichever wins becomes `top_genre`.

margin = 0.0, total = 0.483, margin/total = 0.0 → close race. Ambient (Downtempo family) vs Minimal (Techno family) → different families. `audio_clearly_favors_family` is called:
- For Downtempo: needs `LowEnergy + Atmospheric`. But dynamic_complexity = 3.55 < 5.0, so no Atmospheric flag. **Fails.**
- For Techno: needs `Dancefloor+ + !Broken + bpm >= 125`. But bucket is LowEnergy (< Dancefloor). **Fails.**

Result: **Insufficient** confidence. Genre flips between Ambient and Minimal across runs.

**Dub Techno is never even a candidate** because no enrichment source votes for it, and audio-only inference is gated behind `votes.is_empty()`.

### What the audio actually shows

- BPM: 120 (squarely in Dub Techno range 118-132)
- Rhythm regularity: 0.967 (very regular grid — hallmark of machine-locked dub techno)
- Spectral centroid: 264 Hz (extremely dark/dubby — well below typical techno at 1200-2500 Hz)
- Danceability: 1.37 → LowEnergy bucket (not energetic but not beatless)
- Dynamic complexity: 3.55 (low — steady, hypnotic, not dynamically varied)

This is a textbook Dub Techno audio signature. The system has all the information it needs but cannot use it because audio-only inference is locked out by the existence of (noisy) enrichment votes.

---

## 3. Fix 1: Deterministic Tie-Breaking {#3-fix-1}

### Problem

`find_consensus` (line 549-555) and `build_candidates` (line 1127-1149) both convert a `HashMap` to a `Vec` and sort by score. HashMap iteration order is random per-process in Rust. When two genres have identical scores, the winner depends on HashMap state — the same track gets different recommendations across runs.

There is also a test (`collection_pod_ghost_4way_split_insufficient`) that already acknowledges this: its assertion message says *"depending on sort order"* — the test was written to tolerate the bug.

### Root Cause

```rust
let mut tally: HashMap<&'static str, f32> = HashMap::new();
for v in votes { *tally.entry(v.genre).or_default() += v.weight; }
let mut ranked: Vec<(&'static str, f32)> = tally.into_iter().collect();
ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
```

`tally.into_iter()` produces elements in arbitrary order. `sort_by` with only a score comparator leaves tied elements in whatever order they arrived. The sort is stable (Rust's `Vec::sort_by` is stable), but stability only preserves the *input* order — which is random from the HashMap.

### Proposed Fix

Add a compound 3-key sort: score descending → BPM-plausible first → alphabetical ascending. This requires widening the tally in `find_consensus` from `HashMap<&str, f32>` to `HashMap<&str, (f32, bool)>` to track per-genre BPM plausibility (already done in `build_candidates`).

```rust
// find_consensus
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

Same pattern in `build_candidates`:
```rust
candidates.sort_by(|a, b| {
    b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| b.bpm_plausible.cmp(&a.bpm_plausible))
        .then_with(|| a.genre.cmp(b.genre))
});
```

### Why this tiebreaker order

1. **Score descending** — unchanged, the primary ranking signal.
2. **BPM-plausible first** — when two genres tie on score, the one whose BPM range fits the track is the better recommendation. This doesn't double-count BPM (which is already factored into vote weights) — it's a binary filter on the *winner* of an otherwise-tied race.
3. **Alphabetical** — a deterministic backstop with no false signal. Arbitrary but reproducible.

### Why not other approaches

- **Pure alphabetical:** Deterministic but semantically arbitrary. "Ambient" always beats "Minimal" regardless of evidence.
- **BPM-distance:** Already factored into vote weights via the halving penalty. Re-applying it as a tiebreaker would double-count the same signal.
- **BTreeMap instead of HashMap:** Imposes O(log n) insertion and couples accumulation ordering to the output. The fix belongs in the sort, not the data structure.

### Affected Sites

Only two HashMap→sort patterns exist in the codebase. The audio-only inference `candidates` Vec (lines 1041, 1059) uses `sort_by_key` on a deterministic position lookup — no issue there.

### Downstream Updates

All tuple destructures after `ranked` in `find_consensus` must be updated from 2-tuple to 3-tuple: `ranked[0]`, `ranked.get(1)`, `total_weight` map, BPM-override find closure, `alt_genre` destructure, shallower check (~6 sites within `find_consensus`).

### Elvism Effect

Ambient and Minimal tie at 0.193, both BPM-plausible. Alphabetical tiebreak: "Ambient" < "Minimal" → Ambient always wins. The result becomes deterministic (though still Insufficient confidence — the other fixes address that).

---

## 4. Fix 2: Audio-Augmented Candidates for Weak Consensus {#4-fix-2}

> **STATUS: SUBSUMED** by Genre Audio Profiles (Section 11). Genre Audio Profiles provide the same functionality (injecting audio-derived votes for genres not present in enrichment) but with data-driven scoring instead of hardcoded signatures. Skip this fix.

### Problem

The hard gate at line 186 (`if votes.is_empty()`) creates a binary world: either audio-only inference runs (zero enrichment) or `find_consensus` runs (any enrichment). There is no middle ground for tracks where enrichment exists but is weak or noisy. The system cannot propose genres that enrichment missed — even when the audio signature strongly indicates a specific genre.

This is the core reason Dub Techno never appears as a candidate for Elvism: Discogs provides three noisy album-level votes, Beatport provides nothing useful, and the existence of those Discogs votes blocks audio-only inference entirely.

### Intent

Allow the system to inject audio-derived synthetic votes into the vote pool when enrichment is too weak to produce a confident result on its own. These synthetic votes participate in the normal consensus machinery — they don't bypass it. This means all existing confidence thresholds, depth resolution, and BPM plausibility checks apply equally to audio-derived candidates.

### Design: Inject Before Consensus, Not After

The correct integration point is between `gather_votes` and `find_consensus`:

```
gather_votes(evidence, profile) → votes
    ↓
should_augment(&votes, evidence) → bool
    ↓ (if true)
audio_augment_votes(profile, audio, &votes) → extra_votes
votes.extend(extra_votes)
    ↓
find_consensus(evidence, &votes, profile) → result
```

Injecting before consensus reuses all existing machinery for free. Post-consensus augmentation would require duplicating confidence scoring or adding a fragile special-case layer.

### When to Augment: `should_augment`

Augmentation triggers when enrichment is present but unreliable:

1. **Low total weight** — `total_weight < 0.55`. This catches cases where all enrichment votes are BPM-penalised or spread thin. A single Beatport vote (1.0) or a focused Discogs release (0.90) always exceeds this threshold.
2. **Noisy Discogs heuristic** — 3+ Discogs genres AND no Beatport AND no label mapping. This catches album-level tagging from diverse releases regardless of total weight.
3. **Never when votes are empty** — that case is handled by `audio_only_inference`.

The `label_genre.is_none()` guard prevents augmentation when the label provides a strong signal (e.g. Basic Channel → Dub Techno at weight 0.6).

### Audio Signatures

Each signature defines the conditions under which a synthetic vote is injected, plus the vote weight (capped at 0.45 — always below Beatport's 1.0 and label's 0.6). The `already_voted` guard prevents injection for genres that already have enrichment votes (avoids double-counting).

**Dub Techno** (weight 0.40 — highest audio-only weight)
- BPM ∈ [113, 137]
- rhythm_regularity >= 0.93 (very regular grid)
- spectral_centroid < 800 (very dark/dubby — the single best discriminator from Minimal and Deep Techno)
- LowEnergy or Dancefloor bucket
- dynamic_complexity < 8.0

Why 0.40: The combination of very-low centroid + very-high regularity + correct BPM is a tight intersection. The false positive rate should be low. This is the signature that motivated the entire feature.

Why centroid < 800: Dub Techno sits in the sub-bass register with heavy reverb and minimal high-frequency content. Typical spectral centroids: Dub Techno 200-700 Hz, Minimal 800-1400 Hz, Deep Techno 600-1200 Hz, House 1400-2200 Hz, Techno 1200-2500 Hz. The 800 Hz threshold cleanly separates Dub Techno from adjacent genres.

**Drum & Bass** (weight 0.35)
- BPM >= 163
- Dancefloor+ bucket
- rhythm_regularity >= 0.85
- No existing Bass-family vote

**Hard Techno** (weight 0.30)
- BPM ∈ [143, 162]
- Dancefloor+ bucket
- rhythm_regularity >= 0.90
- spectral_centroid >= 2000 (aggressive high-frequency content)
- No existing Trance or Hardstyle vote

**Ambient** (weight 0.30)
- NonDancefloor bucket
- dynamic_complexity ∈ [7.0, 10.0) (the gap below the existing veto threshold)
- No existing Ambient or Ambient Techno vote

**Downtempo** (weight 0.25)
- BPM < 110
- LowEnergy bucket
- dynamic_complexity >= 4.0
- No existing Downtempo-family vote

**Minimal** (weight 0.20 — lowest, most conservative)
- BPM ∈ [118, 134]
- rhythm_regularity >= 0.93
- spectral_centroid ∈ [800, 1600] (mid-dark but above Dub Techno's range)
- Dancefloor bucket
- Only fires when Dub Techno signature did NOT match (mutual exclusion)

### Elvism Through the New Pipeline

1. `gather_votes`: Trance ≈ 0.097, Ambient ≈ 0.193, Minimal ≈ 0.193. Total ≈ 0.483.
2. `should_augment`: n_discogs = 3, no Beatport, no label → noisy_discogs = true → augment.
3. `audio_augment_votes`: BPM=120 ∈ [113,137] ✓, rr=0.97 >= 0.93 ✓, centroid=264 < 800 ✓, LowEnergy ✓, dc=3.55 < 8.0 ✓. No existing Dub Techno vote. **Dub Techno injected at weight 0.40.**
4. New tally: Dub Techno=0.40, Ambient=0.193, Minimal=0.193, Trance=0.097. Total=0.883.
5. `find_consensus`: Dub Techno wins. margin=0.207, margin/total=0.234 > 0.15 → **Medium confidence**. BPM plausible for Dub Techno at 120bpm ✓.
6. Result: **genre=Dub Techno, confidence=Medium, flags=["audio-augmented"]**.

### Confidence Ceiling

Audio-augmented results always have enrichment votes in the mix (otherwise augmentation wouldn't trigger). The maximum augment weight is 0.45, while the total_weight denominator always includes enrichment. This means the effective confidence ceiling for an augmentation-driven win is Medium in realistic cases — the system cannot produce High confidence from audio augmentation alone. This is the right safeguard.

### Evidence Trail

Add an evidence line in `find_consensus` for augmented votes:
```
audio-augment: Dub Techno(w=0.40)
```

And a flag `"audio-augmented"` on the result so operators can identify augmentation-assisted classifications.

---

## 5. Fix 3: Expanding `audio_clearly_favors_family` {#5-fix-3}

### Problem

The `audio_clearly_favors_family` function (lines 855-884) is the tiebreaker of last resort for cross-family close races. It determines whether audio features unambiguously point to one family. Currently it's too restrictive:

| Family | Current Check | Gap |
|--------|--------------|-----|
| Downtempo | LowEnergy + Atmospheric (dc > 5.0) | Excludes minimal ambient with low dynamic complexity |
| Techno | Dancefloor+ + !Broken + BPM >= 125 | Excludes Dub Techno at 120bpm with LowEnergy |
| House | Dancefloor + !Broken + 118-132 BPM | Adequate |
| Bass | Fast or Broken+Dancefloor | Adequate |
| Hardcore | Dancefloor+ + !Broken + BPM >= 138 | Adequate |
| Other | Always false | Intentional (too diverse) |

### Root Cause: Spectral Centroid Not Available

The function takes `&AudioProfile` which has only `{ bucket, flags, bpm }`. Spectral centroid is available in `AudioFeatures` and used in audio-only inference (D.3), but never reaches `AudioProfile`. This means the tiebreaker cannot distinguish dark-spectrum ambient from bright-spectrum minimal, or dark dub techno from brighter house, at the same BPM.

### Proposed Changes

**Step 1: Promote centroid to AudioProfile**

Add `centroid: Option<f64>` to the `AudioProfile` struct and populate it from `audio.spectral_centroid_mean` in `compute_audio_profile`. This is a single-field addition with no schema changes — `spectral_centroid_mean` is already on `AudioFeatures`.

**Step 2: Expand Downtempo check**

```rust
GenreFamily::Downtempo => {
    let very_low_centroid = profile.centroid.is_some_and(|c| c < 600.0);
    // Original: evolving/layered atmospheric character
    (profile.bucket == EnergyBucket::LowEnergy && has_flag(profile, CharFlag::Atmospheric))
    // New: minimal/flat ambient with very dark timbre
    || (profile.bucket == EnergyBucket::LowEnergy && very_low_centroid)
    // New: non-dancefloor dark ambient not caught by veto
    || (profile.bucket == EnergyBucket::NonDancefloor && very_low_centroid)
}
```

The 600 Hz threshold aligns with D.3's "very-low centroid" category. A centroid below 600 Hz means the track's spectral energy is concentrated in the sub-bass — this is genuinely extraordinary and rules out most dancefloor genres.

**Step 3: Add Dub Techno / Deep Techno branch to Techno**

```rust
GenreFamily::Techno => {
    let dark_timbre = profile.centroid.is_some_and(|c| c < 1200.0);
    // Standard energetic techno
    (profile.bucket >= EnergyBucket::Dancefloor
        && !has_flag(profile, CharFlag::Broken) && profile.bpm >= 125.0)
    // Dub/Deep Techno: low-energy, regular, dark, 118-132bpm
    || (profile.bucket == EnergyBucket::LowEnergy
        && !has_flag(profile, CharFlag::Broken)
        && profile.bpm >= 118.0 && profile.bpm <= 132.0
        && dark_timbre)
}
```

The 1200 Hz threshold aligns with D.3's "low centroid" category. Deep House (same BPM range) typically sits at 1400-2000 Hz (piano stabs, warm chords, vocals), so < 1200 cleanly separates Dub Techno from Deep House in most cases.

### Critical Correctness Fix: Check Both Candidates

With the expanded checks, both Downtempo (centroid < 600) and Techno (centroid < 1200) could pass for the same track. For Elvism at centroid=264, both the Downtempo check (LowEnergy + very_low_centroid) and the Techno check (LowEnergy + dark_timbre + 118-132bpm) would pass. This is a real problem: whichever genre happened to be ranked first (non-deterministic!) would win the tiebreak.

**The fix:** Change the call site in `find_consensus` (lines 695-706) to check both the 1st and 2nd candidates and only tiebreak when exactly one passes:

```rust
} else {
    if let Some(profile) = audio_profile.as_ref() {
        let top_favored = audio_clearly_favors_family(profile, top_genre);
        let second_genre = second.expect("second exists").0;
        let second_favored = audio_clearly_favors_family(profile, second_genre);
        if top_favored && !second_favored {
            flags.push("audio-assisted-tiebreak".into());
            ClassificationConfidence::Low
        } else if second_favored && !top_favored {
            top_genre = second_genre;  // swap winner
            flags.push("audio-assisted-tiebreak".into());
            ClassificationConfidence::Low
        } else {
            // Both pass or neither passes — can't disambiguate
            ClassificationConfidence::Insufficient
        }
    } else {
        ClassificationConfidence::Insufficient
    }
}
```

This also fixes an existing latent bug: even with the current checks, if the random sort puts a genre that happens to pass `audio_clearly_favors_family` first, it wins — even if the other candidate would also pass. The both-candidates check makes the tiebreak honest.

### False Positive Analysis

**Downtempo `LowEnergy + centroid < 600`:** The main collision risk is sub-bass-heavy dance music (Dubstep, Dub Reggae). Mitigation: Dubstep is Bass family and Dub Reggae is Downtempo family — if either wins the vote, the tiebreak is within-family (same-family path handles it). The function only fires for cross-family ties, so these genres never reach the Downtempo check in a competing position.

**Techno `LowEnergy + 118-132 + centroid < 1200`:** The collision risk is sparse Deep House with very dark mixdowns. Deep House centroid typically sits 1400-2000 Hz. A few edge-case deep house tracks might sit around 1100-1200 Hz — but such tracks genuinely straddle Dub Techno/Deep House and a Low confidence tiebreak is defensible.

### Centroid Thresholds as Constants

Extract to module-level constants that align with D.3:
```rust
const CENTROID_VERY_LOW: f64 = 600.0;  // D.3 very-low centroid
const CENTROID_DARK: f64 = 1200.0;     // D.3 low centroid
```

This prevents drift between `audio_clearly_favors_family` and `audio_only_inference` if thresholds are empirically tuned.

### Elvism Effect

With Fix 1 (deterministic sort), Ambient wins the tie alphabetically. `audio_clearly_favors_family` is called with Ambient (Downtempo family): LowEnergy + centroid 264 < 600 → **passes**. Then checked for Minimal (Techno family): LowEnergy + 118-132 + centroid 264 < 1200 → **also passes**. Both pass → Insufficient (the both-candidates guard fires). So Fix 3 alone doesn't resolve Elvism — but combined with Fix 2 (audio augmentation), Dub Techno wins before the tiebreak is even needed.

---

## 6. Fix 4: Candidates Array UX {#6-fix-4}

### Problem

`build_candidates` (line 1126) filters out the chosen `top_genre` from the candidates array. This creates three issues:

1. **Chosen genre's score is invisible.** If Minimal wins at 0.193 and Ambient is also at 0.193, the output shows `genre: Minimal, candidates: [{Ambient: 0.193}]`. Nothing tells the reader that Minimal's score was equal.

2. **Post-processing creates incoherent scores.** When BPM-override or depth-demotion changes the winner, the original vote-winner's raw score appears in candidates, making it look stronger than the chosen genre. Evidence strings explain the override, but without seeing both scores side-by-side, the picture is incomplete.

3. **Key name inconsistency.** The dispatch format uses `"suggested_genre"` while full/compact use `"genre"` — same value, different key names.

### Proposed Changes

**Add `chosen: bool` to `GenreCandidate`:**

```rust
#[derive(Debug, Clone, Serialize)]
pub(crate) struct GenreCandidate {
    pub(crate) genre: &'static str,
    pub(crate) score: f32,
    pub(crate) bpm_plausible: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub(crate) chosen: bool,
}
```

The `skip_serializing_if` ensures `chosen` only appears in JSON when `true`, keeping non-chosen entries compact.

**Revised `build_candidates`:** Include all genres, mark the chosen one, cap at 4 entries (1 chosen + 3 alternatives):

```rust
fn build_candidates(votes: &[GenreVote], top_genre: Option<&str>) -> Vec<GenreCandidate> {
    // ... tally all votes (no filter) ...
    // Mark chosen entry
    // Sort: chosen first, then by score desc
    // Truncate to max 4 (or 3 if no chosen)
}
```

**Rename dispatch key:** `"suggested_genre"` → `"genre"` in `classify_handler.rs` dispatch format. Add `"candidates"` to the dispatch track object so subagents see the scored alternatives.

### Before/After: Elvism

**Before:**
```json
{"genre": "Minimal", "candidates": [{"genre": "Ambient", "score": 0.193}, {"genre": "Trance", "score": 0.097}]}
```

**After:**
```json
{"genre": "Minimal", "candidates": [
    {"genre": "Minimal", "score": 0.193, "chosen": true},
    {"genre": "Ambient", "score": 0.193},
    {"genre": "Trance", "score": 0.097}
]}
```

The subagent immediately sees the tie and can reason about it.

### Before/After: BPM-override case

**Before:**
```json
{"genre": "Techno", "candidates": [{"genre": "Deep House", "score": 0.5}]}
```
Looks like Deep House was stronger — confusing.

**After:**
```json
{"genre": "Techno", "candidates": [
    {"genre": "Techno", "score": 0.3, "chosen": true},
    {"genre": "Deep House", "score": 0.5, "bpm_plausible": false}
]}
```
Clear: Techno chosen despite lower raw score because Deep House is BPM-implausible.

### Score Semantics

Scores in candidates are "BPM-adjusted vote aggregates" — each `GenreVote.weight` already includes the per-vote BPM penalty. They are NOT post-BPM-override or post-depth-demotion adjusted. Those decisions are documented in the evidence strings and flags. Showing raw adjusted vote tallies is the right level of abstraction.

### Breaking Change

The dispatch key rename from `"suggested_genre"` to `"genre"` is a breaking change for any parser that reads dispatch output by key name. Since dispatch is consumed by LLM subagents that read context (not parse schemas), the practical impact is low. But any programmatic parsers need updating.

---

## 7. Fix 5: Ambient / Beatless Detection {#7-fix-5}

### Problem

The current ambient detection relies on `danceability < 1.0 + dynamic_complexity > 10.0`. This has two fundamental issues:

1. **`dynamic_complexity > 10.0` is backwards for drones.** Dynamic complexity measures loudness variation over time. A constant drone pad has very LOW dynamic complexity. A classical piece with fff/ppp dynamics has very HIGH dynamic complexity. The current threshold catches dynamically varied tracks, not monotone ambient pads.

2. **Danceability alone conflates "no beat" with "low energy."** Many ambient/minimal tracks that have a very subtle beat score below 1.0 on danceability, while many tracks that genuinely lack any beat score above 1.0 if they have sufficient spectral energy variation.

The system has rich beat-presence data already cached (Stratum's `bpm_confidence`, `grid_stability`; Essentia's `onset_rate`, `rhythm_regularity`) but none of it reaches the classifier.

### Original Proposal: Beat-Presence Score

The initial design proposed a weighted `compute_beat_presence()` score from `bpm_confidence` (35%), `grid_stability` (30%), `onset_rate` (25%), and `rhythm_regularity` presence (10%), with a `Beatless` CharFlag firing at threshold < 0.25.

### Empirical Validation: The Proposal Doesn't Work

Testing against 5 verified Dub Techno and 5 verified Ambient tracks from the collection revealed that **`bpm_confidence` and `grid_stability` do not reliably separate rhythmic from non-rhythmic tracks** in this music:

**Dub Techno (verified rhythmic):**

| Track | bpm_conf | grid_stab | onset_rate | danceability |
|---|---|---|---|---|
| E110 — After Irradiation | 0.150 | 0.447 | 4.04 | 2.49 |
| Exos — Indigo | 0.028 | 0.530 | 3.80 | 2.54 |
| Gradient — Cloud Three | 0.007 | 0.586 | 4.90 | 2.41 |
| Waage — W7 | 0.005 | 0.541 | 4.10 | 3.36 |
| Monolake — Reminiscence | 0.660 | 0.453 | 4.12 | 1.99 |

**Ambient (verified non-dancefloor):**

| Track | bpm_conf | grid_stab | onset_rate | danceability |
|---|---|---|---|---|
| Skee Mask — 9181 | 0.076 | 0.321 | 1.51 | 0.92 |
| Road Hog — Abuse These Streets | 0.671 | 0.448 | 3.38 | 1.39 |
| Donato Dozzy — Vaporware 01 | 0.045 | 0.472 | 1.22 | 1.00 |
| dj metatron — spiral worlds | 0.297 | 0.419 | 1.91 | 0.84 |
| Burial — Subtemple | 0.182 | 0.362 | 3.94 | 1.01 |

**Key finding:** Dub Techno `bpm_confidence` is actually *lower* than Ambient in several cases (Exos 0.028, Waage 0.005 vs Road Hog 0.671). The heavy reverb and sparse percussion in dub techno confuse Stratum's beat tracker just as much as ambient texture does. `grid_stability` is only marginally higher for Dub Techno (0.45-0.59 vs 0.32-0.47) with wide overlap.

None of the 5 verified ambient tracks would trigger the Beatless flag at threshold 0.25. Scores range from 0.299 to 0.638.

This is because these ambient tracks aren't truly "beatless" in the signal-processing sense — they contain rhythmic texture, crackle, and subtle pulses that Essentia and Stratum detect. They're ambient in the *musical* sense (atmospheric, non-dancefloor, not for dancing) but not in the "no rhythm whatsoever" sense.

### What Actually Separates Ambient from Dub Techno

The empirical data reveals which features actually discriminate:

| Feature | Dub Techno (n=5) | Ambient (n=5) | Overlap? |
|---|---|---|---|
| **Danceability** | 1.99 – 3.36 | 0.84 – 1.39 | **None (gap 0.60)** |
| **Onset rate** | 3.80 – 4.90 | 1.22 – 3.94 | Some (Road Hog, Burial) |
| **Dynamic complexity** | 3.78 – 6.06 | 5.49 – 7.86 | Some (overlap at 5.5-6.0) |
| **Centroid** | 375 – 1060 | 263 – 1432 | Heavy overlap |
| **bpm_confidence** | 0.005 – 0.660 | 0.045 – 0.671 | Total overlap |
| **grid_stability** | 0.447 – 0.586 | 0.321 – 0.472 | Heavy overlap |

**Danceability is the only feature with zero overlap** between verified Dub Techno and verified Ambient. The existing EnergyBucket system (NonDancefloor < 1.0, LowEnergy 1.0-1.5, Dancefloor 1.5-2.5, HighEnergy > 2.5) already captures this — all ambient tracks are NonDancefloor or low LowEnergy, all dub techno tracks are Dancefloor or HighEnergy.

### Revised Approach: Improve Existing Mechanisms

Instead of a new beat-presence score, the empirical data supports **strengthening the existing danceability-based detection** and **using Fix 3's centroid expansion** to handle the remaining gaps:

**1. Lower the Atmospheric threshold for ambient veto expansion.**

The current NonDancefloor + Ambient (dc > 10.0) veto is too restrictive. All 5 verified ambient tracks have dc between 5.49-7.86, well below 10.0. The existing `Atmospheric` flag (dc > 5.0) catches 5/5 ambient and only 2/5 dub techno — and those 2 dub techno tracks are in the Dancefloor bucket so they'd never reach the NonDancefloor veto path.

**Proposed:** Add a second veto branch: `NonDancefloor + Atmospheric` (dc > 5.0) → Ambient, Low confidence. The existing `NonDancefloor + Ambient` (dc > 10.0) → Ambient, Medium confidence remains for the strongest cases.

**2. Rely on Fix 3's centroid expansion for LowEnergy ambient.**

Road Hog — Abuse These Streets is the hardest case: danceability 1.39 (LowEnergy, not NonDancefloor), centroid 536. Fix 3's `LowEnergy + centroid < 600` Downtempo tiebreak would handle this track when it competes with a Techno-family genre.

Donato Dozzy — Vaporware 01 has danceability 1.00 (borderline NonDancefloor/LowEnergy) and dc 7.86 (Atmospheric). It would be caught by the expanded `NonDancefloor + Atmospheric` veto if danceability rounds down, or by Fix 3's centroid tiebreak (centroid 935 is above 600 but dc > 5.0 gives Atmospheric flag).

**3. Wire `onset_rate` and `decay_mid_tau` for evidence and future use.**

The data shows onset_rate has some discriminating power (ambient 1.22-3.94 vs dub techno 3.80-4.90) but the overlap makes it unreliable as a sole decision signal.

`decay_mid_tau` (Stratum's mid-frequency reverb decay time in ms) is more promising. Empirical data across 17 verified tracks:

| Genre | decay_mid_tau range | Notes |
|---|---|---|
| Minimal | 47–82ms | Very dry, short decay |
| Dub Techno | 68–226ms | Moderate reverb, shaped rhythmically |
| Deep Techno | 117–323ms | Variable |
| Ambient | **172–1045ms** | Long reverberant tails |

A threshold of `decay_mid_tau > 300ms` catches 4/5 ambient and 0/5 dub techno, though 2/5 deep techno are near the boundary (nthng 323, Polar Inertia 291). This makes it a useful secondary confirmation signal — not clean enough to gate decisions alone, but valuable combined with danceability and centroid.

Both should be wired into `AudioFeatures` for evidence strings (so subagents can see them) and as candidates for future decision logic once more data points are validated.

**4. Consider `decay_mid_tau` as a future Downtempo tiebreak path.**

Once wired to `AudioProfile`, `decay_mid_tau > 300` combined with `LowEnergy` could serve as an additional Downtempo family tiebreak in `audio_clearly_favors_family`, alongside `LowEnergy + centroid < 600` from Fix 3. This would catch ambient tracks like dj metatron — spiral worlds (centroid 1055, too high for centroid check, but decay 353ms). Not for this iteration — needs more validated data points — but the plumbing should anticipate it.

### Other Tier 1 features evaluated and rejected

**`harmonic_proportion`** (Stratum, H/(H+P) from HPSS): Total overlap across all 4 genres (0.637-0.919). Not useful as a discriminator. Every genre in this collection has a similar mix of harmonic and percussive content.

**`spectral_flux_iqr`** (Essentia, spectral burstiness): Ambient is consistently high (5.21-10.43) but overlaps with Dub Techno (Gradient 12.39, Monolake 5.46). Could serve as a tertiary signal but not reliable enough for threshold decisions.

### The `rhythm_regularity` Default Fix

Separately from ambient detection, the default `unwrap_or(0.85)` for missing `rhythm_regularity` should change to `unwrap_or(0.0)`. None means "Essentia found fewer than 5 beats" — this should read as irregular/absent rhythm, not nearly-perfect regularity. This change affects the Broken/Irregular flag assignment in `compute_audio_profile` and should be validated carefully against the existing test suite.

### Summary: Fix 5 is Now Much Simpler

The original proposal required 3 new AudioFeatures fields, a new scoring function, and a new CharFlag. The empirical validation showed the scoring function doesn't work for this music. The revised Fix 5 is:

1. Add `NonDancefloor + Atmospheric` (dc > 5.0) → Ambient, Low confidence veto (one new branch in `check_audio_vetoes`)
2. Wire `onset_rate` and `decay_mid_tau` into `AudioFeatures` for evidence output (plumbing, no decision logic yet)
3. Fix `rhythm_regularity` default from 0.85 to 0.0
4. Let Fix 3's centroid expansion handle LowEnergy ambient via tiebreaking
5. Future: `decay_mid_tau > 300` as an additional Downtempo family tiebreak path (pending more validation)

No new CharFlag. No beat-presence score. The existing danceability bucketing is the best ambient discriminator we have.

---

## 8. Dependencies and Build Order {#8-dependencies}

### Dependency Graph

```
Fix 1 (tie-breaking) ─────────────────────┐
                                           ├─ both modify find_consensus
Fix 3 (family tiebreak + centroid) ────────┘

Fix 4 (candidates UX) ── independent
Fix 5 (ambient veto expansion) ── independent

Feature wiring (Section 10) ── prerequisite for Genre Audio Profiles
Genre Audio Profiles (Section 11) ── subsumes Fix 2 (audio augmentation)
```

### Recommended Build Order

| Phase | Work | Reason |
|-------|------|--------|
| 1a | Fix 1 (tie-breaking) | Smallest, foundational, correctness bug |
| 1b | Fix 4 (candidates UX) | Independent, improves debuggability |
| 1c | Fix 5 (ambient veto expansion) | Independent, small |
| 2 | Fix 3 (family tiebreak + centroid) | Adds centroid to AudioProfile, both-candidates correctness fix |
| 3 | Feature wiring (Section 10) | Plumb ~8 cached features into AudioFeatures |
| 4 | Genre Audio Profiles (Section 11) | Fisher scoring engine, calibration tool, vote injection. **Subsumes Fix 2.** |

Fixes 1, 4, and 5 can run in parallel. Fix 3 should precede feature wiring (both touch AudioProfile). Genre Audio Profiles is the capstone — highest impact, requires wired features.

### Files Affected

| File | Fix 1 | Fix 3 | Fix 4 | Fix 5 | Wiring | Profiles |
|------|-------|-------|-------|-------|--------|----------|
| `src/classify.rs` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `src/tools/classify_handler.rs` | | | ✓ | | ✓ | |
| `src/store.rs` | | | | | | ✓ |
| `src/audio_profile.rs` (new) | | | | | | ✓ |

---

## 9. Open Questions {#9-open-questions}

### Resolved: Audio augmentation centroid thresholds

**Validated empirically.** Centroid < 800 for Dub Techno: 4/5 verified Dub Techno tracks pass (375, 536, 720, 771). Monolake — Reminiscence is the outlier at 1060 (a more IDM-influenced Monolake production). Clean separation from Minimal (both verified tracks at 1074, 1521). Acceptable overlap with Deep Techno (Toki Fuko at 708, Efdemin at 762) — but these are same Techno family so depth resolution handles them, not the augmentation signature.

**Keep centroid < 800.** Do not tighten to < 600. Would drop 2 of 5 verified tracks.

### Resolved: Beat-presence scoring doesn't work for this music

**Invalidated empirically.** `bpm_confidence` and `grid_stability` do not reliably separate rhythmic from non-rhythmic tracks. Dub techno's heavy reverb confuses Stratum's beat tracker as much as ambient texture does. Danceability (Essentia) is the only feature with zero overlap between verified Dub Techno (1.99-3.36) and verified Ambient (0.84-1.39). See Section 7 for full analysis.

### Resolved: Should the `Ambient` CharFlag be renamed?

**No longer needed.** The revised Fix 5 does not add a `Beatless` flag, so there is no naming conflict. The `Ambient` CharFlag (dc > 10.0) remains as-is. Its role is narrow: it catches extremely dynamically varied non-dancefloor tracks for the highest-confidence ambient veto. The expanded `NonDancefloor + Atmospheric` (dc > 5.0) veto handles the more common case.

### Still open: The `rhythm_regularity` default change

Changing `unwrap_or(0.85)` to `unwrap_or(0.0)` has broader implications. It affects the Broken/Irregular flag assignment, which in turn affects `audio_clearly_favors_family` (Bass family check uses Broken flag) and `audio_only_inference` (D.2 uses rhythm_regularity). This change should be validated against the full test suite and potentially against a broader sample of tracks before shipping.

### Still open: Tier 1 features to wire into AudioFeatures

Two features should be plumbed from cache into `AudioFeatures` for evidence strings and future decision logic:

- **`onset_rate`** (Essentia) — some discriminating power for ambient (1.22-3.94) vs dub techno (3.80-4.90) but overlap prevents threshold-based decisions now.
- **`decay_mid_tau`** (Stratum) — the most promising new signal found. Captures reverb decay time: Minimal 47-82ms, Dub Techno 68-226ms, Deep Techno 117-323ms, Ambient 172-1045ms. A threshold of > 300ms catches 4/5 ambient and 0/5 dub techno. Could become a Downtempo family tiebreak path (`LowEnergy + decay_mid_tau > 300`) once validated with more data.

**Rejected:** `harmonic_proportion` (total overlap, not useful), `spectral_flux_iqr` (overlaps between ambient and dub techno).

---

## Appendix A: Empirical Feature Data {#appendix-a}

Ground truth tracks selected and verified by ear. Note: genre tags in the Rekordbox collection are not all accurate (some were rushed or automated previously). The tracks listed below were verified manually.

### Dub Techno (5 verified tracks)

| Track | BPM | Dance | Centroid | Dyn Cmplx | decay_mid | harm_prop | flux_iqr | bpm_conf | grid_stab | onset_rate |
|---|---|---|---|---|---|---|---|---|---|---|
| E110 — After Irradiation | 126 | 2.49 | 375 | 6.03 | 177 | 0.735 | 1.98 | 0.150 | 0.447 | 4.04 |
| Exos — Indigo | 120 | 2.54 | 536 | 4.02 | 226 | 0.859 | 3.99 | 0.028 | 0.530 | 3.80 |
| Gradient — Cloud Three | 120 | 2.41 | 771 | 6.06 | 68 | 0.637 | 12.39 | 0.007 | 0.586 | 4.90 |
| Waage — W7 | 123 | 3.36 | 720 | 3.78 | 81 | 0.700 | 2.03 | 0.005 | 0.541 | 4.10 |
| Monolake — Reminiscence | 130 | 1.99 | 1060 | 5.13 | 137 | 0.678 | 5.46 | 0.660 | 0.453 | 4.12 |

### Deep Techno (5 verified tracks)

| Track | BPM | Dance | Centroid | Dyn Cmplx | decay_mid | harm_prop | flux_iqr |
|---|---|---|---|---|---|---|---|
| Toki Fuko — Astatine | 128 | 1.97 | 708 | 2.22 | 180 | 0.669 | 1.20 |
| nthng — I Just Am | 128 | 1.69 | 1581 | 4.24 | 323 | 0.736 | 0.85 |
| Eric Cloutier — Ekpyrosis | 128 | 1.91 | 812 | 4.03 | 120 | 0.792 | 2.36 |
| Efdemin — Aachen | 135 | 1.65 | 762 | 5.35 | 117 | 0.919 | 2.57 |
| Polar Inertia — Sonic Outlaws | 122 | 1.97 | 1890 | 2.71 | 291 | 0.695 | 0.59 |

### Minimal (2 verified tracks)

| Track | BPM | Dance | Centroid | Dyn Cmplx | decay_mid | harm_prop | flux_iqr |
|---|---|---|---|---|---|---|---|
| G-Man — Political Prisoner | 132 | 1.92 | 1521 | 3.96 | 47 | 0.670 | 4.24 |
| El Choop — Love Yourself | 125 | 1.91 | 1074 | 3.58 | 82 | 0.696 | 1.83 |

### Ambient (5 verified tracks)

| Track | BPM | Dance | Centroid | Dyn Cmplx | decay_mid | harm_prop | flux_iqr | bpm_conf | grid_stab | onset_rate |
|---|---|---|---|---|---|---|---|---|---|---|
| Skee Mask — 9181 | 86 | 0.92 | 263 | 5.49 | 1045 | 0.654 | 5.21 | 0.076 | 0.321 | 1.51 |
| Road Hog — Abuse These Streets | 112 | 1.39 | 536 | 7.71 | 172 | 0.740 | 10.43 | 0.671 | 0.448 | 3.38 |
| Donato Dozzy — Vaporware 01 | 129 | 1.00 | 935 | 7.86 | 863 | 0.738 | 8.51 | 0.045 | 0.472 | 1.22 |
| dj metatron — spiral worlds | 98 | 0.84 | 1055 | 7.29 | 353 | 0.721 | 7.37 | 0.297 | 0.419 | 1.91 |
| Burial — Subtemple | 106 | 1.01 | 1432 | 6.29 | 367 | 0.719 | 5.53 | 0.182 | 0.362 | 3.94 |

### Key Discriminators (from empirical data)

| Comparison | Best Feature | Threshold | Overlap |
|---|---|---|---|
| Ambient vs Dub Techno | **Danceability** | < 1.4 ambient / >= 1.99 dub techno | **None** |
| Dub Techno vs Minimal | **Centroid** | < 800 dub techno / >= 1074 minimal | **None** |
| Dub Techno vs Deep Techno | Centroid | < 800 vs mixed | Some (same family, handled by depth) |
| Ambient vs everything | Dynamic complexity | > 5.0 ambient / < 6.0 dancefloor | Some (but danceability separates first) |
| Ambient vs dancefloor | **decay_mid_tau** | > 300 ambient / < 226 dub techno | **Near-clean** (4/5 ambient, 0/5 dub techno) |
| Ambient vs Dub Techno | bpm_confidence | — | **Total overlap, not usable** |
| Ambient vs Dub Techno | grid_stability | — | **Heavy overlap, not usable** |
| Any | harmonic_proportion | — | **Total overlap, not usable** |
| Any | mod_centroid | — | **Total overlap, not usable** (9.21-13.38 across all genres) |
| Ambient vs Deep Techno | flux_iqr | > 5.0 ambient vs < 2.6 deep techno | **Clean** (but overlaps with dub techno) |

---

## 10. Audio Feature Expansion: Untapped Signals {#10-feature-expansion}

Research across the codebase, MIR literature, and Essentia documentation reveals that the system computes ~26 audio features but the classifier uses only 5. Many of the unused features are directly relevant to the hardest classification boundaries.

### Current State: What's Computed vs What's Used

```
                          Computed    Cached    In AudioFeatures    Used in Classifier
Stratum DSP fields:          18         18            0                    0
Essentia fields:             18         18            4*                   4*
                            ----       ----          ---                  ---
Total:                       36         36            4                    4

* spectral_centroid_mean is in AudioFeatures but never read by compute_audio_profile()
```

### Invalidated: `mod_centroid`

The **modulation spectral centroid** from Stratum DSP was purpose-built for the Dub Techno vs Techno vs Deep Techno boundary. Source code comments in `stratum-dsp/src/features/modulation.rs` document validation on 15 tracks with reference values of ~9.8 (Deep Techno), ~10.4 (Techno), ~11.3 (Dub Techno).

**Empirical validation against 17 verified tracks shows total overlap:**

| Genre | mod_centroid range | Stratum docs |
|---|---|---|
| Dub Techno (n=5) | 9.21 – 12.32 | ~11.3 |
| Deep Techno (n=5) | 8.54 – 12.27 | ~9.8 |
| Minimal (n=2) | 9.83 – 11.74 | — |
| Ambient (n=5) | 10.57 – 13.38 | — |

E110 — After Irradiation (textbook dub techno) scores 9.21, while nthng — I Just Am (deep techno) scores 8.54. The ranges overlap completely across all four genres. The Stratum validation may have been done on a narrow or non-representative sample. **Not usable for genre classification.**

### Feature Priority Matrix

Based on discriminative power for electronic subgenres, ease of integration, and thresholdability. Features marked "validated" have been tested against 17 verified tracks. Features marked "theoretical" need empirical validation.

#### Tier 1: Already cached, wire into AudioFeatures

| Feature | Source | What it discriminates | Status | Priority |
|---|---|---|---|---|
| **`decay_mid_tau`** | Stratum | Ambient vs dancefloor (reverb tail) | **Validated** — near-clean separation | High |
| **`onset_rate`** | Essentia | Ambient/sparse vs dense/busy | **Validated** — some overlap | High |
| **`loudness_integrated`** | Essentia | Mastering style → genre conventions (LUFS) | Theoretical | Medium |
| **`loudness_range`** | Essentia | Dynamic range → compressed vs dynamic | Theoretical | Medium |
| **`spectral_centroid_cv`** | Essentia | Evolving textures vs consistent energy | Theoretical | Medium |
| **`dissonance_mean`** | Essentia | Industrial/experimental vs house | Theoretical | Medium |
| **`decay_mid_r2`** | Stratum | Decay fit quality — gates decay_mid_tau | Supporting | Low |
| **`spectral_contrast_mean`** | Essentia | Sub-bass vs mid-range energy (per-band) | Theoretical | Low |
| **`spectral_flux_iqr`** | Essentia | Rhythmic consistency | **Validated** — some overlap | Low |

All 9 features are computed and cached today. Zero re-analysis needed.

**Empirically invalidated** (do not wire in for classification):
- `mod_centroid` — total overlap across genres (see above)
- `harmonic_proportion` — total overlap (see Appendix A)
- `bpm_confidence` — total overlap (see Section 7)
- `grid_stability` — heavy overlap (see Section 7)

#### Tier 2: Require new Essentia extractors

| Feature | What it adds | Implementation cost |
|---|---|---|
| Spectral flatness | Noise vs tonal content — separates industrial noise from tonal pads | Add `es.Flatness()` to script, ~5 lines |
| Spectral rolloff | Where high-frequency energy drops off — separates bright from dark | Add `es.RollOff()` to script, ~5 lines |
| Beat histogram entropy | Rhythmic complexity — simple 4/4 vs complex polyrhythm | Requires `es.BeatTrackerMultiFeature()` + histogram computation |

#### Tier 3: ML-based models (future phase)

| Approach | What it provides | Implementation cost |
|---|---|---|
| **Discogs-EffNet** | Per-track probabilities for 400 Discogs styles from audio alone | ~20MB model download, TF integration in Essentia script. Functions as a third enrichment source alongside Discogs/Beatport — never has cache misses. **Highest-impact future addition.** |
| CLAP embeddings | Text-audio similarity for free-form genre queries | ~600MB model, unvalidated for electronic subgenres. Lower priority. |

### Feature Interactions: What Pairs of Features Solve

The power of these features is in combination. Single features have overlap; pairs often have clean separation:

| Boundary | Feature 1 | Feature 2 | Why the pair works | Status |
|---|---|---|---|---|
| Dub Techno vs Minimal | `spectral_centroid` < 800 | `danceability` > 1.5 | Dark + rhythmic | **Validated** |
| Dub Techno vs Ambient | `danceability` > 1.5 | `decay_mid_tau` < 230 | Has a beat + moderate reverb | **Validated** |
| Ambient vs Downtempo | `danceability` < 1.4 | `dynamic_complexity` > 5.0 | Low energy + atmospheric | **Validated** |
| Dub Techno vs Deep Techno | `spectral_centroid` < 800 | same Techno family | Darkness distinguishes (depth resolution handles) | **Validated** |
| Techno vs Hard Techno | `loudness_integrated` > -7 LUFS | `spectral_centroid` > 2000 | Compressed + bright | Theoretical |
| House vs Deep House | `spectral_centroid` < 1200 | `danceability` 1.5-2.5 | Dark + moderate energy | Theoretical |
| Breakbeat vs IDM | `onset_rate` > 5 | `rhythm_regularity` < 0.5 | Dense + broken rhythm | Theoretical |

---

## 11. Genre Audio Profiles: Fisher Discriminant Scoring {#11-genre-profiles}

### The Problem with Hardcoded Thresholds

The current decision tree uses manually chosen thresholds (e.g., `danceability < 1.0` = NonDancefloor, `centroid < 800` = Dub Techno). These work for the specific tracks tested but:

- Can't be calibrated against a larger dataset without code changes
- Don't produce per-genre affinity scores (just boolean pass/fail)
- Don't scale to 40+ genres
- Can't express "this track is 70% likely Dub Techno and 30% likely Deep Techno"
- Can't discover which features matter for which genres — we guess and validate manually

### Core Idea: Let the Data Tell Us What Separates Each Genre

For each genre, for each audio feature, compute the **Fisher discriminant score** — how well that feature separates this genre from everything else:

```
fisher_score(genre, feature) = (mean_genre - mean_everything_else)^2
                               / (var_genre + var_everything_else)
```

A high Fisher score means the feature is discriminative for that genre. A low score means it's noise for that genre. This is computed automatically from the data — no manual threshold picking, no guessing which features matter.

For example, across the collection this would likely discover:
- Dub Techno: `spectral_centroid` is highly discriminative (much darker than average)
- Ambient: `danceability` is highly discriminative (much lower than average)
- Drum & Bass: `BPM` is highly discriminative (160+ vs everything else)
- Hard Techno: `loudness_integrated` might be discriminative (heavily compressed)

Features that don't discriminate — like `mod_centroid`, `bpm_confidence`, `harmonic_proportion` — would automatically get near-zero Fisher scores and contribute nothing. No manual invalidation needed.

### Data Quality Strategy

| Layer | Source | Weight | When to use |
|---|---|---|---|
| **Verified** | Tracks in `genre_verified` playlist, confirmed by ear | 1.0 | **Always — primary source** |
| **High-confidence** | Tracks classified High/Medium with enrichment agreement | 0.5 | Phase 2 — after verified prototypes prove reliable |
| **Collection** | All genred tracks (accuracy significantly below 80%) | 0.0 initially | Phase 3 — only after validated against verified prototypes |

**Start verified-only.** With 574 verified tracks and strong coverage for the top genres, there's no need to risk noise from unreliable collection tags. Collection-level data can be blended in later once we can measure how much it helps or hurts accuracy against the verified baseline.

### The Algorithm

**Calibration (runs once, refreshable on demand):**

1. Pull all audio features for all genred tracks (already cached — no re-analysis)
2. Identify verified tracks from the `genre_verified` playlist
3. For each genre with enough data (>= 3 verified OR >= 20 collection tracks):
   - For each feature: compute weighted mean and std (verified tracks at weight 1.0, collection at 0.3)
   - Compute Fisher score against all-other-genres distribution
   - Normalize Fisher scores per genre to sum to 1.0 → per-genre feature weights
   - Store: `{genre, feature, mean, std, fisher_weight, n_verified, n_total}`
4. Report: "Built prototypes for N genres. Top discriminators per genre: ..."

**Scoring (runs per-track during classification):**

```
For each genre G with a prototype:
    distance(G) = sqrt( sum_over_features(
        fisher_weight(G, F) * ((track_F - mean_G(F)) / std_G(F))^2
    ))
    vote_weight(G) = max(0, AFFINITY_CAP * (1 - distance / SCALE))
```

Top N genres by affinity become `GenreVote` entries with source `"audio-profile"` in the existing `gather_votes` pipeline. They participate in normal consensus — no special-casing needed.

Constants:
- `AFFINITY_CAP = 0.5` — below Beatport (1.0) and label (0.6). Start conservative; raise after validation.
- `SCALE = 2.5` — tracks within 1 stddev score ~0.30 vote weight. Tracks beyond 2.5 stddev score 0.

**Evidence strings show the decomposition:**

```
audio-profile: Dub Techno 0.42 (centroid=536~573 [F=3.2], dance=2.5~2.7 [F=2.1], decay=226~147 [F=1.8])
```

The subagent sees: which genre, how strong, which features drove it, and each feature's Fisher weight (discriminative power). Fully interpretable.

### The Verification Workflow

**What the user does:**

1. Create a `genre_verified` playlist in Rekordbox
2. Add tracks where you're confident of the genre — clear representatives, not edge cases
3. Ensure the genre tag is correct on each track in Rekordbox
4. Don't worry about covering every genre — put in what you know
5. Run calibration: `calibrate_audio_profiles(playlist="genre_verified")`

**What the user does NOT need to do:**
- Verify tracks for genres they don't know well
- Hit any minimum count per genre (system handles sparse data gracefully)
- Pick which features matter (Fisher scores compute this automatically)
- Pad the playlist with uncertain tracks — quality over quantity

**Actual verified playlist: `genre_verified` — 574 tracks across ~20 genres**

The playlist was built manually in Rekordbox with tracks the user is confident classifying. Edge cases were excluded. Genre aliases were normalized before use. Distribution:

| Genre | Verified | | Genre | Verified |
|---|---|---|---|---|
| House | 145 | | Dub Techno | 17 |
| Deep House | 140 | | Disco | 11 |
| Ambient | 87 | | Dancehall | 10 |
| Techno | 50 | | Drum & Bass | 5 |
| Electro | 32 | | Downtempo | 5 |
| Hip Hop | 31 | | Garage | 3 |
| Deep Techno | 20 | | Trance | 3 |
| | | | Others | ~15 |

Strong coverage for the top genres (House 145, Deep House 140, Ambient 87, Techno 50). Adequate for Fisher scoring on Electro (32), Hip Hop (31), Deep Techno (20), Dub Techno (17). Sparse for Drum & Bass (5), Downtempo (5), Trance (3).

Notable gaps: Minimal (0), Breakbeat (0), IDM (1). These genres will need either future verification rounds or collection-only fallback.

**Data quality note:** The user estimates collection-wide genre accuracy at significantly below 80%. The initial calibration should use **verified-only data** (no collection fallback). Collection-level statistics can be blended in later once verified prototypes establish a reliable baseline.

### The Active Learning Loop

After the initial calibration, the system can suggest which tracks to verify next:

```
suggest_verification_candidates(n=50)
→ Tracks where audio-profile score disagrees most with current genre tag
→ Tracks from genres with fewest verified examples
→ Tracks near prototype boundaries (most informative for sharpening)
```

Each round of verification improves the prototypes. The workflow becomes:

```
Verify 150-200 tracks → Calibrate → Classify → System suggests next 50 → Verify → Recalibrate → ...
```

Over time the prototypes converge on accurate genre boundaries. The Fisher weights automatically adjust as more data arrives — features that seemed discriminative with 5 tracks might lose weight with 20, and vice versa.

### Storage

Genre profiles stored in SQLite (`genre_audio_profiles` table), not compiled into source:

```sql
CREATE TABLE genre_audio_profiles (
    genre         TEXT NOT NULL,
    feature       TEXT NOT NULL,
    mean          REAL NOT NULL,
    stddev        REAL NOT NULL,
    fisher_weight REAL NOT NULL,
    n_verified    INTEGER NOT NULL DEFAULT 0,
    n_total       INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (genre, feature)
);
```

- Profiles persist across sessions and binary updates
- User-specific (calibrated against this collection)
- Incrementally refined as more tracks are verified
- Can be exported/imported for sharing between collections

### Integration with the Existing Pipeline

Audio-profile votes are one signal among many. They don't replace enrichment or the decision tree — they augment them:

```
                    ┌─ Beatport vote (weight up to 1.0)
                    ├─ Discogs votes (weight up to 0.9)
gather_votes() ────├─ Label vote (weight up to 0.6)
                    ├─ Audio-profile votes (weight up to 0.5)  ← NEW
                    └─ Current-genre tokens (weight up to 0.5)
                              │
                              v
                    find_consensus() → genre + confidence
```

When enrichment is strong (Beatport + Discogs agree), the audio-profile vote is a minor confirming signal. When enrichment is weak or absent (the Elvism case), the audio-profile vote becomes the strongest signal and surfaces genres that enrichment missed entirely.

### How This Replaces Fix 2 (Audio Augmentation)

The audio-augmented candidates proposal (Fix 2 from Section 4) used hardcoded audio signatures for 6 genres (Dub Techno, D&B, Hard Techno, Ambient, Downtempo, Minimal) with manually chosen thresholds and weights. Genre Audio Profiles subsume this entirely:

- Instead of 6 hardcoded signatures → prototypes for all calibrated genres
- Instead of manual thresholds → Fisher-derived weights from data
- Instead of fixed weights (0.20-0.40) → distance-based scoring
- Instead of manual `should_augment` gating → always scores, low affinity produces negligible vote weight

Fix 2 can be **skipped** if Genre Audio Profiles are implemented. Fixes 1, 3, 4, 5 are still valuable independently.

### Implementation Sequence

**Step 1: Wire features into AudioFeatures (prerequisite)**

Add ~8 features from cache to AudioFeatures. Pure plumbing:
- `decay_mid_tau`, `decay_mid_r2` (Stratum)
- `onset_rate`, `loudness_integrated`, `loudness_range`, `spectral_centroid_cv`, `dissonance_mean` (Essentia)
- `spectral_centroid_mean` is already in AudioFeatures but never read — start reading it

No behavior change. Unblocks everything downstream.

**Step 2: Build the calibration engine**

- Read `genre_verified` playlist via existing `get_playlist_tracks`
- Pull audio features for all genred tracks
- Compute per-genre prototypes with Fisher weights
- Store in SQLite
- Report: per-genre feature discriminators, prototype confidence

This is the core algorithm. Deliverable: `calibrate_audio_profiles` MCP tool.

**Step 3: Build the scoring function**

- Weighted distance from track features to each prototype
- Returns top N genre affinities with per-feature decomposition
- `score_audio_profiles(audio: &AudioFeatures, registry: &ProfileRegistry) -> Vec<AudioAffinity>`

**Step 4: Inject into gather_votes**

- Audio-profile affinities become `GenreVote` entries
- Evidence strings added to `find_consensus` output
- Capped at `AFFINITY_CAP = 0.5`

**Step 5: Verification feedback tools**

- `suggest_verification_candidates(n=50)` — picks most informative tracks to verify next
- Reporting: which genres need more data, which prototypes are strongest/weakest

Steps 1-4 are the MVP. The user builds a playlist, calibrates, and gets audio-profile votes immediately.

### Relationship to Other Fixes

| Fix | Status with Genre Audio Profiles |
|---|---|
| Fix 1 (tie-breaking) | Still needed — deterministic sort is a correctness bug independent of scoring |
| Fix 2 (audio augmentation) | **Subsumed** — Genre Audio Profiles do the same thing but better |
| Fix 3 (family tiebreak + centroid) | Still valuable — centroid on AudioProfile helps the tiebreaker. The both-candidates correctness fix is independent |
| Fix 4 (candidates UX) | Still needed — candidates array improvements help regardless of scoring source |
| Fix 5 (ambient veto expansion) | Still valuable — vetoes are hard overrides that fire before voting. The `NonDancefloor + Atmospheric` veto catches clear ambient cases early |
