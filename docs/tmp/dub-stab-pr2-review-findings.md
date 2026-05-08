# PR 2 Review Findings — Dub-Stab Stage 1 + Stage 2

Aggregated from five parallel reviewers (code-reviewer, silent-failure-hunter, pr-test-analyzer, comment-analyzer, type-design-analyzer) over `stratum-dsp/src/features/dub_stab.rs`.

Each finding below is a **claim** that must be independently verified before action.

---

## P0 — claimed real bugs

### Finding 1: NaN beats silently corrupt the entire histogram

**Claim.** At `dub_stab.rs:203-210`, the `binary_search_by` predicate uses
```rust
b.partial_cmp(&onset_time).unwrap_or(std::cmp::Ordering::Less)
```
A `NaN` in `beat_grid.beats` therefore compares as "less than `onset_time`". The search lands at some arbitrary `beat_idx`, and the subsequent `beat_period = beats[beat_idx + 1] - beats[beat_idx]` produces `NaN`. The guard `if beat_period <= 0.0 { continue; }` at line 215 returns `false` for `NaN` (NaN compares false to all comparisons). Execution falls through to the histogram-update loop with a `NaN` offset, propagating `NaN` into every bin of `global` and `per_bar`.

**Asserted impact.** A single NaN in the supplied `BeatGrid.beats` makes the histogram entirely NaN-tainted with no error, no log, no panic.

**Verification needed.**
- Does `f32::NAN.partial_cmp(&x)` actually return `None`? (Yes, by std contract — but verify.)
- Does `NaN <= 0.0` actually evaluate to `false`? (Yes by IEEE-754 — verify.)
- Trace: a beat grid like `[0.0, NaN, 1.0, 1.5]` with `onset_time = 0.5` — what `beat_idx` does the search produce, and does the histogram in fact end up NaN?

**Proposed fix.** Validate `beats.iter().all(|b| b.is_finite())` once up front; return `AnalysisError::InvalidInput`. Add a regression test.

---

### Finding 2: Non-monotonic beat grid silently degrades

**Claim.** At `dub_stab.rs:215-217`, the per-onset `continue` on `beat_period <= 0.0` swallows a real upstream bug class. A duplicate or non-monotonic `BeatGrid.beats` (which the upstream beat tracker contractually shouldn't produce) silently drops every onset that happens to land in the bad interval. The histogram comes out near-empty with no diagnostic.

**Asserted impact.** A buggy beat tracker silently degrades dub-stab scores rather than failing loudly. Classic "fallback masks the underlying bug" pattern.

**Verification needed.**
- Confirm the existing tests do not exercise the `beat_period <= 0.0` guard (i.e., it is currently dead-code-on-the-happy-path).
- Confirm the `BeatGrid` contract (in `crate::analysis::result::BeatGrid`) is documented as ascending-strict, or at least that downstream code assumes it.
- Trace: a beat grid like `[0.0, 1.0, 1.0, 2.0]` (duplicate) — what does the current implementation produce vs. an outright validation error?

**Proposed fix.** Validate strict monotonicity once up front; demote the per-onset guard to `debug_assert!`.

---

## P1 — claimed test gap

### Finding 3: Circular wrap-around at offset ≈ 0.99 is not deterministically tested

**Claim.** The `if d > 0.5 { d = 1.0 - d; }` wrap-around at `dub_stab.rs:227-229` is core to Stage 2's circular soft-binning. The existing test `on_beat_quarters_peak_at_bin_zero` only weakly asserts `hist[0] > hist[16]` and `hist[0] > hist[8]` and depends on `time_to_frame` rounding to coincidentally produce wrap. A regression that broke the wrap (e.g. `if d > 0.5 { d = 0.5 }`, or removing the branch) would not fail the current suite.

**Asserted impact.** Critical test gap: the most distinctive piece of Stage 2 logic is not directly tested.

**Verification needed.**
- Re-read `on_beat_quarters_peak_at_bin_zero` — does it ever assert anything about `hist[31]`?
- Re-read `soft_binning_spreads_across_neighbour_bins` — does it exercise wrap-around?
- Mentally run the suggested regression `if d > 0.5 { d = 0.5 }` — would any test fail?

**Proposed fix.** Add a test that places a single onset at offset ≈ 0.99 of a beat and asserts both `hist[31]` and `hist[0]` are substantial, with each dominating `hist[15]` and `hist[16]`.

---

## P1 — claimed doc-accuracy issues

### Finding 4: `DubStabConfig` docstring overclaims provenance

**Claim.** At `dub_stab.rs:31-34`, the docstring says defaults are derived from `kick-bleed-investigation.md` Experiment 7 (5-track cross-validation). In reality only `kick_mask_window_ms = 80.0` and the symmetric mask shape come from that experiment. Specifically:
- `band_low_hz = 350.0`, `band_high_hz = 2000.0` come from the chord-stab plan §"Signal Definition", not the bleed investigation.
- `kick_band_low_hz = 40.0`, `kick_band_high_hz = 120.0` come from the plan's Stage 1 description; Experiment 7 itself analysed a 40–200 Hz kick band, so citing it as the provenance for the 120 Hz upper edge is incorrect.
- `onset_threshold_percentile = 0.85` is a project-wide inherited default, not derived from the experiment.

**Verification needed.**
- Open `docs/tmp/kick-bleed-investigation.md`; check whether the 350/2000/40/120 numbers are actually derived in the doc or just used as inputs.
- Open `docs/tmp/chord-stab-detector-plan.md`; check what it says about the band edges and percentile.

**Proposed fix.** Narrow the docstring to claim provenance only for the items actually derived: the symmetric ±80 ms `kick_mask_window_ms`.

---

### Finding 5: Rot-prone numbered-section references

**Claim.** Two doc comments cite `kick-bleed-investigation.md` by section number:
- `dub_stab.rs:31` — "Experiment 7"
- `dub_stab.rs:165` — "§6"

Both will rot if the investigation doc is restructured. Replace with stable descriptive phrases.

**Verification needed.**
- Are "Experiment 7" and "§6" the actual section numbers in the current investigation doc?
- Is there any other place in the file that uses the same pattern?

**Proposed fix.** Replace with phrases like "the 5-track real-audio cross-validation" and "the STFT centring section".

---

### Finding 6: `per_bar` post-grid attribution is asymmetric and undocumented

**Claim.** The docstring at `dub_stab.rs:172-175` states:
> onsets before the first bar are left out of the per-bar accounting (their contribution still lands in `global`).

Onsets *after* the last bar are not symmetrically dropped: `locate_bar` uses `partition_point(|&b| b <= onset_time)` which returns `bars.len()` for post-grid onsets, then `Some(i - 1) = Some(bars.len() - 1)` lumps them into the final bar. Future maintainers will assume symmetric drop-out and be wrong.

**Verification needed.**
- Open `dub_stab.rs:241-247` (the `locate_bar` body); trace the post-grid case.
- Construct: `bars = [0.0, 4.0]`, `onset_time = 100.0` — does `locate_bar` return `Some(1)` or `None`?
- Is there a test that exercises this case?

**Proposed fix.** Either document the asymmetry explicitly, or change `locate_bar` to return `None` when `onset_time` exceeds the last bar's expected end (requires knowing bar duration → may not be locally computable).

---

## P2 — type design

### Finding 7: `OffsetHistogram` should use `[f32; HISTOGRAM_BINS]`

**Claim.** `HISTOGRAM_BINS` is already a public const. Changing
- `global: Vec<f32>` → `[f32; HISTOGRAM_BINS]`
- `per_bar: Vec<Vec<f32>>` → `Vec<[f32; HISTOGRAM_BINS]>`

would encode the bin-count invariant at compile time, eliminate a class of length bugs Stage 3 would otherwise have to defend against, and cost almost nothing (no API breakage outside this PR — Stages 3–4 are not yet wired).

**Verification needed.**
- Are `global.len() == HISTOGRAM_BINS` and `per_bar[i].len() == HISTOGRAM_BINS` true at every public exit point?
- Is there any caller of `OffsetHistogram` outside `dub_stab.rs` and its tests? (PR 2 is internal-only per the module docstring.)
- Would the change affect Serde/clone/debug behaviour?

**Proposed fix.** Make the type substitution; update tests accordingly.

---

## P2 — diagnostics

### Finding 8: Early returns in `detect_kick_disjoint_stab_onsets` lose cause-attribution

**Claim.** Three zero-result paths return successfully with no log line:
- `dub_stab.rs:106-108` — `stab.is_empty()` (no stab-band onsets)
- `dub_stab.rs:117-119` — `kicks.is_empty()` (mask is a no-op)
- `dub_stab.rs:124-132` — fully-masked-out (already logs `kept.len()`)

A caller debugging `dub_stab_score = 0` cannot distinguish "no stabs" from "no kicks" from "all masked" without re-running with extra instrumentation.

**Verification needed.**
- Confirm that no `log::*` call exists on the two early-return paths.
- Confirm there is no caller that already discriminates these cases via return value.

**Proposed fix.** Add one-line `log::debug!` per path with the cause.

---

## Skip / not worth acting on (claims to confirm as not-bugs)

- `clamp(0.0, 1.0 - f32::EPSILON)` is correct and stays correct after Findings 1 and 2 are fixed. (silent-failure Finding 3 lower bound.)
- `mask_kick_coincident` sortedness `debug_assert!` — defensible but `detect_band_onsets` always returns sorted output, so internal callers are safe. (pr-test-analyzer Improvement #5.)
- The `kick_within_helper_handles_window_edges` test has a duplicate assertion at lines 423/425 — cosmetic only.
- Mass `BandHz` newtype refactor — defer to follow-up PR when the type is reused.
- `mask_frames` rounds to 0 for tiny windows — edge case; add a log only.

---

## Verifier instructions

For each finding, an independent verifier should:

1. Open the cited file/line range and read it.
2. Reproduce the cited behaviour (mentally trace the example, or write a one-shot test).
3. Confirm or refute the asserted impact.
4. Note any way the finding is overclaimed, underclaimed, or partly wrong.
5. Indicate whether the proposed fix is the minimum correct response.

A finding marked "verified" must have an independent traceback supporting it. A finding marked "refuted" must have a counter-example.
