# Plan 001: Make Stratum's synthetic success paths fail loudly

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/tests/integration_tests.rs`
> If the in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: tests
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The portable integration suite synthesizes audio in memory, but its principal BPM,
beat-grid, and key checks are nested in `if` blocks. A regression that returns no BPM,
no beats, or zero key confidence therefore prints a diagnostic and passes. This plan
turns the already-working BPM and grid paths into unconditional oracles, creates
reusable invariant assertions for later DSP plans, and keeps the suite independent of
private audio files.

This is deliberately a test-only plan. It must characterize behavior that succeeds at
the planned commit; it must not change DSP algorithms or lock known key/meter defects
into assertions. Plans 004, 005, and 006 build their algorithm-specific regressions on
top of this reliable baseline.

## Current state

- `stratum-dsp/tests/integration_tests.rs` — portable end-to-end tests and all
  synthetic fixtures used by this plan.
- The repository explicitly requires synthetic in-memory DSP fixtures; do not add
  mandatory tests that depend on a local music library.

The 120 BPM test currently makes the entire useful oracle optional
(`stratum-dsp/tests/integration_tests.rs:80-94`):

```rust
// Phase 1B: BPM detection should work
// For fixed-tempo synthetic audio, we can use tighter tolerance (±2 BPM)
if result.bpm > 0.0 {
    assert!(
        (result.bpm - 120.0).abs() < 2.0,
        "BPM should be close to 120 (±2 BPM tolerance), got {:.2}",
        result.bpm
    );
    assert!(
        result.bpm_confidence > 0.0,
        "BPM confidence should be positive"
    );

    // Phase 1C: Beat tracking should work
    if !result.beat_grid.beats.is_empty() {
```

The 128 BPM test repeats the same conditional structure
(`stratum-dsp/tests/integration_tests.rs:157-171`):

```rust
// Phase 1B: BPM detection should work
// For fixed-tempo synthetic audio, we can use tighter tolerance (±2 BPM)
if result.bpm > 0.0 {
    assert!(
        (result.bpm - 128.0).abs() <= 2.0,
        "BPM should be close to 128 (±2 BPM tolerance), got {:.2}",
        result.bpm
    );
    assert!(
        result.bpm_confidence > 0.0,
        "BPM confidence should be positive"
    );

    // Phase 1C: Beat tracking should work
    if !result.beat_grid.beats.is_empty() {
```

The tonal test can also pass without detecting a key
(`stratum-dsp/tests/integration_tests.rs:219-243`):

```rust
// Phase 1D: Key detection should work
// C major scale should be detected as C major (Key::Major(0))
use stratum_dsp::analysis::result::Key;
if result.key_confidence > 0.0 {
    // ... assertions ...
} else {
    println!("C major scale test: Key detection failed or low confidence, ...");
}
```

At `e6eb382`, the focused suite reports non-zero BPM and non-empty grids for the
120/128 BPM fixtures, while the C-major fixture reports zero key confidence. Remove
the conditional passes for the successful tempo paths now. Do **not** make the C-major
expectation unconditional in this plan; replace its misleading success-test name and
document it as a smoke-only path until Plan 004 supplies a deterministic mode oracle.

The strengthened baseline also exposes an existing Plan 006 defect: the 120 BPM
fixture contains equal consecutive generated beat timestamps (`6.491912` at indices
20 and 21). Plan 006 owns cumulative HMM timing, overlapping-window ownership, and
generated-grid deduplication, but it depends transitively on this test-only plan.
Accordingly, this plan requires generated beats to be finite, non-negative, and
non-decreasing without asserting that an equality exists. Downbeats and bars remain
strictly ascending. Plan 006 must strengthen the shared beat invariant from
non-decreasing to strictly ascending after it fixes the producer. This characterizes
the live baseline without freezing the duplicate as expected output or pulling an
algorithm change into this plan. Supplied external grids are not relaxed: their public
configuration contract already requires strictly ascending beats/bars, so the two
external-grid tests must retain an explicit strict-order assertion. Generated
duplicates may continue to produce the existing optional dub-stab/kick-pattern failure
warnings until Plan 006; those warnings are deferred defects, not a valid `BeatGrid`
contract.

Existing style uses plain `#[test]`, direct `assert!` messages with observed values,
and helpers at module scope. Match `synth_kick_track` at
`stratum-dsp/tests/integration_tests.rs:11-40` and
`external_beat_grid_replaces_hmm_grid` at lines 292-371.

## Commands you will need

| Purpose                      | Command                                                             | Expected on success           |
| ---------------------------- | ------------------------------------------------------------------- | ----------------------------- |
| Baseline                     | `cargo test -p stratum-dsp --test integration_tests -- --nocapture` | exit 0; 8 existing tests pass |
| Focused integration          | `cargo test -p stratum-dsp --test integration_tests`                | exit 0; all tests pass        |
| Crate tests                  | `cargo test -p stratum-dsp --no-fail-fast`                          | exit 0; no failures           |
| Format                       | `cargo fmt --check`                                                 | exit 0, no diff               |
| Lint                         | `cargo clippy --workspace --all-targets -- -D warnings`             | exit 0, no warnings           |
| Repository docs/config check | `dprint check`                                                      | exit 0                        |

## Scope

**In scope** (the only source file you should modify):

- `stratum-dsp/tests/integration_tests.rs`
- `plans/README.md` only for the status-row update after completion

**Out of scope** (do NOT touch):

- All files under `stratum-dsp/src/` — this plan establishes tests, not fixes.
- `src/audio.rs` and `STRATUM_SCHEMA_VERSION` — tests do not change output semantics.
- Private/local audio fixtures or paths.
- Exact key-mode, meter, downbeat-phase, or variable-tempo expectations; those belong
  to Plans 004, 005, and 006 after their corresponding algorithms are corrected.
- Test timing/performance thresholds; wall-clock assertions are noisy in CI.

## Git workflow

- Branch: `codex/001-strengthen-stratum-success-oracles`
- Use Conventional Commits. Suggested commit:
  `test(stratum): make synthetic success paths mandatory`
- Do not push or open a PR unless the operator instructs it.
- Do not combine this plan with an algorithm change.

## Steps

### Step 1: Establish the live baseline

Run the focused integration suite before editing. Confirm both kick fixtures produce a
positive BPM and a non-empty beat grid. Also confirm the tonal fixture is not currently
a reliable key oracle; at the planned commit it prints the low-confidence diagnostic.

**Verify**:
`cargo test -p stratum-dsp --test integration_tests -- --nocapture` → exit 0;
the 120 and 128 BPM diagnostics contain positive BPM values and positive beat counts.

If either tempo fixture already returns zero BPM or an empty grid, STOP: this plan is
for strengthening a green baseline, not repairing an algorithm.

### Step 2: Add shared analysis-result invariant helpers

In `stratum-dsp/tests/integration_tests.rs`, add small assertion helpers near the
synthetic fixture functions. They must accept the public `AnalysisResult` (or the
specific slices/scalars needed) and produce useful failure messages. Cover:

1. Every reported scalar used in these tests is finite: BPM, BPM confidence, grid
   stability, duration, and processing time.
2. BPM confidence and grid stability are within `[0.0, 1.0]`.
3. `beat_grid.beats`, `.downbeats`, and `.bars` contain only finite, non-negative
   values. Beats are non-decreasing at this baseline; downbeats and bars are strictly
   ascending. Implement the ordering helpers so Plan 006 can tighten beats to strict
   ordering without duplicating the other checks.
4. Every downbeat and bar lies within the first/last beat range when beats exist.
5. `downbeats == bars`, matching the current `BeatGrid` contract.

Use `windows(2)` for ordering. Do not assert that any duplicate beat exists and do not
silently discard duplicates in this test-only plan. Do not use exact float equality
except for the existing `downbeats == bars` contract, and do not assert a particular
meter here.

Call the helper from both kick-track tests and from the existing external-grid tests
where applicable. In each external-grid test, additionally require supplied and
returned beats/bars to be strictly ascending (a strict wrapper/helper is fine). Keep
assertion messages specific enough to identify the field and bad index/value.

**Verify**:
`cargo test -p stratum-dsp --test integration_tests -- --nocapture` → exit 0; all
existing tests pass with the new invariant helper active.

### Step 3: Remove conditional success from the fixed-tempo tests

Rewrite `test_analyze_120bpm_kick` and `test_analyze_128bpm_kick` so the following are
unconditional after `analyze_audio(...).expect(...)`:

- BPM is positive and within the existing tolerance of the requested BPM.
- BPM confidence is finite and strictly positive.
- The beat grid is non-empty and has at least four beats.
- At least two beats exist before checking the first interval; assert this rather than
  guarding it.
- The first interval is within the existing 0.1-second tolerance.

Keep downbeat/bar checks structural only. Do not preserve the `else { println!(...) }`
branches: a missing result must fail the test, not log and pass. You may retain a
single diagnostic `println!` after all assertions.

**Verify**:
`cargo test -p stratum-dsp --test integration_tests test_analyze_ -- --nocapture` →
exit 0; the two kick tests pass and the silent-input test still passes.

### Step 4: Make the tonal test honest without encoding the known defect

Rename `test_analyze_cmajor_scale` to accurately describe its input (it synthesizes a
C-major chord, not a scale) and its present purpose as a pipeline smoke test. Keep
basic duration and result-invariant checks, but remove comments claiming that key
detection is guaranteed when the test still permits zero confidence. Add a short
comment pointing to Plan 004's detector-level major/minor regression coverage.

Do not assert `key_confidence == 0.0`; that would freeze the bug. Do not add `#[ignore]`
or an intentionally failing test. Plan 004 will add deterministic mode assertions at
the detector layer, where the fixture is not confounded by STFT/chroma extraction.

**Verify**:
`cargo test -p stratum-dsp --test integration_tests` → exit 0; no test name contains
`cmajor_scale` and the renamed smoke test passes.

### Step 5: Run the complete verification gate

Run formatting, linting, all Stratum tests, and the repository's dprint check. Inspect
the diff and ensure it is test-only.

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p stratum-dsp --all-targets -- -D warnings
cargo test -p stratum-dsp --no-fail-fast
git diff --check
git diff --name-only
```

Expected: every command exits 0; `git diff --name-only` lists only
`stratum-dsp/tests/integration_tests.rs` plus `plans/README.md` if the executor owns
the status update.

## Test plan

- Strengthen the two existing fixed-tempo tests; do not add redundant copies.
- Exercise the shared invariants against both generated HMM grids and supplied
  external grids.
- Keep the all-silence error test as the negative path.
- Keep all generated samples deterministic and in memory.
- Use `external_beat_grid_replaces_hmm_grid` as the pattern for detailed public-output
  assertions.
- Do not add tests whose expected result varies by hardware, thread scheduling, or
  processing duration.

## Done criteria

All must hold:

- [ ] `cargo fmt --check` exits 0.
- [ ] `dprint check` exits 0.
- [ ] `cargo clippy -p stratum-dsp --all-targets -- -D warnings` exits 0.
- [ ] `cargo test -p stratum-dsp --no-fail-fast` exits 0.
- [ ] `rg -n 'if result\.bpm > 0\.0|if !result\.beat_grid\.beats\.is_empty\(\)' stratum-dsp/tests/integration_tests.rs` returns no matches.
- [ ] The 120/128 BPM tests unconditionally assert positive BPM, positive confidence,
      non-empty beats, and the existing BPM/interval tolerances.
- [ ] A reusable helper asserts finite/ranged output, non-decreasing generated beats,
      and strictly ascending downbeats/bars; it does not assert that a duplicate exists.
- [ ] Both external-grid tests explicitly retain strict beat/bar ordering; the generated
      bootstrap allowance does not relax supplied-grid validation semantics.
- [ ] No test depends on a filesystem audio fixture or environment variable.
- [ ] `git diff --check` exits 0.
- [ ] `git diff --name-only` contains no production source file and no file outside
      `stratum-dsp/tests/integration_tests.rs` plus the optional index status update.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- The drift check shows the integration tests changed and the conditional excerpts no
  longer match.
- Either fixed-tempo test produces zero BPM, non-positive confidence, or an empty grid
  before edits.
- Making the strengthened BPM/grid assertions pass requires changing tolerances or DSP
  source code.
- The public result no longer uses `downbeats == bars` as its contract.
- A deterministic test still fails twice after allowing only the already-known equal
  consecutive generated beats, or any beat timestamp decreases rather than remaining
  non-decreasing.
- Any required fix would touch a file outside the scope list.

## Maintenance notes

- Treat these helpers as the common end-to-end invariant layer. Plans 004–006 should
  add focused semantic assertions rather than duplicating finite/order checks.
- Plan 006 must change the shared beat ordering check from non-decreasing to strictly
  ascending after it repairs cumulative timing and overlapping-segment output. Do not
  leave the relaxed bootstrap invariant in place once Plan 006 lands.
- Generated-grid duplicates can currently make optional dub-stab/kick-pattern analysis
  fail closed. That is deferred to Plan 006; do not treat the warning as success proof
  and do not repair or suppress it in this test-only plan.
- Review future tests for conditional assertions around the behavior they claim to
  test; optional diagnostics must not replace an oracle.
- Do not add narrow output snapshots for DSP floats. Prefer bounded semantic
  assertions so intentional tuning remains possible.
- If valid algorithm work changes cached Stratum output semantics, consider and
  normally increment `src/audio.rs::STRATUM_SCHEMA_VERSION`; this test-only plan does
  not require a bump.
