# Plan 008: Reject unsafe Stratum configurations before allocating or dividing

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/src/config.rs stratum-dsp/src/lib.rs stratum-dsp/src/analysis/result.rs stratum-dsp/src/preprocessing/silence.rs stratum-dsp/src/features/chroma/extractor.rs stratum-dsp/src/features/onset/hpss.rs stratum-dsp/src/features/dub_stab.rs stratum-dsp/src/features/kick_pattern.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding. Changes
> made by DONE Plan 001 are expected where test support overlaps; reconcile
> dependency changes and continue when intent matches. Unrelated drift is a
> STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/001-strengthen-stratum-success-oracles.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

`AnalysisConfig` is a public struct whose fields can be set to zero, non-finite values,
inverted ranges, or enormous allocation sizes. `analyze_audio` validates only empty
samples and a zero sample rate before copying input and entering helpers. Several
public helpers then divide by `hop_size`, use `frame_size - 1`, cast invalid frequency
ranges to allocation sizes, or reserve windows derived from unchecked margins.

This plan establishes one high-level validation boundary, hardens independently public
helpers, centralizes beat-grid validation, and returns `AnalysisError::InvalidInput`
instead of panics or attempted pathological allocations. Valid default behavior and
serialized output remain unchanged, so this plan must not bump the Stratum schema
version unless implementation unexpectedly changes valid outputs.

## Current state

- `stratum-dsp/src/config.rs` — public `AnalysisConfig` with many documented ranges but
  no validation method.
- `stratum-dsp/src/lib.rs` — public `analyze_audio` entry point.
- `stratum-dsp/src/preprocessing/silence.rs` — demonstrates local validation style but
  permits `frame_size == 1`, which derives a zero hop.
- `stratum-dsp/src/features/chroma/extractor.rs` — public STFT/log-frequency helpers
  with unchecked arithmetic/allocation boundaries.
- `stratum-dsp/src/features/onset/hpss.rs` — public HPSS helpers whose private median
  windows evaluate `index + margin + 1` without checked arithmetic.
- `stratum-dsp/src/analysis/result.rs` — public `BeatGrid` data type.
- `stratum-dsp/src/features/dub_stab.rs` and `kick_pattern.rs` — duplicate private grid
  validators to consolidate.

The top-level boundary currently checks only two inputs
(`stratum-dsp/src/lib.rs:89-116`):

```rust
pub fn analyze_audio(
    samples: &[f32],
    sample_rate: u32,
    config: AnalysisConfig,
) -> Result<AnalysisResult, AnalysisError> {
    // ...
    if samples.is_empty() {
        return Err(AnalysisError::InvalidInput(
            "Empty audio samples".to_string(),
        ));
    }

    if sample_rate == 0 {
        return Err(AnalysisError::InvalidInput(
            "Invalid sample rate".to_string(),
        ));
    }

    let mut processed_samples = samples.to_vec();
```

`compute_stft` divides by `hop_size`, subtracts one from `frame_size`, and allocates
before validating either (`extractor.rs:301-323`):

```rust
pub fn compute_stft(
    samples: &[f32],
    frame_size: usize,
    hop_size: usize,
) -> Result<Vec<Vec<f32>>, AnalysisError> {
    let n_samples = samples.len();
    if n_samples < frame_size {
        return Ok(vec![]);
    }
    let n_frames = (n_samples - frame_size) / hop_size + 1;
    let mut magnitudes = Vec::with_capacity(n_frames);
    let window: Vec<f32> = (0..frame_size)
        .map(|i| {
            let x = 2.0 * std::f32::consts::PI * i as f32 / (frame_size - 1) as f32;
```

Silence detection accepts one sample per frame, then derives a zero hop and divides by
it (`silence.rs:118-145`):

```rust
if detector.frame_size == 0 {
    return Err(AnalysisError::InvalidInput(
        "Frame size must be > 0".to_string(),
    ));
}
// ...
let hop_size = detector.frame_size / 2;
let num_frames = if samples.len() >= detector.frame_size {
    (samples.len() - detector.frame_size) / hop_size + 1
```

Log-frequency conversion clamps invalid bounds and then casts their difference to
`usize` before proving the range is valid (`extractor.rs:733-750`):

```rust
let freq_resolution = sample_rate as f32 / fft_size as f32;
let nyquist = sample_rate as f32 / 2.0;
let fmin = fmin_hz.max(20.0);
let fmax = fmax_hz.min(nyquist - 1.0);
let semitone_min = 12.0 * (fmin / A4_FREQ).log2() + SEMITONE_OFFSET;
let semitone_max = 12.0 * (fmax / A4_FREQ).log2() + SEMITONE_OFFSET;
let semitone_bin_min = semitone_min.floor() as i32;
let semitone_bin_max = semitone_max.ceil() as i32;
let n_semitone_bins = (semitone_bin_max - semitone_bin_min + 1) as usize;
```

HPSS scratch capacity uses unchecked `2 * margin + 1`
(`extractor.rs:1441-1460`). Beat-grid validation is duplicated almost verbatim in
`dub_stab.rs:296-316` and `kick_pattern.rs:395-415`.

Match the existing style: invalid numeric parameters return
`AnalysisError::InvalidInput` with the field name and received value; recoverable
optional feature failures remain warnings only after validated inputs enter the
pipeline.

## Commands you will need

| Purpose               | Command                                                                                                                                        | Expected on success               |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------- |
| Config tests          | `cargo test -p stratum-dsp config -- --nocapture`                                                                                              | exit 0                            |
| STFT tests            | `cargo test -p stratum-dsp features::chroma::extractor::tests -- --nocapture`                                                                  | exit 0                            |
| HPSS tests            | `cargo test -p stratum-dsp features::onset::hpss::tests -- --nocapture`                                                                        | exit 0; overflow/clamp pass       |
| Grid consumers        | `cargo test -p stratum-dsp features::dub_stab::tests -- --nocapture && cargo test -p stratum-dsp features::kick_pattern::tests -- --nocapture` | exit 0; both filtered suites pass |
| Full Stratum suite    | `cargo test -p stratum-dsp --no-fail-fast`                                                                                                     | exit 0                            |
| Format/lint           | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`                                                                   | exit 0                            |
| Repository formatting | `dprint check`                                                                                                                                 | exit 0                            |

## Scope

**In scope** (the only source files you should modify):

- `stratum-dsp/src/config.rs`
- `stratum-dsp/src/lib.rs`
- `stratum-dsp/src/analysis/result.rs`
- `stratum-dsp/src/preprocessing/silence.rs`
- `stratum-dsp/src/features/chroma/extractor.rs`
- `stratum-dsp/src/features/onset/hpss.rs`
- `stratum-dsp/src/features/dub_stab.rs`
- `stratum-dsp/src/features/kick_pattern.rs`
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- DSP tuning, default values, detector weights, BPM/key/meter selection, or valid-output
  serialization.
- CLI argument schemas; this plan protects the library boundary regardless of caller.
- Catching panics with `catch_unwind` instead of validating their causes.
- Adding arbitrary per-track duration limits or rejecting ordinary long-form DJ audio.
- `src/audio.rs` and `STRATUM_SCHEMA_VERSION`; valid output semantics must not change.
- ML feature implementation or any Rekordbox database access/write path.

## Git workflow

- Branch: `codex/008-validate-stratum-analysis-inputs`
- Use Conventional Commits. Suggested logical commits:
  1. `test(stratum): cover invalid analysis configurations`
  2. `fix(stratum): validate public analysis boundaries`
- Do not push or open a PR unless instructed.
- Do not bump `STRATUM_SCHEMA_VERSION` for newly rejected invalid inputs. If valid
  default output changes, STOP and report before altering cache semantics.

## Steps

### Step 1: Build a validation matrix and failing regression tests

Add `AnalysisConfig` tests that mutate one field from `default()` at a time and call a
new validation method. Cover at minimum:

- `frame_size`: 0 and 1; `hop_size`: 0.
- `key_stft_frame_size`: 0/1 and `key_stft_hop_size`: 0 when override is enabled.
- non-finite values (`NaN`, `+/-Infinity`) for representative thresholds, weights,
  frequencies, powers, BPM bounds, and debug ground truth.
- `min_bpm <= 0`, `max_bpm <= min_bpm`, and `bpm_resolution <= 0`.
- percentages/confidences outside `[0, 1]` where documentation declares that range.
- negative weights/powers and all-zero consensus/fusion weight groups.
- unordered frequency bands and values above Nyquist where an enabled algorithm
  requires a real band.
- zero segment/window hops and enabled empty segment-length lists.
- `key_multi_scale_weights` length differing from `key_multi_scale_lengths` when
  non-empty.
- HPSS/window margin arithmetic overflow (`usize::MAX`). Oversized-but-representable
  margins are a supported clamped-window case and must not be rejected merely because
  a valid input spectrogram is short.
- mel band count and candidate counts that would overflow or exceed the actual FFT bin
  resolution.
- a malformed external grid containing NaN, duplicates, descending values, or bars
  outside the beat range.

Also add direct public-helper regressions for `compute_stft([], 0, 0)`, frame size 1,
hop 0, unsafe allocation dimensions, invalid log-frequency bounds, silence frame size
1, and every non-empty `harmonic_spectrogram_hpss_median_mask` argument listed in Step
3. Tests must assert `Err(AnalysisError::InvalidInput(_))`, not an exact full message
and not merely "did not panic".

**Verify**:
`cargo test -p stratum-dsp config -- --nocapture && cargo test -p stratum-dsp features::chroma::extractor::tests -- --nocapture` → a new test fails against unchanged code without aborting the entire test process.

### Step 2: Add `AnalysisConfig::validate` and call it before allocation

Implement a documented public method such as:

```rust
pub fn validate(&self, sample_rate: u32, sample_count: usize)
    -> Result<(), AnalysisError>
```

Call it in `analyze_audio` after the empty/sample-rate checks but **before**
`samples.to_vec()` or any DSP allocation. Keep validation organized with small private
helpers for finite values, closed ranges, positive values, checked window sizes, and
checked allocation products. Error text must identify the bad field.

Validate only constraints required by enabled paths where appropriate; a disabled
experimental feature's unused window size should not reject an otherwise valid
analysis unless the field violates a universal representation invariant such as
non-finiteness. Do not silently clamp caller configuration.

For allocation safety, calculate expected STFT frames and bins using `checked_sub`,
`checked_div`, `checked_add`, and `checked_mul`. Define
`MAX_SPECTROGRAM_CELLS_PER_INPUT_SAMPLE = 16` and reject an STFT request whose
`n_frames * n_bins` exceeds `sample_count * 16` (all products checked). The default
shared STFT is about 2 cells/input-sample and the default key override about 8, so each
passes while hop-size/pathological-frame amplification is bounded. Document that 16
retained `f32` cells have a 64-byte raw floor per input sample before vector/FFT
overhead. This is an explicit fail-closed memory policy with a two-times safety factor
over the largest default pipeline, not an accuracy claim. Dispatching the plan accepts
that policy. Add characterization tests for every shipped preset/default at 44.1, 48,
and 96 kHz; if any documented valid preset exceeds 16, STOP for an owner-reviewed
budget change rather than silently raising the constant. Apply the limit per computed
spectrogram; do not impose a maximum duration.

**Verify**:
`cargo test -p stratum-dsp config -- --nocapture` → exit 0; default config validates at
44.1/48 kHz and every invalid matrix case returns `InvalidInput`.

### Step 3: Harden independently public frame/spectral helpers

Callers can invoke helpers without `analyze_audio`, so add local guards:

- `compute_stft`: require `frame_size >= 2`, `hop_size > 0`, checked frame/bin/cell
  arithmetic, and the same resource budget before window or FFT planning.
- `detect_and_trim`: require `frame_size >= 2` so its derived half-hop is non-zero;
  retain empty-sample behavior.
- `convert_linear_to_log_frequency_spectrogram`: require finite positive frequency
  bounds with `fmin < fmax <= Nyquist`, consistent non-empty frame width, and checked
  positive semitone-bin count before casting/allocating.
- In `features/chroma/extractor.rs`, harden the public
  `harmonic_spectrogram_hpss_median_mask` boundary, not only its scratch capacities.
  Preserve `Ok(vec![])` for an empty spectrogram. For non-empty input, require
  consistent non-empty frames with finite non-negative magnitudes;
  `sample_rate > 0`; `fft_size >= 2`; `frame_step > 0`; finite
  `0 < fmin_hz < fmax_hz <= Nyquist`; and finite `mask_power >= 1.0`.
  Return field-specific `InvalidInput` instead of the current unchanged-output or
  clamped behavior. Validate raw `2 * time_margin + 1` / `2 * freq_margin + 1` with
  checked arithmetic, then clamp each effective window to its actual frame/bin
  dimension before allocating scratch. Oversized representable margins retain the
  current full-dimension median semantics rather than allocate from the raw margin.
  Add one direct test per argument plus empty/valid controls.
- In `features/onset/hpss.rs`, harden both public `hpss_decompose` and
  `harmonic_proportion`: reject raw margin arithmetic overflow, use checked or
  saturating endpoint arithmetic, and preserve the current clamping of representable
  margins larger than the frame/bin dimensions. Direct tests with `usize::MAX`, exact
  dimension boundaries, oversized margins, ragged frames, and ordinary margins must
  return `InvalidInput` where structurally invalid or preserve existing valid output;
  none may panic or attempt a raw-margin allocation.

Do not duplicate the full config validator inside every helper; enforce only that
helper's direct contract.

**Verify**:
`cargo test -p stratum-dsp features::chroma::extractor::tests -- --nocapture && cargo test -p stratum-dsp preprocessing::silence::tests -- --nocapture && cargo test -p stratum-dsp features::onset::hpss::tests -- --nocapture` → all three commands exit 0; invalid helpers reject only unsafe inputs, and existing/oversized-clamped valid cases pass.

### Step 4: Centralize and strengthen `BeatGrid` validation

Move the duplicate strict-order checks into one method or function colocated with
`BeatGrid` in `analysis/result.rs`. It must validate:

- finite, non-negative, strictly ascending `beats`, `downbeats`, and `bars`;
- `downbeats == bars` under the current contract;
- each bar/downbeat lies within the beat range and corresponds to a beat within the
  named `BEAT_GRID_MATCH_TOLERANCE_SECONDS = 0.001` tolerance. Rekordbox PQTZ input is
  millisecond-quantized (`time_ms / 1000.0`), while generated bars are selected from
  beats, so one millisecond is the public compatibility bound; test just-inside and
  just-outside values rather than requiring bit equality;
- a non-empty bars list cannot accompany an empty beats list.

Use it at the `AnalysisConfig.external_beat_grid` boundary and from both dub-stab and
kick-pattern consumers. Remove their duplicate private validators. Keep behavior
result-bearing; do not sort or mutate malformed caller input.

**Verify**:

```bash
cargo test -p stratum-dsp features::dub_stab::tests -- --nocapture
cargo test -p stratum-dsp features::kick_pattern::tests -- --nocapture
rg -n '^fn validate_beat_grid' stratum-dsp/src/features
```

Expected: both suites exit 0; `rg` returns no duplicate feature-local validators.

### Step 5: Prove invalid inputs fail before heavy work

Add high-level `analyze_audio` tests with a small non-empty sample slice and invalid
configurations. Assert field-specific `InvalidInput` is returned before silence/STFT
processing. Include a pathological-but-non-overflow allocation request and ensure the
test returns promptly without allocating it; do not measure wall time, just select
parameters whose derived cell count exceeds the documented budget.

Add a valid-default control test and a valid external-grid control so validation does
not reject normal behavior. Add a short valid spectrogram/audio case whose default HPSS
margins exceed one dimension and assert the same finite `harmonic_proportion` result as
the pre-validation clamped behavior; it must not be converted to `None` or an error.

**Verify**:
`cargo test -p stratum-dsp --lib analysis_config -- --nocapture` → exit 0; invalid
cases return errors and both controls pass.

### Step 6: Run complete gates and verify output/cache scope

Run the full crate suite and inspect the diff. Confirm `src/audio.rs` and the root cache
schema are untouched.

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

Expected: all commands exit 0; `git diff -- src/audio.rs` is empty; changed files are
limited to the scope list.

## Test plan

- Table-driven config tests for zero, non-finite, inverted, mismatched, overflow, and
  resource-budget cases.
- Direct helper tests because `compute_stft`, log-frequency conversion, silence
  detection, and HPSS helpers are independently callable.
- HPSS tests for raw arithmetic overflow, extractor scratch bounds, oversized-margin
  clamping parity, short valid spectrograms, and ordinary windows.
- Central BeatGrid tests for each malformed list plus valid empty/non-empty grids.
- High-level tests proving rejection occurs before the analysis pipeline.
- Valid default and external-grid regression controls.
- Never construct the huge allocation being rejected; test its derived dimensions.

## Done criteria

All must hold:

- [ ] `analyze_audio` validates config before cloning samples or allocating DSP state.
- [ ] Frame size 0/1 and hop size 0 return `InvalidInput` from both the high-level API
      and directly public helpers.
- [ ] Non-finite and inverted enabled configuration ranges return field-specific
      `InvalidInput`.
- [ ] Checked arithmetic rejects margin/count overflow and over-budget spectrograms
      without allocating them.
- [ ] Extractor HPSS scratch capacity/endpoints use checked arithmetic and
      dimension-clamped effective margins rather than raw user/configured margins.
- [ ] Non-empty `harmonic_spectrogram_hpss_median_mask` rejects zero
      sample-rate/FFT/frame-step, non-finite or unordered bands, invalid mask power,
      and non-finite/negative/ragged magnitudes with field-specific `InvalidInput`;
      its empty and valid controls preserve behavior.
- [ ] Public HPSS entry points reject `usize::MAX` overflow without panic, while
      representable oversized margins and short valid spectrograms retain their
      existing clamped finite output.
- [ ] Beat-grid membership uses the documented exact 1 ms tolerance with boundary
      tests.
- [ ] One central BeatGrid validator is reused by config, dub-stab, and kick-pattern.
- [ ] `rg -n '^fn validate_beat_grid' stratum-dsp/src/features` returns no matches.
- [ ] Default config at 44.1/48 kHz and valid external grids remain accepted.
- [ ] `src/audio.rs` is unchanged and no schema version is bumped.
- [ ] Format, dprint, clippy, and full Stratum tests exit 0.
- [ ] `git diff --check` exits 0; no out-of-scope file changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plan 001 is incomplete or its baseline tests are red.
- Drift has already introduced a central config/grid validator with materially
  different contracts; reconcile rather than duplicate it.
- A proposed bound rejects `AnalysisConfig::default()` for a normal 44.1/48 kHz,
  long-form track.
- Preventing allocation abuse appears to require an arbitrary maximum track duration
  rather than checked derived memory.
- Valid default output changes, which would require a `STRATUM_SCHEMA_VERSION` decision.
- A proposed HPSS guard turns a currently valid short/oversized clamped window into an
  error or missing feature; preserve clamping and reject only unsafe arithmetic.
- A helper's public contract explicitly permits a value this plan proposes to reject
  and there is a known in-repo caller using it.
- A step fails twice or requires an out-of-scope file.

## Maintenance notes

- Add new `AnalysisConfig` fields to `validate` in the same change that introduces the
  field. Document whether validation is universal or gated by its enable flag.
- Public helpers must retain their own minimal guards even when the high-level API
  validates first.
- Resource budgets are API policy: review their practical memory cost and long-form
  defaults when spectrogram representation changes.
- Newly rejecting invalid input does not require cache invalidation. Any change to
  valid analysis values still requires `STRATUM_SCHEMA_VERSION` consideration.
