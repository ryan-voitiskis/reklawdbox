# Plan 006: Keep variable-tempo beat grids phase-continuous and unique

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/src/features/beat_tracking/hmm.rs stratum-dsp/src/features/beat_tracking/mod.rs stratum-dsp/src/features/beat_tracking/tempo_variation.rs stratum-dsp/tests/integration_tests.rs src/audio.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition. Changes made by DONE Plan 005 (and
> its dependencies) are expected in shared files: reconcile their committed
> result with this plan and continue when the intent matches. Unrelated drift
> is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/005-infer-meter-and-downbeat-phase.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The HMM describes tempo as a state that may change over time, but both its emissions
and extracted timestamps use `start + frame_index * current_state_interval`. Changing
state therefore re-anchors the whole historical grid instead of accumulating phase,
which can move a beat backward or create a jump. The later variable-tempo refinement
uses 50%-overlapping segments, appends every segment's complete beat list, and only
sorts it, so overlap beats can be duplicated or nearly duplicated.

This plan makes HMM timing cumulative along the selected path, gives overlapping
segments exclusive output ownership, and enforces finite, strictly ascending,
deduplicated grids before downstream meter and bar generation. It retains the safe
fallback to the original HMM grid when refinement cannot satisfy those invariants.

## Current state

- `stratum-dsp/src/features/beat_tracking/hmm.rs` — five-state tempo HMM, emission
  calculation, Viterbi pass, and beat extraction.
- `stratum-dsp/src/features/beat_tracking/tempo_variation.rs` — creates overlapping
  analysis segments.
- `stratum-dsp/src/features/beat_tracking/mod.rs` — re-tracks each segment and merges
  the results before meter/phase selection from Plan 005.
- `stratum-dsp/tests/integration_tests.rs` — portable public-pipeline invariants from
  Plan 001. Its bootstrap beat-order check is intentionally non-decreasing because the
  120 BPM fixture exposes this plan's duplicate-output defect; this plan must strengthen
  that helper to strictly ascending.
- `src/audio.rs` — cache schema version for changed beat-grid semantics.

Emission timing is re-anchored independently for every state and frame
(`hmm.rs:246-281`):

```rust
let start_time = self.onsets[0];
let end_time = self.onsets[self.onsets.len() - 1];
let nominal_beat_interval = 60.0 / self.bpm_estimate;
let num_frames = ((end_time - start_time) / nominal_beat_interval).ceil() as usize + 1;

for t in 0..num_frames {
    for s in 0..NUM_STATES {
        let state_beat_interval = 60.0 / state_bpms[s];
        let expected_beat_time = start_time + (t as f32 * state_beat_interval);
```

Extraction repeats the same non-cumulative formula after Viterbi has selected state
transitions (`hmm.rs:417-450`):

```rust
for (t, &state) in best_path.iter().enumerate() {
    let emission_prob = emission_matrix[t][state];
    if emission_prob > EMISSION_THRESHOLD {
        let state_beat_interval = 60.0 / state_bpms[state];
        let beat_time = start_time + (t as f32 * state_beat_interval);
        // ...
        beats.push(BeatPosition {
            beat_index: 0,
            time_seconds: beat_time,
            confidence,
        });
    }
}
```

Tempo windows deliberately overlap by half and use inclusive endpoints
(`tempo_variation.rs:134-149,204-205`):

```rust
let segment_duration = (total_duration / 4.0).clamp(4.0, 8.0);
let overlap = segment_duration * 0.5;

while current_start < beats[beats.len() - 1] {
    let segment_end = (current_start + segment_duration).min(beats[beats.len() - 1]);
    let segment_beats: Vec<f32> = beats
        .iter()
        .filter(|&&beat| beat >= current_start && beat <= segment_end)
        .copied()
        .collect();
    // ...
    current_start += segment_duration - overlap;
}
```

Every overlapping result is appended, then merely sorted (`beat_tracking/mod.rs:167-213`):

```rust
for segment in &tempo_segments {
    // ... each segment contributes its full inclusive range ...
    refined_beats.extend(segment_beats);
}

if !refined_beats.is_empty() {
    refined_beats.sort_by(|a, b| a.time_seconds.partial_cmp(&b.time_seconds).unwrap());
    beat_positions = refined_beats;
}
```

Follow existing conventions: `AnalysisError` for invalid public inputs, log-and-fallback
for an optional refinement failure, deterministic in-memory fixtures, and no mandatory
private audio.

## Commands you will need

| Purpose               | Command                                                                                    | Expected on success |
| --------------------- | ------------------------------------------------------------------------------------------ | ------------------- |
| HMM unit tests        | `cargo test -p stratum-dsp features::beat_tracking::hmm::tests -- --nocapture`             | exit 0              |
| Tempo unit tests      | `cargo test -p stratum-dsp features::beat_tracking::tempo_variation::tests -- --nocapture` | exit 0              |
| Beat-grid unit tests  | `cargo test -p stratum-dsp features::beat_tracking::tests -- --nocapture`                  | exit 0              |
| Integration           | `cargo test -p stratum-dsp --test integration_tests -- --nocapture`                        | exit 0              |
| Full suites           | `cargo test -p stratum-dsp --no-fail-fast && cargo test -p reklawdbox --no-fail-fast`      | exit 0              |
| Format/lint           | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`               | exit 0              |
| Repository formatting | `dprint check`                                                                             | exit 0              |

## Scope

**In scope** (the only source files you should modify):

- `stratum-dsp/src/features/beat_tracking/hmm.rs`
- `stratum-dsp/src/features/beat_tracking/mod.rs`
- `stratum-dsp/src/features/beat_tracking/tempo_variation.rs`
- `stratum-dsp/tests/integration_tests.rs`
- `src/audio.rs` only for a live-value `STRATUM_SCHEMA_VERSION` increment and its test
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- BPM candidate generation, meter/accent scoring, or downbeat-phase selection. Preserve
  Plan 005's interfaces and semantics.
- Redesigning the Bayesian estimator or expanding the HMM beyond its existing five
  adjacent tempo states.
- External Rekordbox grids; they bypass this generated-grid path and must remain exact.
- A new public serialized output field for tempo curves.
- Private audio/corpus tuning, ML code, or direct `master.db` writes.
- Relaxing strict monotonicity to allow duplicate beats.

## Git workflow

- Branch: `codex/006-stabilize-variable-tempo-beat-grids`
- Use Conventional Commits. Suggested logical commits:
  1. `test(stratum): cover variable-tempo grid continuity`
  2. `fix(stratum): preserve phase across tempo changes`
- Do not push or open a PR unless instructed.
- Increment `STRATUM_SCHEMA_VERSION` by one from its live numeric value after semantic
  output changes; do not restore the planned-at value `18`.

## Steps

### Step 1: Add failing continuity and overlap regressions

Add deterministic unit fixtures before changing the algorithm:

1. In `hmm.rs`, create onset times whose intervals change gradually across the HMM's
   supported ±10% state range. Assert returned beat times are finite, strictly
   ascending, align to a unique nearest onset within `TIMING_TOLERANCE_S`, and change
   interval smoothly rather than jumping by a multiple of the elapsed frame count.
2. Add a direct path-timing test (inside the private test module) with a known state
   sequence such as nominal → +5% → +10%. Assert each timestamp equals the previous
   timestamp plus the selected state's interval, within float tolerance. Make the
   +10% tail long enough to require more rows than the current nominal-BPM
   `num_frames`; assert the final pulse is covered within one fastest-state interval
   and is not truncated at the old horizon.
3. In `tempo_variation.rs`/`mod.rs`, create two overlapping segments that both produce a
   beat at the shared region. Assert the merge output contains one beat, choosing the
   higher-confidence candidate; equal confidence must use a deterministic tie rule.
4. Cover exact endpoints and near-duplicates on either side of a boundary.
5. Cover non-finite and non-monotonic candidate lists; the validator must reject them
   rather than invoking `partial_cmp(...).unwrap()`.

Before the fix, at least the cumulative-time or duplicate-output regression must fail.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking -- --nocapture` → exits non-zero in
one of the newly added regressions before implementation. If all pass unchanged, STOP
and report drift rather than rewriting the tests to force a failure.

### Step 2: Make Viterbi cells carry cumulative beat time

Refactor the HMM dynamic program so the emission for a candidate state transition is
evaluated at a time derived from the winning predecessor, not from absolute frame
index. Use a private cell/trace representation containing at least:

- best log probability
- predecessor state
- cumulative expected beat time
- emission probability/alignment used for that cell

Initialize all states at the first onset. For each later beat step and each destination
state, evaluate every permitted predecessor:

```text
candidate_time = predecessor_time + 60 / destination_bpm
candidate_score = predecessor_log_score
                + log(transition_probability)
                + log(emission(candidate_time))
```

Choose the highest finite score and store that candidate's time and predecessor. Reuse
one helper for nearest-onset distance/emission; do not precompute emissions using
`start + t * state_interval`. Backtrack both state and stored time from the winning
final cell.

Do not retain the nominal-BPM `num_frames` as the iteration horizon. Derive a checked
hard bound from the shortest supported state interval:

```text
min_interval = 60 / max(state_bpms)
max_interval = 60 / min(state_bpms)
max_steps = ceil((end_time - start_time + max_interval) / min_interval) + 1
```

Build rows only until `max_steps` or until every reachable successor would be later
than `end_time + max_interval`. Record terminal candidates at their actual row once a
path reaches the final-onset region; select/backtrack from the best real terminal cell,
not blindly from the last allocated row. Because terminal paths can contain different
beat counts, compare their mean log score per scored transition first, then total log
score and stable state order as deterministic tie-breakers. This avoids penalizing a
faster valid path merely for containing more beats.

Retain log-space probability arithmetic and `EMISSION_FLOOR`. Use checked duration,
division, ceiling, and `usize` conversion for the horizon. Stop extending a path after
its candidate time is beyond the observable onset range by more than one maximum beat
interval. Never produce NaN/Infinity or allocate from an unbounded float-to-integer
cast.

**Verify**:

```bash
cargo test -p stratum-dsp features::beat_tracking::hmm::tests -- --nocapture
rg -n 'start_time \+ \(t as f32 \* state_beat_interval\)' stratum-dsp/src/features/beat_tracking/hmm.rs
```

Expected: tests exit 0; `rg` returns no matches.

### Step 3: Extract one strictly ordered beat path

Build `BeatPosition` values from the backtracked cell times and stored emissions.
Calculate confidence with the existing emission/alignment weighting, but reject/skips
cells below the existing threshold only if doing so cannot create duplicate positions.
After extraction:

- reject non-finite or negative times;
- sort only as a defensive check (the backtracked result should already be ordered);
- merge positions closer than
  `min(TIMING_TOLERANCE_S, 0.20 * (60.0 / nominal_bpm))`;
- preserve the higher-confidence position in a merge cluster;
- require strict `a < b` for every adjacent pair.

Do not hide a phase discontinuity by sorting alone. A unit test must demonstrate that
the pre-sort path is cumulative and monotonic.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::hmm::tests -- --nocapture` → exit 0;
constant-tempo tests retain their existing intervals and variable-tempo tests pass.

### Step 4: Give each overlapping segment an exclusive output interval

Keep overlapping windows for estimating variation, but separate the **analysis range**
from the **output ownership range**. For adjacent analysis windows `current` and
`next`, set their ownership boundary to
`(current.end_time + next.start_time) / 2.0`. The current segment owns timestamps below
that boundary and the next owns timestamps at or above it; only the final segment may
include its right endpoint. Every timestamp in the track range must be owned by exactly
one segment.

When collecting original or re-tracked beats, retain only beats within that segment's
ownership interval. Continue to provide the complete analysis-window onset set to the
Bayesian/HMM estimator so boundary context is not lost. Do not change inclusive
analysis windows merely to mask duplicates.

Add unit tests that sample the start, every midpoint, and final endpoint and count
exactly one owner.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::tempo_variation::tests -- --nocapture` →
exit 0; ownership tests prove no gaps and no double ownership.

### Step 5: Merge and validate refined beats before replacing the original grid

Centralize refined-output merging in a private helper in `beat_tracking/mod.rs`:

1. Flatten owned segment outputs.
2. Reject non-finite/negative candidates.
3. Sort with `f32::total_cmp` (or validate finiteness before `partial_cmp`) so no unwrap
   can panic.
4. Deduplicate candidates within
   `min(TIMING_TOLERANCE_S, 0.20 * nominal_beat_period)`, preferring higher confidence;
   on equal confidence keep the earlier timestamp.
5. Assert strict monotonicity and local intervals within
   `[0.45 * nominal_beat_period, 2.25 * nominal_beat_period]`. The upper bound admits
   one missing beat at roughly two periods plus modest drift, while larger
   discontinuities still reject the refinement. Unit-test an exact two-period gap as
   accepted and a gap above 2.25 periods as rejected so the missed-beat policy cannot
   drift back below its stated contract.

If the merged refinement is empty or violates continuity, log a warning and retain the
original pre-refinement `beat_positions`. Do not return a partially merged grid and do
not synthesize missing beats at boundaries.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::tests -- --nocapture` → exit 0;
duplicate, near-duplicate, boundary, one-missing-beat, and fallback tests pass.

### Step 6: Add a portable end-to-end variable-tempo fixture

In `integration_tests.rs`, first strengthen Plan 001's shared beat-order invariant from
non-decreasing to strictly ascending; its other finite/range/downbeat checks stay
unchanged. Then synthesize a pulse train that changes gradually between two tempos
within the tracker's supported range and has enough duration to create multiple tempo
segments. Reuse the strengthened result-invariant helper. Assert:

- analysis succeeds with a non-empty generated grid;
- all beats are finite and strictly ascending;
- no adjacent beat difference is within the merge tolerance;
- each detected beat maps to at most one synthesized pulse;
- the median interval in the later section moves in the expected direction relative
  to the early section.

Do not assert exact beat counts or timestamps. Run the test five times to detect
nondeterministic merge ordering.

**Verify**:
`for i in 1 2 3 4 5; do cargo test -p stratum-dsp --test integration_tests variable_tempo || exit 1; done` → all five runs exit 0.

### Step 7: Invalidate cached Stratum output

Generated beat grids and all grid-dependent features can change. Increment
`src/audio.rs::STRATUM_SCHEMA_VERSION` once from its live value and update the matching
exact-value test. Leave Essentia's version unchanged.

**Verify**:
`cargo test -p reklawdbox audio::tests::stratum_result_shape_matches_schema_version` →
exit 0 with a matching incremented constant/test.

### Step 8: Run full gates and inspect the diff

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p stratum-dsp --no-fail-fast
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
git diff --name-only
```

Expected: every command exits 0; only in-scope files are changed.

## Test plan

- HMM unit tests: constant tempo, known state transition sequence, gradual
  acceleration/deceleration, invalid/non-finite inputs, strict ordering.
- Segment unit tests: complete ownership partition, exact midpoint/endpoints, overlap
  context retained.
- Merge unit tests: exact duplicate, near duplicate, higher-confidence selection,
  stable tie, invalid candidate, invalid-refinement fallback.
- End-to-end test: deterministic gradual-tempo pulse train using only in-memory samples.
- Regression: all Plan 001 fixed-tempo and Plan 005 meter/phase tests remain green.
- External Rekordbox-grid equality tests must remain green and unmodified in behavior.

## Done criteria

All must hold:

- [ ] No HMM expected/extracted time uses `start + frame_index * current_interval`.
- [ ] A known state-transition path advances by cumulative selected intervals.
- [ ] The checked fastest-state horizon covers a sustained +10% tail beyond the old
      nominal frame count, and terminal selection uses the path's actual row.
- [ ] Overlap ownership assigns every boundary/range sample to exactly one segment.
- [ ] Merged generated beats are finite, non-negative, and strictly ascending.
- [ ] Plan 001's shared integration helper now requires strictly ascending beats; no
      non-decreasing bootstrap allowance remains.
- [ ] Duplicate/near-duplicate beats collapse deterministically to one candidate.
- [ ] Invalid refinement retains the original valid grid.
- [ ] The portable variable-tempo integration test passes five consecutive runs.
- [ ] Plan 005's meter/phase and external-grid tests still pass.
- [ ] `STRATUM_SCHEMA_VERSION` is incremented exactly once from its live starting value;
      `ESSENTIA_SCHEMA_VERSION` is unchanged.
- [ ] Format, dprint, workspace clippy, both crate suites, release build,
      `--version`, and `--help` exit 0.
- [ ] `git diff --check` exits 0 and no out-of-scope files changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plans 001 or 005 are incomplete or their tests are red.
- The HMM has been replaced or its state/emission API no longer matches the excerpts.
- The regression requires tempo changes outside the documented ±10% state space.
- Correct cumulative emissions require increasing state count, adding an external DSP
  dependency, or reproducing a paper beyond the bounded dynamic-program change here.
- Segment continuity cannot be achieved without changing Plan 005's meter/phase model.
- A proposed merge would fabricate beats or relax strict monotonicity.
- The fix would modify supplied external Rekordbox grids or write to `master.db`.
- A step fails twice or requires an out-of-scope file.

## Maintenance notes

- Any tempo state model must carry phase or cumulative time; tempo alone is not a
  complete beat-tracking state when transitions are allowed.
- Overlap is useful for estimation context but requires an explicit ownership rule for
  emitted values. Apply the same distinction to future segmented features.
- Keep merge tolerances derived from beat interval and bounded by timing tolerance;
  fixed magic seconds will behave differently across BPM ranges.
- Generated-grid semantic changes require `STRATUM_SCHEMA_VERSION` review. External
  Rekordbox grids remain authoritative and outside this refinement path.
