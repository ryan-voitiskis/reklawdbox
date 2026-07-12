# Plan 005: Infer meter from accents and preserve downbeat phase

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- stratum-dsp/src/lib.rs stratum-dsp/src/features/beat_tracking/mod.rs stratum-dsp/src/features/beat_tracking/time_signature.rs stratum-dsp/tests/integration_tests.rs src/audio.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition. Changes made by DONE Plans 001, 004,
> and 008 are expected in shared files: reconcile their committed result with
> this plan and continue when the intent still matches. Unrelated drift is a
> STOP.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: `plans/004-preserve-key-mode-evidence.md`, `plans/008-validate-stratum-analysis-inputs.md` (both transitively depend on Plan 001)
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The current meter detector receives beat timestamps only. A regular beat grid has the
same interval at every lag, so 3/4, 4/4, and 6/8 all score equally; the explicit tie
rule then chooses the shortest period, 3/4. Downbeat generation compounds the error by
assuming the first tracked beat is beat one and stepping forward by a fixed bar time.
For ordinary regular 4/4 material this produces a bar every three beats and can shift
grid-dependent section, dub-stab, and kick-pattern features.

This plan introduces explicit onset-accent evidence, jointly selects meter and phase,
and falls back conservatively to low-confidence 4/4 when the audio contains no
discriminating accents. It preserves the existing public `generate_beat_grid` entry
point as a compatibility wrapper and adds a richer crate-internal API for callers that
have accent evidence.

## Current state

- `stratum-dsp/src/lib.rs` — owns decoded samples, onset sample indices, STFT output,
  and the call into beat-grid generation.
- `stratum-dsp/src/features/beat_tracking/mod.rs` — tracks beats, invokes meter
  detection, and turns the selected meter into bars/downbeats.
- `stratum-dsp/src/features/beat_tracking/time_signature.rs` — interval-only meter
  scorer.
- `stratum-dsp/tests/integration_tests.rs` — deterministic synthetic public-pipeline
  tests established by Plan 001.
- `src/audio.rs` — Stratum output cache version.

The current detector calculates only beat intervals and tests autocorrelation at each
candidate period (`time_signature.rs:107-137`):

```rust
let mut intervals = Vec::new();
for i in 1..beats.len() {
    let interval = beats[i] - beats[i - 1];
    if interval > 0.0 {
        intervals.push(interval);
    }
}

let mean_interval: f32 = intervals.iter().sum::<f32>() / intervals.len() as f32;
let score_44 = score_time_signature(&intervals, 4, mean_interval);
scores.push((TimeSignature::FourFour, score_44));
let score_34 = score_time_signature(&intervals, 3, mean_interval);
scores.push((TimeSignature::ThreeFour, score_34));
let score_68 = score_time_signature(&intervals, 6, mean_interval);
scores.push((TimeSignature::SixEight, score_68));
```

Equal scores deliberately prefer the shortest period
(`time_signature.rs:139-147`):

```rust
// Find best match; on tied scores prefer shorter period (3/4 over 6/8)
// since a period-N signal always also matches period-2N.
let (best_sig, best_score) = scores
    .iter()
    .max_by(|(sig_a, a), (sig_b, b)| {
        a.partial_cmp(b)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| sig_b.beats_per_bar().cmp(&sig_a.beats_per_bar()))
    })
    .unwrap();
```

The selected meter is applied without phase evidence (`beat_tracking/mod.rs:221-233`):

```rust
let beat_times: Vec<f32> = beat_positions.iter().map(|bp| bp.time_seconds).collect();
let (time_sig, time_sig_confidence) = detect_time_signature(&beat_times, bpm_estimate)?;

let beat_grid =
    generate_beat_grid_from_positions_with_time_sig(&beat_positions, bpm_estimate, time_sig)?;
```

Downbeat generation always anchors the first beat and assumes a fixed bar interval
(`beat_tracking/mod.rs:379-400`):

```rust
let beats_per_bar = time_sig.beats_per_bar() as f32;
let beat_interval = 60.0 / bpm_estimate;
let bar_interval = beat_interval * beats_per_bar;
let tolerance = bar_interval * 0.1;

let mut downbeats = Vec::new();
downbeats.push(beats[0]);
for &beat_time in &beats[1..] {
    let last_downbeat = downbeats[downbeats.len() - 1];
    let expected_next_downbeat = last_downbeat + bar_interval;
    if (beat_time - expected_next_downbeat).abs() <= tolerance {
        downbeats.push(beat_time);
    }
}
```

At the top-level pipeline, onset positions are still available as sample indices before
beat tracking (`stratum-dsp/src/lib.rs:187-188`):

```rust
let mut onsets_for_legacy: Vec<usize> = energy_onsets.clone();
let mut onsets_for_beat_tracking: Vec<usize> = energy_onsets.clone();
```

Match repository conventions: public numeric failures return
`AnalysisError::InvalidInput`; tests synthesize samples in memory; external Rekordbox
grids bypass HMM generation and must remain unchanged.

## Commands you will need

| Purpose               | Command                                                                                   | Expected on success |
| --------------------- | ----------------------------------------------------------------------------------------- | ------------------- |
| Meter unit tests      | `cargo test -p stratum-dsp features::beat_tracking::time_signature::tests -- --nocapture` | exit 0 after fix    |
| Beat-grid unit tests  | `cargo test -p stratum-dsp features::beat_tracking::tests -- --nocapture`                 | exit 0              |
| Integration tests     | `cargo test -p stratum-dsp --test integration_tests -- --nocapture`                       | exit 0              |
| Stratum suite         | `cargo test -p stratum-dsp --no-fail-fast`                                                | exit 0              |
| Root suite            | `cargo test -p reklawdbox --no-fail-fast`                                                 | exit 0              |
| Format/lint           | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`              | exit 0              |
| Repository formatting | `dprint check`                                                                            | exit 0              |

## Scope

**In scope** (the only source files you should modify):

- `stratum-dsp/src/lib.rs`
- `stratum-dsp/src/features/beat_tracking/mod.rs`
- `stratum-dsp/src/features/beat_tracking/time_signature.rs`
- `stratum-dsp/tests/integration_tests.rs`
- `src/audio.rs` only for a live-value `STRATUM_SCHEMA_VERSION` increment and its test
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- HMM state-time transitions, tempo-segment overlap, or beat deduplication; Plan 006
  owns those concerns.
- BPM estimation and candidate selection.
- External Rekordbox grid parsing or mutation. Supplied external grids must round-trip
  exactly as they do now.
- Changing `BeatGrid`'s serialized shape to add a time-signature field.
- Breaking or removing the existing public
  `detect_time_signature(&[f32], f32) -> Result<(TimeSignature, f32)>` API.
- ML features, private audio fixtures, or corpus-specific thresholds.
- Direct writes to Rekordbox `master.db`; the read-only boundary is non-negotiable.

## Git workflow

- Branch: `codex/005-infer-meter-and-downbeat-phase`
- Use Conventional Commits. Suggested logical commits:
  1. `test(stratum): reproduce meter and phase errors`
  2. `fix(stratum): infer meter from onset accents`
- Do not push or open a PR unless instructed.
- Increment the numeric `STRATUM_SCHEMA_VERSION` by one from its live value after the
  algorithm change; dependency plans may already have moved it beyond `18`.

## Steps

### Step 1: Add meter, ambiguity, and phase regression fixtures

Extend `time_signature.rs` tests with explicit beat-aligned accent vectors. Use a small
value type such as `BeatEvidence { time_seconds, accent }` or equivalent, defined in
`beat_tracking/mod.rs` and accepted by the new detector API. Tests must cover:

1. Uniform 0.5-second beats with uniform accents are **ambiguous** and therefore return
   `FourFour`, phase `0`, and low confidence. This reproduces the present regular-grid
   failure without pretending interval periodicity identifies meter.
2. Four bars of 4/4 with a strong accent every fourth beat selects `FourFour` and the
   correct phase, including a fixture whose first observed beat is phase 2 rather than
   a downbeat.
3. Four bars of 3/4 selects `ThreeFour` and the correct phase.
4. Four bars of 6/8 with a primary accent on beat 1 and a weaker secondary accent on
   beat 4 selects `SixEight` and the correct phase.
5. Non-finite, negative, unsorted, or length-mismatched evidence returns
   `AnalysisError::InvalidInput`.

Add a beat-grid unit test asserting that phase 2 in 4/4 produces downbeats at indices
2, 6, 10, ... rather than index 0. Assert exact selected enums/indices and bounded
confidence, not exact score floats.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::time_signature::tests -- --nocapture` →
before the implementation, the new regression either does not compile against the old
API or fails against the old selection; existing tests remain identifiable.

### Step 2: Preserve the compatibility API and add evidence-aware generation

Keep the public `generate_beat_grid(bpm, confidence, onsets, sample_rate)` function so
downstream users and doctests do not break. Add a `pub(crate)` evidence-aware sibling
(name it clearly, for example `generate_beat_grid_with_evidence`) that accepts sorted
onset times plus finite, non-negative accent strengths. Do not expose the new evidence
type/API publicly unless `rg` finds a current external crate consumer that cannot use
the compatibility wrapper.

Also preserve the exact existing public
`detect_time_signature(&[f32], f32) -> Result<(TimeSignature, f32)>` signature. Make it
a compatibility wrapper that constructs uniform evidence, calls a new crate-private
`detect_time_signature_with_evidence` (or equivalently explicit name), and returns only
the selected meter and confidence while discarding phase. The new internal function
returns the named meter/phase/confidence struct below. Do not overload or replace the
public function with a different parameter or return type, and add a compile-time/unit
test that calls the old signature directly.

The compatibility wrapper must assign uniform evidence and therefore invoke the
documented ambiguous fallback: low-confidence 4/4, phase 0. Do not infer 3/4 from
uniform timing. Validate evidence length, finiteness, non-negativity, and time ordering
at the new boundary.

Represent the detector result as a named struct containing at least:

- `time_signature: TimeSignature`
- `downbeat_phase: usize`, guaranteed `< beats_per_bar`
- `confidence: f32` in `[0.0, 1.0]`

Do not return an unlabelled tuple with a third value; the executor and future callers
must not confuse phase with confidence.

**Verify**:
`cargo test -p stratum-dsp --no-fail-fast features::beat_tracking` → exit 0; the old
public API's existing tests and doctests still compile.

### Step 3: Replace interval periodicity with accent-and-phase scoring

In `time_signature.rs`, replace `score_time_signature(intervals, ...)` with joint
candidate scoring over each `(meter, phase)` pair. Use these explicit invariants:

1. Map each tracked beat to the nearest onset evidence within one quarter of the
   nominal beat interval. Unmatched beats receive zero accent; one onset may not be
   assigned to multiple beats.
2. Robustly normalize finite accent strengths to `[0.0, 1.0]` using observed lower and
   upper values: `(x - min) / (max - min)`. If `max - min <= 1e-6`, evidence is
   uniform/ambiguous; do not manufacture contrast.
3. For 3/4 and 4/4, define candidate score as
   `mean(primary_positions) - mean(other_positions)`. For 6/8, use
   `0.65 * mean(primary) + 0.35 * mean(beat_4_secondary) - mean(other_four)` and reject
   a candidate whose secondary mean exceeds its primary mean. Keep `0.65` and `0.35`
   as named, documented constants.
4. Require at least three complete candidate bars with mapped evidence before allowing
   a non-fallback decision.
5. Multiply each candidate score by mapped-evidence coverage in `[0, 1]`. Define final
   confidence as `coverage * (0.5 * max(best_score, 0) + 0.5 * max(margin, 0))`, clamped
   to `[0.0, 1.0]`.
6. If evidence is uniform, insufficient, `best_score < 0.10`, or the best-vs-runner-up
   margin is `< 0.05`, return `FourFour`, phase 0, and confidence `0.20`.
7. Use deterministic candidate ordering only as a final exact-tie rule; it must not
   turn a tie into high confidence.

These constants are a deliberately fail-closed initial production contract: weak or
ambiguous evidence falls back to low-confidence 4/4 rather than making a positive
meter claim. Dispatching this plan accepts that conservative false-negative bias. The
implementation may tune named thresholds only against the synthetic invariants in
this plan and only toward stricter fallback; any request to optimize real-corpus
accuracy or relax a gate is a separate calibration plan. Do not introduce
genre-specific priors or claim real-corpus accuracy.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::time_signature::tests -- --nocapture` →
exit 0; every meter and phase case passes, and uniform timing returns the fallback.

### Step 4: Generate bars by beat index and selected phase

Pass the complete meter result into grid generation. Replace the fixed-time walk and
"first beat is always a downbeat" assumption with index-based selection:

- Starting at `downbeat_phase`, take every `beats_per_bar`-th tracked beat.
- Set `bars` and `downbeats` to the same selected timestamps, preserving the current
  `BeatGrid` contract.
- Validate `downbeat_phase < beats_per_bar` and reject impossible values.
- Do not synthesize timestamps that are absent from `beats`.

Index-based downbeats remain attached to actual tracked beats under mild tempo drift;
Plan 006 will separately stabilize the underlying variable-tempo beat sequence.

**Verify**:
`cargo test -p stratum-dsp features::beat_tracking::tests -- --nocapture` → exit 0;
the shifted-phase fixture produces the exact expected beat indices.

### Step 5: Derive onset accent evidence in `analyze_audio`

Before converting `onsets_for_beat_tracking` to seconds, compute a deterministic local
transient-strength value from `trimmed_samples` for each onset index. Use RMS over the
80 ms window starting at the onset (`[onset, onset + round(0.080 * sample_rate))`),
clamped to the sample slice. Keep the helper private to `lib.rs`, use
checked/saturating index arithmetic, and normalize only inside the meter detector. The
values must be finite and non-negative.

Call the new evidence-aware grid function for HMM-generated grids. Preserve the old
path exactly when `config.external_beat_grid` is `Some`: do not reinterpret Rekordbox
bars, meter, or phase.

Add focused unit coverage for the accent helper if it contains non-trivial windowing:
a stronger pulse must produce a greater value than a weak pulse, and boundary onsets at
sample 0/end must not panic.

**Verify**:
`cargo test -p stratum-dsp --lib --no-fail-fast` → exit 0; accent helper tests pass.

### Step 6: Tighten the synthetic public-pipeline meter oracle

Using Plan 001's deterministic fixtures:

- Assert the uniform 120 BPM 4/4 kick fixture falls back to bars every four beats, not
  every three. Compare downbeat indices/timestamps with tolerance derived from the
  measured median beat interval.
- Add accent-aware 3/4 and shifted-phase 4/4 pulse fixtures only if the full pipeline
  reliably preserves their accents. Run each at least five times.
- Keep all samples in memory and durations short enough for normal CI.
- Retain the existing external-grid equality tests unchanged.

If full-pipeline 3/4 or 6/8 extraction is unstable while detector-level evidence tests
are deterministic, do not loosen semantic assertions. Keep those cases at the detector
layer and retain the mandatory 4/4 fallback integration regression.

**Verify**:
`for i in 1 2 3 4 5; do cargo test -p stratum-dsp --test integration_tests -- test_analyze_120bpm_kick || exit 1; done` → all five runs exit 0 and assert four beats per bar.

### Step 7: Invalidate cached Stratum semantics

Meter and phase alter `beat_grid`, `grid_stability`, sections, dub-stab, and kick-pattern
fields in cached output. Increment `src/audio.rs::STRATUM_SCHEMA_VERSION` by one from
the value present when this plan starts and update its exact-value test. Do not change
the Essentia version.

**Verify**:
`cargo test -p reklawdbox audio::tests::stratum_result_shape_matches_schema_version` →
exit 0 with the test and constant on the same new value.

### Step 8: Run full verification gates

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

Expected: all commands exit 0; only in-scope files are listed.

## Test plan

- Unit-test meter/phase selection directly with explicit evidence for 4/4, shifted
  4/4, 3/4, 6/8, uniform ambiguity, insufficient evidence, and malformed evidence.
- Unit-test index-based bar selection independently of meter scoring.
- Test the private sample-window accent helper at boundaries and with strong/weak
  pulses.
- Make regular 120 BPM public analysis assert four beats per bar unconditionally.
- Preserve the external-grid round-trip tests to prove Rekordbox grids are untouched.
- Use relationships and tolerances, not exact DSP score snapshots.

## Done criteria

All must hold:

- [ ] Uniform regular beats select fallback `FourFour`, phase 0, with confidence at
      most `0.25`.
- [ ] Explicit synthetic 4/4, 3/4, and 6/8 accents select the correct meter and phase.
- [ ] A shifted 4/4 fixture does not mark the first observed beat as a downbeat.
- [ ] Bars/downbeats are selected from actual beat indices; `bars == downbeats`.
- [ ] `rg -n 'score_time_signature|First beat is always a downbeat' stratum-dsp/src/features/beat_tracking` returns no matches.
- [ ] The original `generate_beat_grid` API and doctest still compile.
- [ ] The original public `detect_time_signature(&[f32], f32)` signature still
      compiles and delegates to uniform-evidence fallback without exposing the
      internal phase-bearing result.
- [ ] External `BeatGrid` values round-trip unchanged.
- [ ] `STRATUM_SCHEMA_VERSION` is incremented once from its live starting value and
      the exact-value test matches; Essentia's version is unchanged.
- [ ] Formatting, dprint, workspace clippy, both crate suites, release build,
      `--version`, and `--help` exit 0.
- [ ] `git diff --check` exits 0; no out-of-scope files changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plans 001, 004, or 008 are incomplete or their mandatory baselines are red.
- The drift check shows the meter or beat-grid APIs no longer match the excerpts.
- Distinguishing the explicit synthetic meters requires using beat intervals as the
  sole evidence or hardcoding fixture-specific timestamps.
- The implementation would change or reinterpret an external Rekordbox beat grid.
- Reliable meter inference requires a private corpus or an undocumented genre prior.
- A reviewer expects the initial constants to maximize corpus accuracy rather than
  enforce the documented fail-closed fallback policy; that requires a separate
  calibration plan and explicit data set.
- Correct phase propagation requires solving the HMM transition/segment problems owned
  by Plan 006; keep this plan on fixed-tempo fixtures and report the dependency.
- A step fails twice or requires an out-of-scope file.

## Maintenance notes

- Interval regularity estimates tempo stability, not meter. Future meter work must use
  accent/structure evidence and retain an explicit ambiguous state/fallback.
- Review phase as carefully as meter: a correct 4/4 label with bars shifted one beat is
  still wrong for all bar-relative features.
- The compatibility wrapper intentionally cannot infer meter from uniform onset times;
  callers with real audio evidence should use the evidence-aware API.
- Any later change to meter, phase, or generated bars changes cached output semantics
  and must reconsider `STRATUM_SCHEMA_VERSION`.
