# Genre Classification: Feature Development TODO

**Started:** 2026-05-10 (post v0.27.0).
**Current as of:** 2026-06-14.
**Goal:** Build out several more discriminating features in stratum-dsp, then wire
them all into the genre classifier in one pass — avoiding per-feature classifier
tuning churn.

This is the canonical short roadmap. The other `docs/tmp/*` files are source
notes or speculative plans; use them for implementation detail, not priority.

## Current direction

1. Keep the classifier evidence-first and reviewable. Do not turn it into an
   opaque auto-retagger.
2. Add observability before adding more rules. `calibration_coverage` now tells
   us where the verified playlist and cached audio are strong enough to train.
3. Validate new DSP features on hand-picked real tracks before classifier
   wiring. User listening and genre judgement are part of the development loop.
4. Defer conjunctive templates and training refactors until several
   independently validated features exist.

## Supporting docs

- Current implementation spine: this file.
- Calibration UX: `verification-feedback-loop-plan.md`.
- Cached classifier flags: `cached-feature-flags-plan.md`.
- Next DSP candidate: `kick-pattern-detector-plan.md`.
- Later DSP candidates: `sub-rumble-and-sidechain-plan.md`.
- Historical/currently speculative: `band-split-flux-plan.md`,
  `conjunctive-templates-plan.md`,
  `prototype-training-improvements-plan.md`,
  `deep-techno-classification-ideas.md`.

## Background

The chord-stab detector (A1) shipped in v0.27.0. Validation on 24 Dub Techno +
20 non-Dub-Techno tracks confirmed:

- Pipeline works end-to-end (Stage 1+2+3 + Rekordbox grid loading).
- The feature is necessary but not sufficient for Dub Techno classification —
  House and Techno tracks with offbeat mid-band content also match.
- Discrimination needs to come from **combining features**, not from any single
  one.

So: ship 3–4 more discriminating features, then do one classifier-tuning pass.

## Phase 1 — Carry-over fix from v0.27.0 testing

- [x] **F0. `dub_stab_onset_rate`.** Surface stab onsets per second alongside
      `dub_stab_onset_count`. Rate is more comparable across track durations —
      pure dub techno has 1.5–4 stabs/s; sparse minimal has < 1; ambient hits
      the `DubStabGridTooShort` flag. ~5 LOC. No new DSP, no schema break (new
      Optional field).

      **Status:** implemented with schema bump and cache output. Real-library
      MCP validation confirms the rate is useful but not discriminative alone:
      Dub Techno samples landed in the expected 1.5-4 stabs/s range, but some
      Techno/House controls also landed there.

## Validation snapshot — 2026-06-13

Using the local stdio MCP smoke path against `genre_verified`:

- `calibration_coverage`: 604 verified tracks across 23 canonical genres;
  604 have current audio features overall, but only 4 had current-schema
  Stratum after the schema 11 bump (Essentia remained 604/604).
- Fresh section analysis after schema 11 produced contiguous, non-overlapping
  section ranges on the real-track sample.
- Dub-stab validation:
  - E110 — "00946" (Dub Techno): 1.55 stabs/s, `all_16th_offbeats`, score 0.82.
  - Tzusing — "4 Floors of Whores" (Techno control): 3.59 stabs/s,
    `offbeat_eighth`, score 0.65.
  - Daniel Stefanik — "#six" (House control): 2.17 stabs/s,
    `all_16th_offbeats`, score 0.72.
  - nthng — "And Then There Was Light" (Ambient control): 0.80 stabs/s,
    template score 0.44.

**Conclusion:** MainGroove section filtering is worth keeping, but
`dub_stab_onset_rate` / template score should not be wired as a direct Dub
Techno rule. Treat it as one feature to combine with A2/A4/A5 and profile
training. False-positive mitigation may need tonal/percussive separation or a
more chord-specific stab detector, but defer until it hurts classifier diffs.

## Validation snapshot — 2026-06-14

After rehydrating the `genre_verified` playlist with local Stratum analysis:

- `calibration_coverage`: 604 verified tracks across 23 canonical genres;
  604/604 have current Stratum features, 604/604 have current Essentia
  features, and no stored profiles point outside the playlist.
- The rehydration completed without final failures. Stratum emitted a small
  number of per-track BPM candidate warnings (`No reasonable-range candidates
  (60-180 BPM)`), which should be treated as track-level review signals rather
  than cache failures.

## Phase 1.5 — Track-section detection primitive (foundational)

Moved here from Phase 2.5 — see "Why this exists" below. Starting here
because it retroactively improves the already-shipped dub_stab feature
**and** every new DSP feature in Phase 3 will use it.

The Phase 2 cached-feature wiring (B2/B3/B4) is independent of sections
and can run in parallel with this work.

(See "Track-section detection primitive" section below for full spec —
S1 and S2.)

## Phase 2 — Cached-feature wiring (no new DSP)

Plan: [`cached-feature-flags-plan.md`](cached-feature-flags-plan.md) (B1–B4
from `deep-techno-classification-ideas.md`).

These are wire-only — features already exist in the cache or come from
already-emitted Essentia output that the classifier isn't reading yet. Cheap
and fast.

- [x] **B1. `Atonal` CharFlag from `key_confidence`** — shipped in v0.27.0
      (commit 37a5322). Triggers when `key_confidence < 0.1`. Decision rules
      land Atonal as a near-veto for House family, deep-preference for
      Techno family.
- [x] **B2. `LongTail` CharFlag from `decay_mid_tau`** — threshold
      `decay_mid_tau > 200 ms`. Reinforces deep-preference branch in
      Techno-family. Conjunctive `LongTail + Atonal` boosts Drone Techno.
      Value already extracted; tree-side only. ~20 LOC.

      **Status:** implemented with tree-side flagging, reviewable `long-tail`
      evidence, same-family Techno depth preference, LowEnergy Techno-family
      tiebreak support, and the `LongTail + Atonal` Drone Techno candidate
      boost. HighEnergy still demotes deep Techno variants. Real-library MCP
      validation found `decay_mid_tau` on only 4 current-schema verified tracks
      after the schema 11 bump; 3/4 fired LongTail (Dub Techno, Techno,
      Ambient). Treat this as wired but mostly dormant until the verified
      playlist is rehydrated.
- [x] **B3. `Compressed` CharFlag from `loudness_range`** — threshold
      `loudness_range < 1.0 LU`. Adds club-master signal for Deep Techno
      preference; suppresses the Atmospheric → Ambient veto when set.
      Needs `loudness_range` extraction added to `AudioFeatures`. ~30 LOC.
      Add duration guard (`> 60s`) — short tracks artificially compress.

      **Status:** implemented with `duration_seconds > 60s` guard, Essentia
      `loudness_range` extraction, reviewable `compressed` evidence, same-family
      Techno depth preference, and suppression of the expanded
      Atmospheric → Ambient veto. A read-only cache aggregate over
      `genre_verified` found 22/604 tracks firing the flag (3.6%): Deep Techno
      3/20, Dub Techno 3/24, Techno 4/50, Deep House 9/140, House 3/145, and
      zero Ambient/Downtempo fires. This is plausible enough to keep the 1.0 LU
      threshold for now; spot-check the House-family fires if classifier diffs
      look noisy.
- [x] **B4. `bpm_agreement` cross-detector fallback** — when
      `bpm_agreement == Some(false)` AND stratum + Essentia consensus on a
      different BPM (within 3% of each other), substitute their mean for
      the Rekordbox BPM in plausibility checks. Adds `"bpm-rekordbox-disagrees"`
      flag. ~50 LOC.

      **Status:** implemented with detector consensus evidence and
      `bpm-rekordbox-disagrees` flagging. Real-cache validation initially found
      many Ambient/Dancehall half-time and non-dancefloor detector agreements,
      so the shipped fallback is intentionally conservative: only
      Dancefloor/HighEnergy audio can use detector consensus, and near-2x tempo
      relationships are rejected. With those guards, the fallback fires on
      1/604 verified tracks (Techno), which is in the expected single-digit
      percent range.

      **Acceptance per item:** flag fires on the verified playlist with
      expected per-genre rates (see `cached-feature-flags-plan.md`
      "Validation"); tests cover positive + negative cases; classifier
      output diff is reviewable. No schema bump needed for B1–B4 — all
      values already serialised.

## Track-section detection primitive (full spec — see Phase 1.5)

**Why this exists.** Track-level feature aggregation is lossy. Real tracks
have intro / main-groove / breakdown / outro sections with very different
content. A kick-pattern detector run over the full track averages real
signal (in the main groove) with noise (silence in the breakdown, sparse
content in the intro/outro). Same for sub-rumble and sidechain depth.

This was already visible in the v0.27.0 dub_stab work: 90-second window
analyses produced different histograms than full-track analyses on the same
tracks, partly because windows hit the main groove cleanly while the
full-track aggregate diluted with intro/breakdown content.

Doing this BEFORE A2/A4/A5 means each new feature can use sections from
day one. Doing it after means refactoring all of them.

- [x] **S1. `detect_track_sections` primitive** in `stratum-dsp`.

      **API sketch:**
      ```rust
      pub struct TrackSection {
          pub start_seconds: f32,
          pub end_seconds: f32,
          pub kind: SectionKind,
          pub kick_density: f32,    // kicks per second in this section
          pub energy_db: f32,        // mean RMS energy
      }

      pub enum SectionKind {
          Intro,         // low energy, low kick density, near track start
          MainGroove,    // high energy, high kick density
          Breakdown,     // low/medium energy, low kick density, mid-track
          Outro,         // low energy, low kick density, near track end
      }

      pub fn detect_track_sections(
          spec: &[Vec<f32>],
          samples: &[f32],
          sample_rate: u32,
          frame_size: usize,
          hop_size: usize,
      ) -> Result<Vec<TrackSection>, AnalysisError>;
      ```

      **Algorithm (initial — keep simple):**
      1. Sliding-window kick density over kick band (40–120 Hz, threshold
         from `DubStabConfig`) at e.g. 4-second windows.
      2. Sliding-window RMS energy over the same windows.
      3. Threshold each independently into low/high (e.g. percentile-based,
         not absolute).
      4. Combine: high-energy + high-kick → MainGroove; high-energy +
         low-kick → Breakdown; low-energy → Intro (early) or Outro (late).
      5. Smooth transitions with a minimum-section-length filter
         (e.g. 8-second floor) to avoid sub-bar flicker.

      **Status:** implemented in the current branch and surfaced through
      `AnalysisResult.sections`. Unit tests cover synthetic sections. Real-track
      validation found and fixed overlapping boundary output; schema 11
      invalidates older cached section ranges. Broader validation on known
      breakdown-heavy tracks is still useful.

- [x] **S2. Retrofit dub_stab to use sections.** Filter
      `beat_relative_offset_histogram` per-bar histograms to MainGroove bars
      when available; fall back to full-track aggregation otherwise.

      **Status:** implemented in the current branch with `rate_basis` reporting.
      Initial real-track comparison shows MainGroove filtering gives a better
      denominator, but does not make dub-stab evidence independently
      discriminative enough for classifier use.

**Note for A2/A4/A5 below:** each new DSP feature should accept an optional
`sections: &[TrackSection]` parameter and aggregate stats over MainGroove
sections only when supplied. Treat the full track as a single MainGroove
when sections are unavailable, so the old path keeps working.

## Phase 3 — New DSP features (in dependency order)

Pick **one at a time**, with validation against `genre_verified` corpus before
moving to the next. Each ships behind its own commit but classifier wiring
waits until phase 4.

- [x] **A2. Kick-pattern detector.**
      Plan: [`kick-pattern-detector-plan.md`](kick-pattern-detector-plan.md).
      Discriminates Electro and broken-beat from straight 4/4 Techno. Sibling
      of A1 — shares the `detect_band_onsets` primitive. Highest expected
      classifier-accuracy lift per the master doc.

      **Status:** implemented detector-only with Stratum schema `17`.
      `StratumResult` now surfaces `kick_pattern`, confidence, kicks-per-bar,
      onset count, rate basis, and a flattened 4x16 histogram. Classifier rules
      do **not** consume the detector yet; `AudioFeatures` only carries the
      cached values for validation. The detector deduplicates raw low-band
      onsets into one beat-level anchor per bar/beat before reporting density,
      so bassline movement and kick tails do not inflate kicks-per-bar.
      Validation widened the default kick band to 40-200 Hz after 40-120 Hz
      missed acoustic/disco kicks and produced confident false `Sparse` labels.
      Early listening shows `broken_beat` is a rhythmic-shape flag, not a hard
      genre veto: some darker/upbeat Techno tracks legitimately match it.
      `Halftime` stays detector-only until we validate it against Breakbeat by
      ear; do not add it to the genre taxonomy yet.

      **Acceptance:** validation on a hand-picked corpus of Electro,
      broken-beat, and 4/4 Techno tracks. Detector should classify each
      cleanly. Don't merge into the classifier yet.

- [ ] **A4. Sub-rumble detector.**
      Plan: [`sub-rumble-and-sidechain-plan.md`](sub-rumble-and-sidechain-plan.md)
      (paired with A5). Distinguishes Berlin-school sub-heavy techno from
      offbeat-stab dub techno where the bass is conventional.

      **Acceptance:** detector returns a continuous "sub presence" score;
      validated on a corpus split between sub-heavy (Surgeon, Sleeparchive)
      and not-sub-heavy (early Basic Channel, Dub Taylor) tracks.

- [ ] **A5. Sidechain depth detector.**
      Plan: [`sub-rumble-and-sidechain-plan.md`](sub-rumble-and-sidechain-plan.md).
      Sidechain compression is a strong signal for modern productions vs
      pre-2010 work. Useful for several genre boundaries (Tech House,
      progressive, modern dub).

      **Acceptance:** detector returns a "sidechain depth" score in
      [0, 1]; validated on corpus split between heavily-sidechained tracks
      (modern house / progressive) and minimal-compression tracks (Basic
      Channel-era dub).

## Phase 4 — Wire all features into classifier (single pass)

After F0, B1–B4, A2, A4, A5 ship, do **one** classifier-tuning pass:

- [ ] **W1.** Add new feature signatures to `audio_profile.rs` Fisher
      discriminant.
- [ ] **W2.** Re-run prototype training on `genre_verified` (currently 604
      tracks, 24 of which are Dub Techno).
- [ ] **W3.** Re-classify the full library; diff against current
      classifications. Investigate disagreements — usually a sign the new
      feature is finding mis-tagged tracks (good!) or the discriminant is
      over-fit to a sub-genre (re-tune).
- [ ] **W4.** Document the new feature weights in
      `genre-classification-implementation.md`.

      **Acceptance:** measurable accuracy improvement on the verified
      corpus. Genre-tag disagreements are reviewable and the user signs off
      on each.

## Things explicitly skipped or deferred

- **A3 band-split spectral flux.** Overlaps with `detect_band_onsets`
  already in stratum-dsp. Reconsider only if A2/A4/A5 turn out insufficient.
- **C1–C5 conjunctive templates** (`conjunctive-templates-plan.md`). Gated
  on at least 3 of A1–A5 shipping AND validation. Defer to a later milestone.
- **D1–D2 training improvements**
  (`prototype-training-improvements-plan.md`). The current Fisher pipeline
  works; revisit only if Phase 4 reveals systematic issues.
- **D3–D4 verification feedback loop**
  (`verification-feedback-loop-plan.md`). Started only with the read-only
  `calibration_coverage` tool. The verification registry (`verify_track`,
  `verify_tracks`, calibration union source) remains a separate workstream.

## Carry-over notes from v0.27.0 testing

For Phase 4 tuning, remember:

1. **Hi-hat false positive class:** A1 (chord-stab) matches tracks with
   offbeat hi-hats but no chord stabs (Tzusing techno scored 0.65). Mitigation
   is HPSS-based tonal/percussive separation on the stab band — defer until
   we see this hurt the classifier in practice. Document the limitation in
   the dub_stab module docstring.
2. **Genre-verified label noise.** Talis (Donato Dozzy / Peter Van Hoesen)
   was tagged Dub Techno but exhibits on-beat stabs typical of deep/minimal
   techno. Expect 5–10% of `genre_verified` to be edge cases the classifier
   will "disagree" with — investigate disagreements rather than
   auto-correcting them.
3. **The strongest single signal across the 24-track corpus was
   `dub_stab_onset_rate`.** Pure dub techno had 1.5–4 stabs/s;
   non-dub-techno tracks below 1.0 stab/s. Useful for the Fisher discriminant
   weighting.

## Status legend

- [ ] Not started
- [~] In progress
- [x] Done
- [-] Skipped / deferred

Update this doc when starting/finishing each item; don't squash items —
keep them visible with status changes for diff readability.
