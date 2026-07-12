# Plan 004: Preserve major-versus-minor evidence in key detection

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/src/features/key/detector.rs stratum-dsp/src/features/key/templates.rs stratum-dsp/tests/integration_tests.rs src/audio.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition. Changes made by a DONE dependency
> plan are expected: reconcile its committed result with this plan and continue
> when the intent still matches. Unrelated drift is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: HIGH
- **Depends on**: `plans/001-strengthen-stratum-success-oracles.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The key detector L2-normalizes every key template, computes comparable scores across
all 24 keys, and then divides major scores by the best major score and minor scores by
the best minor score. That guarantees both mode groups have a score of `1.0`, erasing
the evidence that should distinguish major from minor and often reducing confidence to
zero. The subsequent "weighted voting" cannot aggregate votes because each of its
three keys is unique, and hash-map tie ordering can make a close result nondeterministic.

This plan restores one comparable 24-key score space, makes selection deterministic,
and pins the behavior with detector-level major/minor tests. Because key and confidence
values in cached `StratumResult` records may change, the Stratum cache schema version
must be incremented.

## Current state

- `stratum-dsp/src/features/key/templates.rs` — creates and L2-normalizes the
  Krumhansl-Kessler and Temperley templates.
- `stratum-dsp/src/features/key/detector.rs` — scores, refines, ranks, and reports keys.
- `stratum-dsp/tests/integration_tests.rs` — portable public-pipeline smoke tests from
  Plan 001; only add an end-to-end assertion if it is deterministic after the
  detector-level correction.
- `src/audio.rs` — root-crate cache schema constant for serialized Stratum output.

Templates are already normalized to the same magnitude
(`stratum-dsp/src/features/key/templates.rs:126-139`):

```rust
// L2-normalize each template so dot-products behave like cosine similarity against L2-normalized chroma.
fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|&x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

for k in 0..12 {
    l2_normalize(&mut major[k]);
    l2_normalize(&mut minor[k]);
}
```

The detector then destroys cross-mode scale information
(`stratum-dsp/src/features/key/detector.rs:135-167`):

```rust
// Step 1.5: Normalize major and minor scores separately to address scale differences.
let max_major = scores
    .iter()
    .filter_map(|(k, s)| {
        if matches!(k, Key::Major(_)) { Some(*s) } else { None }
    })
    .fold(0.0f32, f32::max);
let max_minor = scores
    .iter()
    .filter_map(|(k, s)| {
        if matches!(k, Key::Minor(_)) { Some(*s) } else { None }
    })
    .fold(0.0f32, f32::max);

if max_major > 1e-9 && max_minor > 1e-9 {
    for (k, s) in scores.iter_mut() {
        match k {
            Key::Major(_) => *s /= max_major,
            Key::Minor(_) => *s /= max_minor,
        }
    }
}
```

The top-three voting step stores each distinct key once and therefore cannot combine
support (`stratum-dsp/src/features/key/detector.rs:248-275`):

```rust
let final_key = if use_weighted_voting {
    let mut key_votes: std::collections::HashMap<Key, f32> = std::collections::HashMap::new();
    for (key, score) in scores.iter().take(3) {
        let vote_weight = *score / best_score;
        *key_votes.entry(*key).or_insert(0.0) += vote_weight;
    }
    key_votes
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(k, _)| *k)
        .unwrap_or(best_key)
} else {
    best_key
};
```

Current detector coverage only checks a C-major-shaped vector and does not assert
positive confidence or minor-mode behavior
(`stratum-dsp/src/features/key/detector.rs:1015-1047`).

Root cache versioning is explicit (`src/audio.rs:129-132`):

```rust
/// Expected analysis schema versions. Bump these when adding/changing output
/// fields so that stale cache entries are evicted automatically.
pub const STRATUM_SCHEMA_VERSION: &str = "18";
pub const ESSENTIA_SCHEMA_VERSION: &str = "2";
```

Match existing error handling: invalid detector input returns
`AnalysisError::InvalidInput`; numerical ambiguity returns a low confidence rather
than panicking. Keep the public `KeyDetectionResult` shape unchanged.

## Commands you will need

| Purpose               | Command                                                                              | Expected on success  |
| --------------------- | ------------------------------------------------------------------------------------ | -------------------- |
| Detector tests        | `cargo test -p stratum-dsp features::key::detector::tests -- --nocapture`            | exit 0 after the fix |
| Integration tests     | `cargo test -p stratum-dsp --test integration_tests -- --nocapture`                  | exit 0               |
| Stratum suite         | `cargo test -p stratum-dsp --no-fail-fast`                                           | exit 0               |
| Root cache tests      | `cargo test -p reklawdbox audio::tests::stratum_result_shape_matches_schema_version` | exit 0               |
| Format                | `cargo fmt --check`                                                                  | exit 0               |
| Lint                  | `cargo clippy --workspace --all-targets -- -D warnings`                              | exit 0, no warnings  |
| Repository formatting | `dprint check`                                                                       | exit 0               |

## Scope

**In scope** (the only source files you should modify):

- `stratum-dsp/src/features/key/detector.rs`
- `stratum-dsp/src/features/key/templates.rs` only if a test-only accessor/helper is
  genuinely necessary; prefer its existing public getters
- `stratum-dsp/tests/integration_tests.rs` only for a deterministic public-pipeline
  regression that builds on Plan 001
- `src/audio.rs` only to increment `STRATUM_SCHEMA_VERSION` and update its exact-value
  test
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- Template profile values or the Krumhansl-Kessler/Temperley source data.
- Chroma/HPCP extraction, frame weighting, key-segment voting, ensemble weights,
  tuning compensation, or mode heuristics.
- New configuration flags to preserve the broken normalization.
- Rekordbox database or cache storage code beyond the schema-version constant.
- Any direct write to Rekordbox `master.db`; it must remain read-only.
- Accuracy claims based on private music files.

## Git workflow

- Branch: `codex/004-preserve-key-mode-evidence`
- Use Conventional Commits. Suggested logical commits:
  1. `test(stratum): cover major and minor key evidence`
  2. `fix(stratum): preserve cross-mode key scores`
- Do not push or open a PR unless the operator instructs it.
- Increment the numeric `STRATUM_SCHEMA_VERSION` by one from its **live** value. Do
  not assume it is still `18` after dependency plans land.

## Steps

### Step 1: Add deterministic detector-level regression tests

In the existing `#[cfg(test)]` module in `detector.rs`, construct chroma frames directly
from `KeyTemplates` getters so STFT, tuning, and frame-selection behavior cannot
confound the scoring test. Add tests covering both `TemplateSet::KrumhanslKessler` and
`TemplateSet::Temperley`:

1. A repeated C-major template selects `Key::Major(0)`.
2. A repeated C-minor template selects `Key::Minor(0)`.
3. At least one transposed pair (for example D major and A minor) selects the matching
   tonic and mode.
4. For every case, the chosen score is greater than the parallel-mode score, the
   confidence is finite and strictly positive, all 24 scores are finite, and
   `top_keys[0]` equals `key`.
5. Repeat one case with positive frame weights to cover `detect_key_weighted`.

Use several cloned frames from the selected template; do not perturb or tune values to
favor the implementation. Before the fix, at least the positive cross-mode margin or
confidence assertion must fail, reproducing the defect.

**Verify**:
`cargo test -p stratum-dsp features::key::detector::tests -- --nocapture` → before the
fix, exits non-zero in a newly added cross-mode assertion; existing tests remain green.
If every new regression already passes against unchanged live code, STOP and report
that the premise has drifted.

### Step 2: Keep all 24 raw template scores comparable

Remove the separate major/minor max normalization from `detect_key_weighted`. Do not
replace it with another per-mode transformation. The template constructors already
L2-normalize every profile, so the weighted dot products are in one comparable score
space.

Retain the existing circle-of-fifths refinement only if its bonus scales each candidate
from scores in that same global space. Add a concise comment stating the invariant:
major and minor candidates must remain directly comparable through final ranking.

Guard all sorting/comparison logic against non-finite input. The detector already
validates dimensions; add rejection of non-finite chroma values and non-finite/negative
frame weights if absent, returning `AnalysisError::InvalidInput` rather than allowing a
`partial_cmp(...).unwrap()` panic.

**Verify**:
`cargo test -p stratum-dsp features::key::detector::tests -- --nocapture` → all detector
tests pass; both major and minor template fixtures have positive margins.

### Step 3: Make final-key selection deterministic

Delete the top-three `HashMap` voting branch. It cannot aggregate distinct candidates.
After sorting, select `scores[0]` as the final key. Compute confidence from the margin
between that selected score and the best _other_ key, clamped to `[0.0, 1.0]` as today.

Keep `top_keys` as the first three ranked candidates and ensure `top_keys[0].0 == key`.
Update the debug message so it no longer claims weighted voting occurred. Do not add a
new tie-break based on hash iteration. For exact score ties, use a documented stable
ordering (score descending, then the existing major-then-minor/tonic construction
order) and assert it in a zero-information/uniform-chroma unit test with confidence
`0.0`.

**Verify**:

```bash
cargo test -p stratum-dsp features::key::detector::tests -- --nocapture
rg -n 'max_major|max_minor|use_weighted_voting|key_votes' stratum-dsp/src/features/key/detector.rs
```

Expected: tests exit 0; `rg` returns no matches.

### Step 4: Exercise the public analysis path without weakening the oracle

Run the integration suite created by Plan 001. If its deterministic tonal chord now
produces the expected major key with positive confidence on repeated local runs, make
that assertion unconditional and add a corresponding minor-chord fixture with the same
harmonic construction and amplitude envelope. Run the focused tests at least five
times.

If the end-to-end key remains confounded by chroma extraction or is not deterministic,
do **not** broaden this plan or loosen tolerances. Keep the reliable detector-level
regressions and leave the integration smoke test honest as Plan 001 specified.

**Verify**:
`for i in 1 2 3 4 5; do cargo test -p stratum-dsp --test integration_tests -- test_analyze_ || exit 1; done` → exit 0 on all five runs. Any new key assertion must run on every iteration, not be behind an `if`.

### Step 5: Invalidate cached Stratum results

The corrected detector can change `key`, `key_camelot`, and `key_confidence` in
serialized output. Increment `src/audio.rs::STRATUM_SCHEMA_VERSION` by one from the
live numeric value and update `stratum_result_shape_matches_schema_version` to expect
that same value. Do not change `ESSENTIA_SCHEMA_VERSION`.

This is output-semantic invalidation, not a store migration. Existing rows remain in
the local cache but are no longer considered fresh.

**Verify**:
`cargo test -p reklawdbox audio::tests::stratum_result_shape_matches_schema_version` →
exit 0; the test and constant contain the same incremented value.

### Step 6: Run full gates and inspect scope

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

Expected: every command exits 0; changed files are limited to the scope list.

## Test plan

- Detector unit tests must cover representative major and minor profiles for both
  template families, a transposition, weighted frames, uniform ambiguity, and invalid
  non-finite input.
- Assert relationships (correct key, global score ordering, positive/zero confidence),
  not brittle exact floating-point scores.
- Reuse `test_detect_key_basic` in `detector.rs` as the structural test pattern.
- Use Plan 001's synthetic integration helpers only if the complete pipeline is stable;
  detector-level tests are the required oracle.
- Run root tests because schema invalidation is owned by `src/audio.rs`.

## Done criteria

All must hold:

- [ ] Major and minor template fixtures select the correct mode with a finite,
      positive confidence under both template sets.
- [ ] A uniform/ambiguous fixture has deterministic ordering and zero confidence.
- [ ] Non-finite chroma or weights return `AnalysisError::InvalidInput` without panic.
- [ ] `rg -n 'max_major|max_minor|use_weighted_voting|key_votes' stratum-dsp/src/features/key/detector.rs` returns no matches.
- [ ] `top_keys[0].0 == key` is asserted by tests.
- [ ] `STRATUM_SCHEMA_VERSION` is exactly one greater than its value at the start of
      this plan, and its exact-value test matches.
- [ ] `ESSENTIA_SCHEMA_VERSION` is unchanged.
- [ ] `cargo fmt --check`, `dprint check`, and workspace clippy all exit 0.
- [ ] Both crate test commands exit 0.
- [ ] Release build, `--version`, and `--help` smoke commands exit 0.
- [ ] `git diff --check` exits 0 and no out-of-scope file is changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plan 001 is not complete or its strengthened Stratum baseline is red.
- The template constructors no longer L2-normalize both major and minor templates.
- The new cross-mode regression passes unchanged code, so the audited premise cannot
  be reproduced.
- Correct mode selection appears to require changing profile values, chroma extraction,
  mode heuristics, or private-fixture tuning.
- A representative exact template cannot beat its parallel mode after removing the
  separate normalization; report the scores and template set.
- The fix changes the public `KeyDetectionResult` or `StratumResult` shape.
- A step fails twice after a reasonable correction or requires an out-of-scope file.

## Maintenance notes

- Any future score calibration must preserve direct comparability across all 24 keys.
  Review per-mode normalization or bonuses with particular suspicion.
- Template values are already L2-normalized; changing that invariant requires revisiting
  the score and confidence model together.
- Keep deterministic detector fixtures separate from full audio-pipeline accuracy
  evaluation. Real-corpus tuning is a distinct, data-dependent project.
- This plan intentionally bumps `STRATUM_SCHEMA_VERSION`. Later output-semantic plans
  must inspect the live value and increment it again rather than restoring a hardcoded
  number.
