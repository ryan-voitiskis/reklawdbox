# Set Builder Harmonic Query Plan — 2026-02-22

## Purpose

Capture the current product/algorithm requirements for improving set building with stronger harmonic reasoning, including pitch/key coupling behavior when Master Tempo is off.

This document is a planning artifact for the next implementation session.

## Confirmed Product Requirements

1. The set builder should support intentional harmonic modulations, but also allow conservative behavior when requested.
2. Before building a set, there should be a short planning conversation in plain English about:
   - Desired progression/curve
   - Allowed jump/modulation style
   - Set length and other key parameters
3. Runtime can be slower (up to hours if needed) if it materially improves signal quality and decision quality.
4. Controlled randomness is desired in final output candidates, while core scoring logic should remain deterministic.
5. Ordering quality is a higher priority than phrase/cue modeling at this stage.
6. DJs should be able to describe desired progression naturally in session (no rigid preset-only UX).
7. Do not rely on a hard `pitch_range_pct` user parameter in SOP. Instead:
   - Prefer transitions requiring low pitch adjustment (tracks near native BPM).
   - Still permit pitch adjustment as a creative tool for intentional effects.
8. Master Tempo behavior must be explicit at SOP start:
   - `master_tempo = on`: BPM can change while key remains fixed.
   - `master_tempo = off`: BPM change implies key transposition and must be scored accordingly.

## Harmonic Scoring Direction (Adopt)

Use MIREX-style weighted key relation scoring as the base relation matrix:

- Same key: `1.0`
- Fifth relation: `0.5`
- Relative major/minor: `0.3`
- Parallel major/minor: `0.2`
- Other: `0.0`

Planned adaptation for DJ transition ranking:

1. Compute compatibility on the candidate track's **effective key** (post tempo/pitch adjustment if Master Tempo is off).
2. Keep relation labels (`same`, `up_fifth`, `down_fifth`, `relative`, `parallel`, `other`) for explainability.
3. Add a modulation budget/policy layer so low-compatibility moves can be intentionally placed in desired phases.

## Current System Snapshot (Relevant Implementation)

1. Rekordbox metadata query path (SQLCipher `master.db`):
   - `src/db.rs`
   - Current query entrypoints are metadata-first (`search_tracks`, `get_playlist_tracks`, `get_tracks_by_ids`).
2. Cached signals store (`internal.sqlite3`):
   - `src/store.rs`
   - Tables: `audio_analysis_cache`, `enrichment_cache`, `broker_discogs_session`.
3. Set builder and scorer:
   - `build_set` and `score_transition` in `src/tools.rs`
   - Track profile assembly reads cache per track (`build_track_profile`).
4. Current limitation:
   - Candidate selection is metadata-first, then scoring over provided IDs.
   - There is no first-class transition-query API that directly returns next-track candidates using transposition-aware harmonic logic.

## Required Design Change: Transition Query Primitive

Introduce a transition-focused query surface (tool/API), conceptually:

`query_transition_candidates(from_track_id, context) -> ranked candidates`

Where `context` includes at least:

- Target phase/curve intent
- Target BPM strategy
- Master Tempo mode (`on`/`off`)
- Harmonic style (`conservative`/`balanced`/`adventurous`)
- Modulation policy/budget
- Optional pool constraints (genre, label, date range, etc.)

Returned payload per candidate should include:

- `track_id`, `title`, `artist`
- `required_bpm_adjustment` and signed pitch indication
- `effective_key` (when applicable)
- `harmonic_relation_label`
- Axis scores and composite score
- Explanatory rationale fields for UI/agent narration

## Master Tempo and Pitch/Key Coupling Rules

For candidate track `j` relative to a target tempo:

1. Determine required tempo ratio:
   - `r = target_bpm / bpm_j`
2. Always compute required adjustment magnitude for cost/penalty:
   - Higher absolute adjustment should be penalized by default.
3. Effective key handling:
   - If Master Tempo is `on`: effective key remains source key.
   - If Master Tempo is `off`: effective key is transposed by `12 * log2(r)` semitones.
4. Harmonic compatibility is evaluated against this effective key, not raw metadata key.

This makes harmonic ranking behavior match practical DJ mixing behavior on CDJs/controllers depending on Master Tempo usage.

## SOP Updates Needed

Update `docs/operations/sops/set-builder.md` to include an explicit pre-build configuration step:

1. Confirm Master Tempo intention (`on` or `off`).
2. Confirm whether the set should prioritize near-native BPM or allow stronger tempo moves.
3. Confirm modulation intent:
   - Conservative harmonic flow
   - Balanced
   - Intentional/adventurous modulations
4. Confirm set length and progression narrative in plain English.

The SOP should also instruct that the agent summarizes back the interpreted constraints before generating candidates.

## Implementation Plan (Phased)

### Phase 1: Planning + Config Contract

1. Add a formal planning/config object for set building (internal schema).
2. Include Master Tempo mode and modulation policy fields.
3. Add English-to-config interpretation layer with explicit confirmation summary.

### Phase 2: Transition Query Engine

1. Implement transposition-aware harmonic relation scoring with MIREX base matrix.
2. Add penalty term favoring lower required pitch adjustment by default.
3. Add modulation budget logic (phase-aware allowances for lower compatibility moves).
4. Expose a ranked transition-candidate tool response with explanations.

### Phase 3: Ordering Upgrade

1. Move from purely greedy next-pick to multi-path ordering (beam-style search).
2. Keep deterministic scoring, apply controlled randomness when selecting among top near-optimal paths.
3. Preserve reproducibility option via seed if needed.

### Phase 4: Calibration + Evaluation

1. Build evaluation harness using curated set examples and DJ feedback.
2. Compare old vs new on:
   - Harmonic coherence
   - Energy-curve adherence
   - Average required pitch adjustment
   - Subjective transition quality notes
3. Tune weights/policies per harmonic style profile.

## Deferred Scope (Intentionally Not In First Iteration)

1. Phrase-aware and cue-point-aware transition modeling.
2. Full MIP/global optimization approach.
3. Preset-only UX that replaces conversational control.

## Open Questions for Next Session

1. What exact default policy should represent "near-native BPM" (penalty curve shape and strength)?
2. How should modulation budget be expressed (count-based, section-based, or score-budget)?
3. Should the final tool output include only top-N candidates or a full scored frontier for agent reasoning?
4. Which explanation fields are mandatory for DJ trust in recommendations?
5. Should conservative mode hard-block certain harmonic relations or only strongly penalize them?

## Next Session Start Checklist

1. Confirm config schema fields and defaults.
2. Confirm Master Tempo semantics in scoring implementation.
3. Implement transition query primitive.
4. Wire set builder ordering to consume transition query results.
5. Run one Deep Techno regression test and compare against prior 12-track example.
