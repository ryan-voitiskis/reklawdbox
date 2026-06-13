# Deep Techno Classification: Ideas for Improving Accuracy

**Date:** 2026-04-26
**Status:** Brainstorm / proposals. Not validated against a verified-track sample.
**Prior work:** Builds on [genre-classification-improvements.md](genre-classification-improvements.md) and [genre-classification-implementation.md](genre-classification-implementation.md). Respects the empirically invalidated features list from that research (do not reintroduce: `harmonic_proportion`, `bpm_confidence`, `grid_stability`, `mod_centroid`).

## Origin

Generated from a manual-tagging session walking through four POM POM "Lost Tracks" comp tracks (2020):

| Track ID | Title | BPM | Key conf | Decay τ (mid/high) | Spectral flux | Resolved genre |
|---|---|---|---|---|---|---|
| 170960182 | Untitled 95885 | 115 | 0.04 | 102 / 83 | 177 | Downtempo |
| 203991790 | Untitled 26596 | 125 (Rb said 130) | 0.05 | 141 / 178 | 98 | Dub Techno (had chord stab) |
| 113107185 | Untitled 27037 | 124 | 0.01 | 336 / 254 | 18 | Deep Techno (Berghain template) |
| 59342610 | Untitled 42433 | 127 | 0.01 | 224 / 104 | 111 | Electro |

All four tracks scored as atonal (key_conf < 0.05) with long decay tails — superficially similar audio fingerprints, but four different correct genres. None of the discriminators that actually settled each call are currently captured by the classification pipeline.

## Core Observation

The boundaries between Deep Techno and its sonic neighbours hinge on signals the system cannot see today:

| Boundary | Discriminator | Currently measured? |
|---|---|---|
| Deep Techno vs Dub Techno | Periodic chord-stab/wash event | No |
| Deep Techno vs Drone Techno | Kick presence/weight vs drone-as-foundation | No |
| Deep Techno vs Electro | 4/4 kick vs broken/syncopated kick | No |
| Deep Techno vs Tech House | Sidechain pumping | No |
| Deep Techno vs Deep House | Tonality (atonal vs chord-progression) | Partially (key_clarity in Fisher only) |

The decision tree at `src/classify.rs:280` (`compute_audio_profile`) currently consumes only 5 features: `bpm`, `danceability`, `dynamic_complexity`, `rhythm_regularity`, `spectral_centroid_mean`. The Fisher prototype layer can in principle learn these boundaries from MFCCs and spectral contrast, but it can't if any given Techno-family genre has only 3–4 verified tracks (current MIN_TRACKS=5 floor).

## Proposals

Grouped by leverage. Each block names the feature, where it would slot, and what it discriminates.

### A. New stratum-dsp features (highest leverage)

These are signals that genuinely don't exist anywhere in the cache today. New work in `stratum-dsp` crate; new fields on `AudioFeatures` in `src/audio.rs`.

#### A1. Chord-stab detector — *single biggest gap*

- **What:** Detect periodic transient events in the 200–800 Hz band, with off-beat (8th-note offset) periodicity. Output `dub_stab_score: f32` (0.0–1.0) and `dub_stab_period: Option<u8>` (in 16th notes).
- **Why:** This is the *defining* feature of Dub Techno and is currently invisible. It's also what tonight's Untitled 26596 turned on — the audio descriptors all looked Deep-Techno-like, but the user heard the chord stab and the genre flipped.
- **Implementation sketch:** Onset detection in mid-band; cluster onsets by inter-onset interval; if dominant period aligns to off-beat 8th and persists across most of the track, report high score. Beat grid is already detected for `grid_stability`, so the alignment check is cheap.
- **Discriminates:** Dub Techno vs Deep Techno vs Drone Techno (definitively). Also useful for Tech House (if syncopated stabs exist with sidechain).

#### A2. Kick-pattern classifier

- **What:** Detect 4/4 vs broken-beat from kick-band onsets aligned to the beat grid. Output `kick_pattern: enum { FourOnFloor, BrokenBeat, Halftime, Sparse, Irregular }` and confidence.
- **Why:** Pulls Electro out of the Techno-family decisions cleanly. Currently Electro classification leans on Discogs styles — no audio-side check. Tonight's Untitled 42433 was tagged Deep House by an automated pass; a kick-pattern classifier would have flagged it as broken-beat and ruled out everything in the House/Techno four-on-the-floor families.
- **Implementation sketch:** Filter to ~40–120 Hz kick band; detect transient onsets; check phase relative to detected beat grid. If kick lands on every beat → `FourOnFloor`. If kick lands on 1 and 3 with off-beat hits → `BrokenBeat`. If only on 1 of every 2 bars → `Halftime`.
- **Discriminates:** Electro from all 4/4 Techno-family genres. Halftime Dubstep from Dub Techno. Drum & Bass family vs others.

#### A3. Band-split spectral flux

- **What:** Replace single `spectral_flux_mean` with three: `flux_low` (60–250 Hz), `flux_mid` (250–2000 Hz), `flux_high` (2–8 kHz).
- **Why:** Tonight's Untitled 27037 had `spectral_flux_mean = 18` (extraordinarily low) and read as Drone Techno from descriptors. The user heard "warped synth messy melodies create rhythm" in upper bands — i.e. the rhythmic activity was concentrated in 2–8 kHz, drowned out in the global average by static lows. The Berghain Deep Techno template *requires* high upper-band flux over a static low-mid floor; the global mean conceals it.
- **Implementation sketch:** Same FFT pipeline, just sum spectral flux in three bands separately. Cheap.
- **Discriminates:** Deep Techno (high flux_high, low flux_low) from Drone Techno (low everywhere) from Dub Techno (mid flux from chord stabs) from Tech House (flux_low from sidechain pumping).

#### A4. Sub-rumble vs kick separation

- **What:** Split low-frequency content into kick-band (60–100 Hz transients) and sub-rumble (30–60 Hz sustained energy between kicks). Output `sub_rumble_proportion: f32`.
- **Why:** Sustained sub-bass rumble between kicks is a strong Berghain/Deep Techno signature. Tech House has effectively zero rumble (sidechain ducks it). Minimal often has none.
- **Discriminates:** Deep Techno from Tech House and Minimal.

#### A5. Sidechain depth

- **What:** Measure short-term loudness modulation aligned to the beat (i.e. is there a 4-Hz-ish amplitude modulation of the non-kick content). Output `sidechain_depth: f32` (0.0–1.0).
- **Why:** Sidechain pumping is the Tech House signature. Deep Techno has none. House has it lightly. Currently no proxy exists.
- **Discriminates:** Tech House vs Deep Techno; also useful for House vs Deep House.

### B. Use cached features the tree currently ignores

These are already computed and cached — the work is purely wiring them into `compute_audio_profile` and the depth resolver. Lowest cost, real wins.

#### B1. `Atonal` flag from stratum-dsp `key_confidence` — *cheapest win*

- **What:** Add `CharFlag::Atonal` set when `audio.key_confidence < 0.1`. (Note: `key_confidence` from stratum-dsp is distinct from the empirically-invalidated `bpm_confidence` and from the active `key_clarity`. It's stored in the cache at `src/audio.rs:35` but not extracted into `AudioFeatures`.)
- **Why:** Tonight's Untitled 42433 was classified as Deep House at key_conf 0.01. Deep House requires chord progressions; that classification should be impossible. A simple atonal flag with a small set of conjunctive rules catches a whole class of errors:
  - Atonal + House-family votes → strong negative for Deep House (suggest a Techno-family genre instead, or Electro)
  - Atonal + Techno-family votes → boost Deep Techno over Techno (Techno proper has identifiable lead synths more often than not)
  - Atonal + Trip-Hop / Downtempo votes → boost Ambient over melodic alternatives
- **Where it slots:** `compute_audio_profile` at `src/classify.rs:280`. Read in `resolve_same_family_specificity` at `src/classify.rs:914`.
- **Cost:** ~5-line change to set the flag, plus rule additions.

#### B2. `LongTail` flag from `decay_mid_tau`

- **What:** Add `CharFlag::LongTail` when `decay_mid_tau > 200ms`. Already in the Fisher input set, but not in the tree-side flag set.
- **Why:** Long mid-decay is a strong dub/atmospheric tell. Combined with chord-stab presence (A1) it nails Dub Techno; without chord-stab and with low flux_high (A3) it nails Drone Techno; with high flux_high it's Deep Techno.
- **Where it slots:** `compute_audio_profile`. Consume in same-family resolver and in the per-genre conjunctive templates (section C).

#### B3. `Compressed` flag from `loudness_range`

- **What:** Add `CharFlag::Compressed` when `loudness_range < 1.0` (LU). Per the existing research, `loudness_range` is not currently extracted from Essentia at all. Surface it in `AudioFeatures` first.
- **Why:** Heavily-compressed sustained loudness is a club-track signature. Wide loudness range is a headphone/listening track signature. Deep Techno almost always presents `Compressed`.
- **Where it slots:** Add to `AudioFeatures` via `classify_handler.rs` audio extraction (around line 706). Then flag in `compute_audio_profile`.

#### B4. `bpm_agreement` for confidence weighting on the BPM plausibility check

- **What:** `bpm_agreement` is computed but `#[allow(dead_code)]` (per explore report). When the Rekordbox BPM disagrees with the stratum + Essentia consensus by more than a few %, fall back to the detector consensus for the BPM-plausibility check at `src/classify.rs:625`.
- **Why:** Tonight's Untitled 26596 had Rekordbox BPM 130.77 but stratum and Essentia both at ~125. The Rekordbox value was the outlier (likely picking up a syncopated layer). The plausibility check at line 625 reads only Rekordbox BPM, so it would have wrongly demoted any genre with a 125-BPM-but-not-130-BPM range.
- **Where it slots:** Around `gather_votes` BPM check at `src/classify.rs:625`.

> ⚠ Do **not** reuse the `bpm_confidence` (stratum) or `grid_stability` features for any decision — both empirically invalidated in the prior research (total overlap across genres). `bpm_agreement` is a different signal: a Rekordbox-vs-detector consistency check, not a single-detector confidence.

### C. Decision-tree conjunctive templates

Once A and B exist, individual signals are still ambiguous — but conjunctions are highly diagnostic. A small library of per-genre templates would beat any single-feature rule.

#### C1. Deep Techno (Berghain template)

```
Atonal (B1)
  + Techno-family votes (existing)
  + LongTail (B2)
  + rhythm_regularity > 0.9 (existing)
  + Compressed (B3)
  + Dancefloor energy bucket (not HighEnergy) (existing)
  + kick_pattern == FourOnFloor (A2)
  → Deep Techno, high confidence
```

#### C2. Dub Techno

```
Template C1 base
  + dub_stab_score > 0.5 (A1)
  → Dub Techno, high confidence (overrides C1)
```

#### C3. Drone Techno

```
Atonal (B1)
  + LongTail (B2)
  + flux_high < 30 (A3, very-low upper-band flux)
  + flux_low low (A3, no kick activity dominating)
  + duration > 7 minutes (existing)
  → Drone Techno, high confidence
```

#### C4. Electro (negative for Deep Techno)

```
kick_pattern == BrokenBeat (A2)
  → veto all Techno-family / House-family classifications
  → Electro candidate
```

#### C5. Tech House (negative for Deep Techno)

```
sidechain_depth > 0.4 (A5)
  + Dancefloor energy bucket
  + 4/4 kick (A2)
  → Tech House over Deep Techno even on Techno-family Discogs styles
```

These slot in as a pre-pass before the existing `resolve_same_family_specificity` at `src/classify.rs:914`. If a template fires with high confidence it short-circuits the generic resolver; otherwise the existing logic runs.

### D. Prototype-training pipeline improvements

Per memory and the `audio_profile.rs` MIN_TRACKS=5 floor, Deep Techno may not yet have a Fisher prototype at all, or has a very low-N one. These are improvements to how prototypes are built.

#### D1. Hierarchical prototypes

- **What:** Build a Techno-family centroid from all Techno-family verified tracks, then store each Techno-family genre's *delta* from that family centroid in addition to its absolute centroid.
- **Why:** Disambiguates intra-family even when per-genre N is low. Per the existing implementation doc, Deep Techno's Fisher discriminator is currently weak because shared characteristics with Techno proper drown out the differences.
- **Where:** `src/audio_profile.rs` calibration around lines 240–420.

#### D2. Hard feature pruning per genre

- **What:** The Fisher discriminant ranking already down-weights low-variance features, but features like `key_clarity` are near-zero for *every* atonal genre, contributing only noise to those genres' prototypes. Hard-disable features below a between-genre variance threshold per genre, not just down-weight.
- **Where:** `src/audio_profile.rs:288–346` (Fisher discriminant weights selection).

#### D3. Verification feedback loop

- **What:** Tonight's manual-tagging conversation produced four verified classifications. They become training data only if the user manually adds them to the `genre_verified` Rekordbox playlist. Add an MCP tool — `verify_track(track_id, genre)` — that stages the genre change *and* the playlist add atomically. Or alternately, expose a "verified" Rekordbox MyTag check.
- **Why:** Closes the loop without manual playlist curation. Prototypes can then be incrementally recalibrated as verifications accumulate (e.g. nightly).
- **Where:** New tool in `src/tools/`. Calibration trigger in `classify_handler.rs`.

#### D4. Calibration coverage report

- **What:** Per-genre verified-N count alongside the prototype-training threshold. `Deep Techno: 4/5 verified — 1 needed for prototype`.
- **Why:** Tells the user which genres to prioritise verifying. Currently calibration is opaque.
- **Where:** Extend `cache_coverage` tool, or new `audit_calibration` tool.

## Prioritisation

If picking one tiny thing first: **B1 (Atonal flag from `key_confidence`)**. Five-line change in `compute_audio_profile`. Would have caught tonight's Deep House misclassification immediately. Lowest possible cost, real win, and a clean dry-run for the broader "promote cached features into the tree" pattern.

If picking one substantial thing: **A1 (chord-stab detector in stratum-dsp)**. It's the single discriminator that genuinely doesn't exist in any form today, and it cleanly resolves the most consequential mistake the system can make — calling Dub Techno "Deep Techno" or vice versa. These two genres are sonically and DJ-context distinct enough that confusing them matters in real usage.

Suggested implementation order:

1. **B1, B2, B3** — wire cached features into the tree (low cost, immediate accuracy gains)
2. **A2 (kick-pattern)** — pulls Electro out cleanly; clears noise from Techno-family decisions
3. **A1 (chord-stab)** — Dub Techno discrimination
4. **A3 (band-split flux)** — Drone Techno vs Deep Techno discrimination
5. **C1–C5 (conjunctive templates)** — assemble the above into per-genre rules
6. **A4, A5** — round out the feature set
7. **D1–D4** — prototype pipeline improvements (long-running, but D3 unblocks faster verification)

## Validation Plan

Each new feature should be checked against a small ear-verified set before being relied on, mirroring the methodology in [genre-classification-improvements.md](genre-classification-improvements.md). Specifically:

1. Pick 5–10 verified tracks per genre across Deep Techno, Dub Techno, Drone Techno, Tech House, Electro.
2. Compute the proposed feature for each.
3. Check for between-genre separation (no total overlap, like the four invalidated features).
4. Only wire into the decision tree if separation is real.

Features that look obvious from theory have failed this test before (`harmonic_proportion`, `mod_centroid` — both are intuitive but empirically useless for these boundaries). Don't skip the validation step.

## Open Questions

- Does the existing 574-track verified set include enough Deep Techno / Dub Techno / Drone Techno / Tech House samples to validate A1–A5 against, or does the verification feedback loop (D3) need to land first?
- Is band-split spectral flux a stratum-dsp addition, or could it be derived post-hoc from cached spectral data?
- For the conjunctive templates (C1–C5), what's the right confidence boost — a single flag-vote at weight 0.5 (matching AFFINITY_CAP), or a bigger override that short-circuits the same-family resolver?
- Should `verify_track` (D3) gate on user-confirmed-via-conversation only, or auto-verify any track the user has manually genre-tagged in Rekordbox post-XML-import?
