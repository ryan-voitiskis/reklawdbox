# Plan 017: Run legacy BPM estimation only when policy needs it

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/src/lib.rs stratum-dsp/tests/integration_tests.rs`
> If an in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding. Changes
> made by DONE Plan 005 and its dependencies are expected in `lib.rs` and the
> integration tests; reconcile those committed changes and continue when
> intent matches. Unrelated drift is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: MED
- **Depends on**: `plans/005-infer-meter-and-downbeat-phase.md` (transitively includes Plan 008)
- **Category**: perf
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Default configuration is documented and selected as tempogram-first with legacy BPM
used only when tempogram fails. The implementation nevertheless runs the legacy
autocorrelation/comb-filter estimator before it runs tempogram, then discards the
legacy value on the normal successful path. Every default analysis therefore pays an
avoidable estimator cost, and a legacy error can fail analysis even when tempogram
would have succeeded.

This plan evaluates tempogram first and lazily invokes legacy estimation only for
forced-legacy mode, fusion mode, or a missing tempogram estimate. It leaves onset
consensus in place because those onsets also feed beat tracking, preserves all BPM
selection semantics, and does not change serialized output fields or cache versions.

## Current state

- `stratum-dsp/src/lib.rs` — top-level pipeline and both BPM estimators.
- `stratum-dsp/tests/integration_tests.rs` — deterministic default-path tests from
  Plan 001; use only for output parity, not wall-clock thresholds.

The default flags explicitly disable forced/fused legacy estimation
(`stratum-dsp/src/config.rs:642-643`):

```rust
force_legacy_bpm: false,
enable_bpm_fusion: false,
```

Despite that, legacy estimation runs unconditionally before tempogram
(`stratum-dsp/src/lib.rs:307-343`):

```rust
// BPM estimation: tempogram (Phase 1F) + legacy (Phase 1B), optionally fused
let legacy_estimate = {
    use features::period::{estimate_bpm, estimate_bpm_with_guardrails, LegacyBpmGuardrails};
    if onsets_for_legacy.len() >= 2 {
        if config.enable_legacy_bpm_guardrails {
            let guardrails = LegacyBpmGuardrails {
                preferred_min: config.legacy_bpm_preferred_min,
                preferred_max: config.legacy_bpm_preferred_max,
                soft_min: config.legacy_bpm_soft_min,
                soft_max: config.legacy_bpm_soft_max,
                mul_preferred: config.legacy_bpm_conf_mul_preferred,
                mul_soft: config.legacy_bpm_conf_mul_soft,
                mul_extreme: config.legacy_bpm_conf_mul_extreme,
            };
            estimate_bpm_with_guardrails(/* ... */)?
        } else {
            estimate_bpm(/* ... */)?
        }
    } else {
        None
    }
};
```

Tempogram is computed afterward (`lib.rs:351-353`):

```rust
let tempogram_estimate = if !config.force_legacy_bpm && !magnitude_spec_frames.is_empty() {
    use crate::analysis::result::TempoCandidateDebug;
    use features::period::multi_resolution::multi_resolution_tempogram_from_samples;
```

The final default branch claims the intended lazy policy even though the value is
already eagerly computed (`lib.rs:907-913`):

```rust
// Default behavior: tempogram first; legacy fallback only if tempogram fails.
tempogram_estimate
    .as_ref()
    .map(|e| (e.bpm, e.confidence))
    .or_else(|| legacy_estimate.as_ref().map(|e| (e.bpm, e.confidence)))
    .unwrap_or((0.0, 0.0))
```

The onset list must not be removed or lazily omitted: comments at `lib.rs:182-186`
state that it also supports beat tracking. Match existing `Result<_, AnalysisError>`
propagation and `BpmEstimate` usage.

## Commands you will need

- Policy unit tests: `cargo test -p stratum-dsp legacy_bpm -- --nocapture` — exits 0.
- Integration parity:
  `cargo test -p stratum-dsp --test integration_tests -- --nocapture` — exits 0.
- Stratum suite: `cargo test -p stratum-dsp --no-fail-fast` — exits 0.
- Format/lint:
  `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings` — exits 0.
- Repository formatting: `dprint check` — exits 0.

## Scope

**In scope** (the only source files you should modify):

- `stratum-dsp/src/lib.rs`
- `stratum-dsp/tests/integration_tests.rs` only if a default/forced/fusion parity case
  cannot be expressed in the existing `lib.rs` test module
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- Onset detection/consensus computation; beat tracking still consumes the selected
  onsets.
- Tempogram, legacy estimator, guardrail, or BPM fusion algorithms and thresholds.
- Removing legacy support or changing the public `AnalysisConfig` flags.
- Timing assertions or a new benchmark framework.
- `src/audio.rs` and `STRATUM_SCHEMA_VERSION`; valid selected outputs must be identical.
- Any Rekordbox database code or cache-store migration.

## Git workflow

- Branch: `codex/017-defer-legacy-bpm-fallback`
- Use Conventional Commits. Suggested commit:
  `perf(stratum): defer legacy bpm fallback`
- Do not push or open a PR unless instructed.
- Do not bump `STRATUM_SCHEMA_VERSION`; STOP if the refactor changes selected BPM or
  confidence for an equivalent pair of estimator results.

## Steps

### Step 1: Encode and test the estimator policy matrix

Add a small private pure policy helper in `lib.rs`, named for the decision it makes.
Its truth table must be:

| force legacy | fusion enabled | tempogram available | run legacy |
| ------------ | -------------- | ------------------- | ---------- |
| true         | either         | either              | yes        |
| false        | true           | either              | yes        |
| false        | false          | false               | yes        |
| false        | false          | true                | no         |

Wrap lazy execution in a private generic helper accepting an `FnOnce` estimator closure
and returning `Result<Option<BpmEstimate>, AnalysisError>`. This makes laziness directly
testable without logging or wall-clock measurements.

Unit-test every policy row. Use `Cell`/`AtomicUsize` in tests to assert the estimator
closure is called exactly once when required and zero times on the default successful
tempogram path. Also assert an error-producing closure is not evaluated when policy
says skip.

**Verify**:
`cargo test -p stratum-dsp legacy_bpm_policy -- --nocapture` → exit 0; the skip case
proves zero closure calls.

### Step 2: Extract the existing legacy calculation without changing it

Move the current guardrail/non-guardrail calculation into a private function that
accepts exactly the values it needs (prefer `&AnalysisConfig`, onset indices, sample
rate) and returns the same `Result<Option<BpmEstimate>, AnalysisError>`. Preserve:

- the `onsets_for_legacy.len() >= 2` guard;
- `LegacyBpmGuardrails` field mapping;
- `estimate_bpm_with_guardrails` versus `estimate_bpm` selection;
- existing error propagation when the estimator is actually required.

Do not change estimator parameters or clamp values differently. Plan 008 has already
validated configuration at the public boundary.

Add focused tests comparing this helper with direct estimator calls for one synthetic
onset sequence in guardrail-on and guardrail-off modes.

**Verify**:
`cargo test -p stratum-dsp legacy_bpm -- --nocapture` → exit 0; helper/direct results
match for both guardrail modes.

### Step 3: Move legacy invocation after tempogram and make it lazy

Delete the eager `let legacy_estimate = { ... }` block before tempogram. After
`tempogram_estimate` has been fully computed, call the policy/lazy helper:

```text
run_legacy = force_legacy
          || fusion_enabled
          || tempogram_estimate.is_none()
legacy_estimate = maybe_run(run_legacy, || compute_legacy(...))?
```

Then leave the existing forced/fusion/default selection logic semantically unchanged.
Ensure each required path evaluates legacy no more than once. Add a debug log when the
default successful tempogram path skips legacy so profiling remains observable; do not
log per-frame or at info level.

**Verify**:

```bash
cargo test -p stratum-dsp legacy_bpm -- --nocapture
rg -n 'let legacy_estimate' stratum-dsp/src/lib.rs
```

Expected: tests exit 0; exactly one production `legacy_estimate` binding exists and it
appears after the `tempogram_estimate` block.

### Step 4: Add selection/output parity coverage

Test all behavior modes without asserting runtime:

1. Default + available tempogram: selected BPM/confidence comes only from tempogram and
   the legacy closure is not called.
2. Default + no tempogram: legacy is called once and selected.
3. Forced legacy: tempogram remains skipped and legacy is called once.
4. Fusion: both estimates exist, legacy is called once, tempogram BPM remains selected,
   and the existing agreement/disagreement confidence rule is unchanged.
5. Required legacy error: error still propagates.
6. Skipped legacy error closure: analysis/selection succeeds from tempogram.

Prefer unit tests around private policy/selection helpers using constructed
`BpmEstimate` values. Keep Plan 001's 120/128 BPM integration tests as the default-path
public control. Do not add flaky processing-time expectations.

**Verify**:

```bash
cargo test -p stratum-dsp legacy_bpm -- --nocapture
cargo test -p stratum-dsp --test integration_tests -- --nocapture
```

Expected: both commands exit 0; fixed-tempo BPM tolerances and confidence assertions
remain unchanged.

### Step 5: Run full gates and verify no cache-semantic change

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p stratum-dsp --no-fail-fast
git diff --check
git diff --name-only
git diff -- src/audio.rs
```

Expected: all commands exit 0; `src/audio.rs` diff is empty; only in-scope files changed.

## Test plan

- Exhaustive pure policy truth table.
- Closure call-count tests proving actual laziness, including a skipped error closure.
- Legacy helper parity for guardrails on/off.
- Selection parity for default fallback, forced, fusion agreement/disagreement, and
  required error propagation.
- Existing synthetic fixed-tempo integration tests as public default controls.
- No wall-clock threshold or private audio fixture.

## Done criteria

All must hold:

- [ ] Default config with a tempogram estimate invokes the legacy closure zero times.
- [ ] Forced, fusion, and missing-tempogram cases invoke legacy exactly once.
- [ ] A skipped legacy error cannot fail a successful tempogram analysis.
- [ ] A required legacy error still returns `AnalysisError`.
- [ ] Fusion continues to select tempogram BPM and applies its existing confidence rule.
- [ ] Onset consensus and beat-tracking onset inputs remain present.
- [ ] `STRATUM_SCHEMA_VERSION` and `src/audio.rs` are unchanged.
- [ ] Format, dprint, clippy, integration tests, and the full Stratum suite exit 0.
- [ ] `git diff --check` exits 0 and no out-of-scope files changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plan 005 or its dependencies are incomplete, or the validated default pipeline is red.
- The current pipeline already lazily invokes legacy, so the audited excerpt no longer
  represents live behavior.
- Moving the call changes forced/fusion/default selected BPM or confidence for the same
  pair of estimator results.
- Avoiding eager legacy work would also require removing onset consensus or changing
  beat tracking.
- The only way to prove laziness appears to be a flaky timing threshold; use the
  injected closure/call-count design instead.
- Valid output semantics change and would require cache schema invalidation.
- A step fails twice or requires an out-of-scope file.

## Maintenance notes

- Keep the policy truth table next to the helper. Any future estimator mode must update
  both the table tests and lazy-decision function.
- Do not infer work scheduling from final `Option` chaining: expensive fallback values
  must themselves be produced lazily.
- Onset consensus remains intentionally eager for beat tracking; profile it separately
  before proposing another deferral.
- This refactor changes performance/error avoidance, not cached valid output. Revisit
  `STRATUM_SCHEMA_VERSION` only if a future change alters selected analysis values.
