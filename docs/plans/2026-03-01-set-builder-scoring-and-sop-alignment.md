# Set Builder: Scoring Improvements & SOP Alignment — 2026-03-01

Follows up on `docs/operations/reports/2026-02-22-set-builder-harmonic-query-plan.md`.
That plan's Phases 1-3 are substantially implemented. This plan covers the
remaining gaps: scoring refinements (Category B) and SOP documentation alignment
(Category A).

---

## Category B: Scoring Improvements (Code Changes)

Code changes land first so the SOP documents the final state.

### B1. Adjustment notifications in response JSON

Add a top-level `adjustments` array to `TransitionScores`, only serialized when
non-empty. Each entry:

```rust
struct ScoreAdjustment {
    kind: &'static str,        // "harmonic_gate", "bpm_drift", "genre_streak", etc.
    delta: f64,                // signed composite impact
    composite_without: f64,    // what composite would have been
    reason: String,            // human-readable for agent narration
}
```

Covers: harmonic gate (0.5× or 0.1×), BPM drift (0.7×), genre streak (+0.1),
genre early switch (-0.1), phase boundary boost (+0.1), sustained peak (+0.05).

Zero overhead for clean transitions (field omitted entirely). The `composite_without`
field is the key insight — seeing `composite: 0.328, composite_without: 0.656`
tells the agent "this transition would be decent if not for the key clash."

**Why not alternatives:**

- Per-axis `notes`: the two biggest penalties (harmonic gate, BPM drift) operate
  on the composite, not individual axes.
- Prose `rationale`: harder for agent to parse programmatically.
- Separate `penalties`/`bonuses` arrays: sign on `delta` conveys direction.

**Files:** `scoring.rs` (ScoreAdjustment struct, collect in
`score_transition_profiles` and `build_candidate_plan`/`_beam`),
`sequencing_handlers.rs` (no changes — uses `to_json()` which auto-includes),
`tests.rs` (assertions on adjustment presence/absence).

### B2. Modulation budget: keep threshold-only (no change)

Current per-transition threshold gate is sufficient. The AI agent can inspect
results and re-run with tighter constraints interactively. Budget state would
bloat beam search (`BeamState` would need `remaining_clashes: u32`, breaking
deduplication) and add a parameter DJs can't reason about. DJs think
per-transition ("can I get away with this mix?"), not per-set ("I'll allow
exactly 2 clashes").

### B3. BPM scoring: smooth exponential curve

Replace the 5-step `if/else` in `score_bpm_axis` with a continuous curve:

```rust
fn score_bpm_pct(pct: f64) -> f64 {
    (-0.019 * pct * pct).exp()
}
```

k = 0.019 was fitted to match the existing anchor points:

| pct   | exp(-0.019 × pct²) | Old stepped |
| ----- | ------------------ | ----------- |
| 0.0%  | 1.000              | 1.0         |
| 0.5%  | 0.999              | 1.0         |
| 1.0%  | 0.998              | 1.0         |
| 1.5%  | 0.996              | 1.0         |
| 2.0%  | 0.993              | 0.85        |
| 3.0%  | 0.843              | 0.85        |
| 5.0%  | 0.621              | 0.6         |
| 8.0%  | 0.297              | 0.3         |
| 12.0% | 0.064              | 0.1         |

**Why exponential over piecewise linear:** BPM matching perception is genuinely
continuous. The exponential naturally scores near-1.0 below 1.5% (0.996)
while still correctly preferring 0% over 1.4%. No artificial flat zone — if
BPM measurement has tolerance, that's handled at the measurement layer, not
baked into the scoring curve. At the scoring layer, "closer is better with
diminishing returns" is the honest model.

**Why not stepped (current):** 2.9% and 3.1% produce a 0.25 score gap for a
perceptually negligible difference. This distorts beam search ranking.

Labels assigned by bracket for readability:

- < 2% → Seamless
- < 4% → Comfortable
- < 6% → Noticeable
- < 9% → Creative transition needed
- ≥ 9% → Jarring

**Files:** `scoring.rs` (`score_bpm_axis`), `tests.rs` (update threshold
assertions to use curve values).

### B4. Conservative harmonic penalty: 0.1× (from 0.5×)

Change `HARMONIC_PENALTY_FACTOR` from a constant to a per-style function:

```rust
fn harmonic_penalty_factor(style: HarmonicMixingStyle) -> f64 {
    match style {
        Conservative => 0.1,   // effectively blocks; only wins as last resort
        Balanced     => 0.5,   // current behavior
        Adventurous  => 0.5,
    }
}
```

**Why:** 0.5× is too weak for Conservative. A clash with perfect BPM/energy/genre
scores ~0.4 (0.8 × 0.5), beating a harmonically perfect transition with mediocre
rest (~0.35). With 0.1×, the clash scores ~0.08 — can't win unless it's literally
the only remaining candidate.

**Why not hard block:** Pool exhaustion is real. By track 8 of a 20-track pool,
remaining tracks may have no key-compatible options. Hard block silently
truncates the set. 0.1× preserves the escape hatch.

**Files:** `scoring.rs` (replace constant with function, update call site at
line ~740), `tests.rs` (update `harmonic_style_conservative_penalizes_poor_transitions`).

### B5. Scoring evaluation harness

New file `src/tools/eval_scoring.rs` as a `#[cfg(test)]` module, following the
`eval_routing.rs` pattern (threshold-based quality gates with structured
reporting).

**Synthetic pools (no DB required):**

- `pool_camelot_walk` — 8 tracks forming a perfect Camelot walk (8A→9A→...→3A).
  Tests that the sequencer finds the optimal harmonic path.
- `pool_adversarial` — 6 tracks with hostile distributions (random keys, spread
  BPMs, mixed genres). Tests graceful degradation.
- `pool_iso_key_bpm` — all same key/BPM, forces differentiation on
  energy/genre/brightness.
- `pool_realistic_club` — 20 tracks spanning 3 genre families, realistic
  distributions.

**Quality gate thresholds:**

| Metric               | Threshold | What it catches            |
| -------------------- | --------- | -------------------------- |
| Mean composite       | ≥ 0.65    | Overall quality regression |
| Min composite        | ≥ 0.30    | Single terrible transition |
| Composite variance   | ≤ 0.08    | Consistency                |
| Harmonic coherence % | ≥ 50%     | Key scoring changes        |
| Energy fidelity %    | ≥ 40%     | Phase requirement changes  |
| Max pitch adjustment | ≤ 8%      | BPM threshold changes      |
| Determinism          | = true    | Tiebreaking bugs           |

**Additional tests:**

- Beam search ≥ greedy quality (best beam mean composite ≥ greedy - 0.01)
- Priority axis shift verification (Harmonic priority → better key scores)
- Sensitivity smoke test (vary one constant, assert monotonic behavior)

**Not building:** parameter sweep scripts, gradient optimization, multi-objective
Pareto — all overkill at this stage. The harness gives regression confidence;
tuning is manual with metrics as feedback.

**Files:** new `src/tools/eval_scoring.rs`, one line added to `src/tools/mod.rs`.
Runs in CI automatically via existing `cargo test`.

---

## Category A: SOP Alignment (Documentation)

Bring `docs/operations/sops/set-builder.md` into alignment with the actual
implementation. The SOP was written before several features landed and now omits
or misrepresents implemented capabilities.

### A1. Add pre-build configuration dialog to Step 1

Step 1 currently asks about duration, genre, BPM range, energy curve, priority,
starting track, and exclusions. It needs to also cover:

**Add to "Ask the user" block:**

```
Master Tempo?        [on (default) / off — affects key transposition when pitching]
Harmonic style?      [conservative / balanced (default) / adventurous]
BPM drift tolerance? [default 6% — max BPM wander from opening track]
BPM trajectory?      [optional — e.g., "start 122, peak at 130" → bpm_range]
```

**Add to defaults table:**

| Parameter           | Default                       |
| ------------------- | ----------------------------- |
| Master Tempo        | on                            |
| Harmonic style      | balanced                      |
| BPM drift tolerance | 6%                            |
| BPM trajectory      | None (no trajectory planning) |

**Add explanatory notes:**

- **Master Tempo on** = CDJ/controller pitch-locks the key. BPM changes don't
  affect harmonic compatibility. Modern default.
- **Master Tempo off** = pitching a track changes its key. Harmonic scoring
  accounts for the transposition automatically.
- **Conservative** harmonic style: only Perfect, Adjacent (±1), and Mood shift
  (A↔B) pass without heavy penalty (0.1× composite for others).
- **Adventurous** loosens harmonic gates during Build and Peak phases.
- **BPM trajectory** enables a phase-aware BPM ramp: Warmup holds start_bpm,
  Build ramps linearly to end_bpm, Peak holds end_bpm, Release ramps back.

**Add confirmation step:** After collecting parameters, the agent must summarize
the interpreted constraints back to the user before proceeding to Step 2.

### A2. Add `query_transition_candidates` to tool tables and Step 4

**Add to "New tools needed" table:**

| Tool                          | Purpose                                                                                                               |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------- |
| `query_transition_candidates` | Rank pool tracks as transition candidates from a reference. Context-aware (BPM target, energy phase, harmonic style). |

**Update Step 4 "suggest" operation** to call `query_transition_candidates` with:

- `from_track_id` = track at position N-1
- `pool_track_ids` = remaining pool (minus tracks already in set)
- `energy_phase` = phase at position N
- `target_bpm` = trajectory BPM at position N (if bpm_range set)
- `master_tempo` and `harmonic_style` from session config

Then filter results to also score well against position N+1 (if exists).

### A3. Replace deprecated `candidates` with `beam_width`

Update Step 3 tool call example and "How build_set works internally" section:

- `beam_width=1`: greedy single-path (fast, good baseline)
- `beam_width≥2`: beam search exploring N parallel paths, keeping top N by mean
  composite at each step, deduplicating identical sequences
- Default beam_width is 3
- Guidance: use beam_width=1 for quick previews, 3-5 for final candidates

### A4. Fix inaccurate scoring tables

**Key Compatibility:** SOP says Extended=0.5, Energy diagonal=0.4. Code has
0.45 and 0.55. Fix to match.

**BPM Compatibility:** SOP uses absolute BPM deltas with stepped scores.
Replace with the exponential curve documentation from B3.

### A5. Add Master Tempo and pitch shift documentation

New section after Composite Score covering:

- Pitch shift formula: `round(12 × log₂(target_bpm / native_bpm))`
- Camelot transposition: each semitone = +7 positions mod 12, letter unchanged
- Worked example
- Master Tempo on (default) = key unchanged

### A6. Update tool spec examples

**`score_transition`:** Add `master_tempo` and `harmonic_style` to request.
Add `effective_to_key`, `pitch_shift_semitones`, and `adjustments` to response.

**`build_set`:** Replace `candidates` with `beam_width`. Add `master_tempo`,
`harmonic_style`, `bpm_drift_pct`. Add `play_at_bpm`, `pitch_adjustment_pct`,
`effective_key` to per-track response. Add `beam_width`, `bpm_trajectory` to
top-level response.

### A7. Add `query_transition_candidates` tool spec

Full request/response documentation with all params and example JSON.

### A8. Document post-composite adjustments

New section covering:

- Harmonic gate (style-dependent: 0.1× Conservative, 0.5× Balanced/Adventurous)
- BPM drift penalty (0.7× when exceeding position-proportional budget)
- Genre stickiness (+0.1 streak bonus, -0.1 early switch penalty)
- Phase boundary boost (+0.1 energy) and sustained peak boost (+0.05)
- Note that these are now surfaced via the `adjustments` array (B1)

### A9. Update success criteria

Add checkboxes for implemented features not yet listed.

---

## Implementation Order

### Phase 1 — Code changes (Category B):

1. **B3:** BPM exponential curve — small, self-contained, improves beam search
   quality immediately.
2. **B4:** Conservative penalty 0.1× — 5 lines, immediate correctness fix.
3. **B1:** Adjustment notifications — ~50 lines, enables agent explainability.
4. **B5:** Evaluation harness — ~250 lines, regression safety net for B1-B4.

### Phase 2 — SOP alignment (Category A):

5. **A4:** Fix scoring table inaccuracies — quick, high impact on accuracy.
6. **A1:** Pre-build config dialog — core alignment gap.
7. **A2 + A7:** `query_transition_candidates` spec and Step 4 integration.
8. **A3:** Replace `candidates` with `beam_width`.
9. **A5 + A8:** Master Tempo docs + post-composite adjustments.
10. **A6:** Update tool spec examples.
11. **A9:** Update success criteria.

---

## Open Questions Resolved

From the 2026-02-22 plan:

1. **Near-native BPM penalty curve:** Smooth exponential `exp(-0.019 × pct²)`.
   No flat zone — "closer is better" at all ranges. (B3)
2. **Modulation budget format:** Keep threshold-only. No budget. (B2)
3. **Top-N vs. full frontier:** Keep top-N (default 10, max 50). Agent can
   request more via `limit` param. Full frontier adds noise without value.
4. **Mandatory explanation fields:** `adjustments` array on scored transitions,
   only when non-empty. (B1)
5. **Conservative hard-block vs. penalty:** Stronger penalty (0.1×), no hard
   block. Pool exhaustion is a real failure mode. (B4)
