# Plan 009: Preserve missingness in audio classification

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` unless an orchestrator or reviewer told you that it owns the
> index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat e6eb382..HEAD -- src/classify.rs src/audio_profile.rs src/tools/classify_handler.rs src/tools/tests.rs
> ```
>
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding. If the
> defaulted values, vote gates, or auto-stage predicate no longer match, stop
> and report instead of adapting this plan silently.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The classifier currently turns absent Essentia measurements into real musical
evidence: missing danceability becomes the Dancefloor bucket and missing rhythm
regularity becomes the Broken flag. A Stratum-only/BPM-only row can therefore
receive a concrete, Medium-confidence Bass recommendation that is eligible for
auto-staging. Missing measurements must remain unknown; rule-based and
calibrated audio votes should run only when their required observations are
present, while independent enrichment evidence must continue to classify
normally.

## Current state

- `src/classify.rs` owns the rule-based audio profile, vetoes, consensus, and
  audio-only inference.
- `src/audio_profile.rs` owns Fisher-weighted calibrated audio affinities. It
  already skips individual missing scalar values, but missing inputs also
  remove their distance contribution. There is no minimum coverage or
  observed-weight normalization before emitting a vote.
- `src/tools/classify_handler.rs` auto-stages any result whose genre and
  requested confidence tier match. Preserve this `ChangeManager` path; do not
  write Rekordbox directly.
- `src/tools/tests.rs` contains MCP-level classifier/cache regression tests.

Current `src/classify.rs:212-217` makes the energy bucket mandatory:

```rust
struct AudioProfile {
    bucket: EnergyBucket,
    flags: Vec<CharFlag>,
    bpm: f64,
    centroid: Option<f64>,
}
```

Current `src/classify.rs:333-363` invents defaults for two absent features:

```rust
let danceability = audio.danceability.unwrap_or(1.5);

let bucket = if danceability < 1.0 {
    EnergyBucket::NonDancefloor
} else if danceability < 1.5 {
    EnergyBucket::LowEnergy
} else if danceability <= 2.5 {
    EnergyBucket::Dancefloor
} else {
    EnergyBucket::HighEnergy
};

let mut flags = Vec::new();
let dc = audio.dynamic_complexity.unwrap_or(0.0);
let rr = audio.rhythm_regularity.unwrap_or(0.0);

if dc > 10.0 {
    flags.push(CharFlag::Ambient);
}
if rr < 0.5 {
    flags.push(CharFlag::Broken);
} else if rr < 0.8 {
    flags.push(CharFlag::Irregular);
}
```

Current `src/classify.rs:653-658` accepts calibrated affinities without a
coverage contract, and `src/audio_profile.rs:460-486` merely skips missing
individual scalars while scoring whatever remains.

Current `src/audio_profile.rs:457-480` sums only observed contributions, making
a sparse track look artificially close to every prototype:

```rust
let mut total_contribution = 0.0;

for (fi, &fname) in SCALAR_FEATURE_NAMES.iter().enumerate() {
    let track_val = match scalars[fi] {
        Some(v) => v,
        None => continue,
    };
    let stat = match proto.features.get(fname) {
        Some(s) if s.fisher_weight > 0.0 => s,
        _ => continue,
    };
    let z = (track_val - stat.mean) / effective_std;
    let contrib = stat.fisher_weight * z * z;
    total_contribution += contrib;
}
```

Current `src/audio_profile.rs:516-524` converts that unnormalized partial sum
directly into affinity:

```rust
let distance = total_contribution.sqrt();
let adjusted_distance = distance + penalty;
let vote_weight = (AFFINITY_CAP as f64 * (1.0 - adjusted_distance / SCALE))
    .clamp(0.0, AFFINITY_CAP as f64) as f32;
```

Current `src/classify.rs:1236-1268` repeats the missing-to-zero conversion in
audio-only inference:

```rust
let dc = evidence.audio.as_ref()
    .and_then(|a| a.dynamic_complexity)
    .unwrap_or(0.0);
let rr = evidence.audio.as_ref()
    .and_then(|a| a.rhythm_regularity)
    .unwrap_or(0.0);
// danceability is used indirectly via the audio profile's energy bucket
```

Current `src/tools/classify_handler.rs:97-107` will stage any selected tier:

```rust
let track_changes: Vec<TrackChange> = results
    .iter()
    .filter(|r| {
        r.genre.is_some() && levels.iter().any(|l| l.matches_confidence(&r.confidence))
    })
    .map(|r| TrackChange {
        track_id: r.track_id.clone(),
        genre: r.genre.map(String::from),
        ..Default::default()
    })
    .collect();
```

Applicable conventions:

- Optional measurements are represented as `Option<f64>` in `AudioFeatures`.
  Preserve that representation instead of adding numeric sentinels.
- `audio_profile::extract_scalar_features` uses `finite(...)` so NaN/infinity
  become `None`; the new coverage calculation must use the same finite-value
  semantics.
- Today `rhythm_regularity=None` conflates no Essentia cache, extractor failure,
  and the extractor's "four or fewer beats" outcome. Until the analyzer emits
  a reason, this plan makes the explicit fail-closed product decision that all
  three mean **unknown**, never positive `Broken` evidence. Dispatching this
  plan accepts possible false negatives for genuinely beatless tracks rather
  than auto-staging a genre from ambiguous provenance.
- Classification communicates uncertainty through `ClassificationConfidence`,
  `flags`, `evidence`, and `review_hint`; use these existing surfaces rather
  than adding a new public response type.
- The cache-first/no-direct-write contract remains unchanged. User-visible
  metadata changes still flow through `ChangeManager` and XML export.

## Commands you will need

| Purpose                      | Command                                                   | Expected on success             |
| ---------------------------- | --------------------------------------------------------- | ------------------------------- |
| Targeted classifier tests    | `cargo test -p reklawdbox classify`                       | exit 0; all matching tests pass |
| Targeted audio-profile tests | `cargo test -p reklawdbox audio_profile`                  | exit 0; all matching tests pass |
| Tool regression tests        | `cargo test -p reklawdbox classify_tracks`                | exit 0; all matching tests pass |
| Format                       | `cargo fmt --check`                                       | exit 0, no diff                 |
| Docs/config format           | `dprint check`                                            | exit 0                          |
| Lint                         | `cargo clippy -p reklawdbox --all-targets -- -D warnings` | exit 0, no warnings             |
| Full crate tests             | `cargo test -p reklawdbox --no-fail-fast`                 | exit 0; all tests pass          |

## Scope

**In scope** (the only source files you may modify):

- `src/classify.rs`
- `src/audio_profile.rs`
- `src/tools/classify_handler.rs` only if an additive missing-evidence flag or
  auto-stage regression assertion cannot be implemented without a handler
  change
- `src/tools/tests.rs`
- `plans/README.md` for the status row only

**Out of scope**:

- Genre taxonomy, aliases, BPM ranges, source vote weights, and label mappings.
- Recalibrating danceability/rhythm thresholds when values are actually
  present.
- Cache hydration, analyzer schemas, and Essentia extraction.
- Adding a rhythm-missing reason/status to the analyzer output; that is a
  separate schema-versioned follow-up if beatless tracks need distinct rules.
- Direct writes to Rekordbox `master.db`; it must remain read-only.
- Replacing `ChangeManager` or XML export for user-visible metadata.
- Documentation/SOP changes unless the implementation changes the public tool
  contract beyond making existing "insufficient evidence" behavior truthful.

## Git workflow

- Branch: `codex/009-preserve-classification-missingness`
- Use Conventional Commits; preferred final message:
  `fix(classify): preserve missing audio evidence`.
- Keep commits limited to this plan. Do not push or open a PR unless the
  operator explicitly requests it.

## Steps

### Step 1: Characterize missing-value regressions before changing logic

Add unit tests in `src/classify.rs` using the existing `make_audio` and
`make_evidence` helpers, but explicitly set optional fields to `None`. Cover:

1. Missing danceability does not create a Dancefloor/HighEnergy bucket and
   cannot trigger the fast+dancefloor Bass veto.
2. Missing rhythm regularity does not add `Broken` or `Irregular`. Cover both
   an otherwise successful Essentia record with this field absent and a track
   with no Essentia cache; both are unknown under the explicit fail-closed
   policy.
3. BPM-only audio with no enrichment returns `genre: None`,
   `ClassificationConfidence::Insufficient`, and stable missing-evidence flags.
4. Complete observed audio retains an existing representative classification
   result, proving the thresholds for present data did not change.
5. Strong Beatport/Discogs evidence can still produce its prior confidence
   when optional audio fields are absent; missing audio must not penalize
   independent sources.

Add an MCP-level test in `src/tools/tests.rs` that seeds a fresh Stratum cache
entry but no Essentia cache entry, calls `classify_tracks` with
`auto_stage=["medium"]`, and asserts `staging.staged == 0` for the BPM-only
track. Model the cache setup after the existing stale/fresh classifier tests
around `src/tools/tests.rs:2408-2500`.

**Verify**: `cargo test -p reklawdbox classify -- --nocapture` → the new
regression tests fail for the intended current behavior, while unrelated
classifier tests pass. Record the exact failing assertions in the commit/PR
notes; do not weaken them in later steps.

### Step 2: Represent energy and rhythm evidence as genuinely optional

In `src/classify.rs`:

1. Change `AudioProfile.bucket` to `Option<EnergyBucket>` (or rename it
   `energy_bucket` if that makes every call site clearer).
2. Derive the bucket with `audio.danceability.map(...)`; do not use a default.
3. Add `Ambient`/`Atmospheric` only when `dynamic_complexity` is `Some` and
   crosses its current thresholds. Add `Broken`/`Irregular` only when
   `rhythm_regularity` is `Some` and crosses its current thresholds.
4. Update comparisons, ordering checks, evidence formatting, depth demotion,
   BPM fallback, family-affinity helpers, and vetoes to require the exact
   observed field they semantically use. Do not treat `None` as any energy
   bucket and do not make `None` sort above or below a real bucket.
5. Add stable result flags such as `missing-danceability` and
   `missing-rhythm-regularity` when an audio cache exists but those values do
   not. Avoid duplicate flags.

The fast Bass veto must require `Some(LowEnergy | Dancefloor | HighEnergy)`;
the absence of danceability is not evidence that a track is dancefloor. Audio
evidence strings should say `energy-unknown` or omit the energy adjective,
never print a fabricated category.

**Verify**: `cargo test -p reklawdbox classify` → all classifier tests pass,
including the new missing-energy and missing-rhythm cases.

### Step 3: Gate audio-only rules on the fields they require

Refactor `audio_only_inference` so it never calls `unwrap_or(0.0)` for an
optional measurement. Use these explicit rules:

- D.1 requires an observed danceability/energy bucket. Without it, return no
  recommendation with `Insufficient` confidence and a review hint/flag stating
  that energy evidence is missing.
- D.2 rhythm branches require observed `rhythm_regularity`. If it is missing,
  do not infer regular or broken rhythm from BPM. Return no audio-only genre
  unless another existing branch has all of its own required observed inputs.
- Dynamic-complexity and spectral-centroid refinements run only when their
  values are present.
- Audio-only results based on incomplete observations must never rise to
  Medium or High confidence. Preserve the existing Low/Insufficient ceiling
  for complete audio-only paths.

Do not change how enrichment-only consensus works.

**Verify**: `cargo test -p reklawdbox classify` → all tests pass; the BPM-only
case is `Insufficient` with no genre, and present-data fixtures retain their
expected results.

### Step 4: Require meaningful coverage before calibrated audio votes

Fix coverage per prototype rather than with a count-only pre-gate. In
`src/audio_profile.rs::score_track`, track all of:

- `eligible_optional_weight`: the sum of every positive Fisher weight for an
  optional scalar supported by the prototype, plus `0.05` for each usable
  prototype timbral centroid/mean-distance family. Exclude Rekordbox BPM;
- `observed_optional_weight`: the subset of that exact optional weight whose
  corresponding finite track feature/vector is present and dimensionally valid;
- `eligible_optional_features`: the number of those eligible scalar/timbral
  families, excluding BPM;
- `observed_optional_features`: observed eligible scalar features excluding
  the always-present Rekordbox BPM, plus one for each observed eligible timbral
  family.

Add named, documented constants with these initial conservative gates:

```rust
const TARGET_CALIBRATED_OPTIONAL_FEATURES: usize = 3;
const MIN_CALIBRATED_WEIGHT_COVERAGE: f64 = 0.50;
```

Change `score_track` to return `Option<AudioAffinity>` and return `None` unless
`eligible_optional_weight > 0`, at least
`min(TARGET_CALIBRATED_OPTIONAL_FEATURES, eligible_optional_features)` optional
families are observed, and
`observed_optional_weight / eligible_optional_weight >= 0.50`. `score_all`
should use `filter_map`. A BPM-only prototype is deliberately ineligible. For
small-N prototypes with only one or two eligible optional families, fully
observing all available families remains sufficient; do not impose an
impossible absolute three-family gate. For prototypes with three or more,
three negligible features still cannot compensate for absent features carrying
most discriminative weight. The 50% threshold is an explicit fail-closed
majority-evidence policy; dispatching this plan accepts that conservative gate.

Keep the always-observed BPM contribution separate. Normalize only the observed
optional squared contribution back to the prototype's full optional weight
budget before adding BPM and taking the square root:

```text
normalized_squared_distance = bpm_squared_contribution
    + observed_optional_squared_contribution
      * eligible_optional_weight / observed_optional_weight
```

For fully observed tracks the multiplier is exactly one, preserving current
distance/vote behavior. For partially observed tracks it prevents omitted
features from reducing distance merely by disappearing. Include the fixed
`0.05` timbral family weights consistently in eligible optional weight,
observed optional weight, feature count, and contribution. Reject non-finite inputs and invalid
or non-positive `mean_dist`; do not synthesize zeroes.

Add coverage metadata (`observed_optional_features` and weight-coverage ratio)
to `AudioAffinity` or otherwise make it available to tests and
`format_evidence`. Include a concise coverage marker in calibrated evidence so
a reviewer can distinguish a full-data vote from a threshold-level partial
vote. Do not expose eligible feature values themselves beyond the existing
top-contribution evidence.

Add focused `audio_profile.rs` tests for BPM-only; small-N prototypes with one
and two eligible optional families; prototypes with at least three families;
adequate count but less-than-50% Fisher coverage; threshold coverage; full
coverage; complete/malformed timbral vectors; and NaN cases. Construct N=5-9
and N=10-14 calibration fixtures matching the live `max_features = n / 5`
selection so a fully observed small prototype is not made impossible. Add a
regression where two tracks have equivalent observed deviations but one omits
an eligible feature; omission must not make its normalized distance smaller.

In `src/classify.rs::gather_votes`, make the lack of calibrated coverage
observable through a result flag/evidence line only when a registry was
available and an audio record existed; do not imply that calibration was
attempted when no registry exists. If returning this metadata would require a
large public type redesign, keep `score_all` returning an empty vector and add
the flag in `classify_track_with_profiles` through the same coverage helper.

**Verify**: `cargo test -p reklawdbox audio_profile` → all coverage and existing
profile tests pass.

### Step 5: Prove auto-staging cannot convert missingness into metadata

Run the MCP-level test from Step 1. If `src/tools/classify_handler.rs` requires
no production change, leave it untouched. If an explicit defensive gate is
needed, limit it to rejecting results flagged as missing/insufficient audio
evidence when the result itself came solely from audio; do not block valid
enrichment-backed Medium/High results.

Confirm the test inspects `ChangeManager`/the returned `staging` object and
does not assert against Rekordbox database mutation.

**Verify**: `cargo test -p reklawdbox classify_tracks` → all tool tests pass and
the BPM-only auto-stage regression reports zero staged changes.

### Step 6: Run the complete Rust gate

Run all repository checks for server/CLI changes. Review `git diff` afterward
for threshold or taxonomy changes that are outside scope.

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
```

Expected: every command exits 0; no warnings; no source outside the scope list
is modified.

## Test plan

- `src/classify.rs` unit tests:
  - no danceability does not become dancefloor;
  - no rhythm regularity does not become broken;
  - BPM-only/no-enrichment is insufficient and genre-less;
  - complete audio preserves a representative old decision;
  - enrichment confidence is unaffected by missing optional audio.
- `src/audio_profile.rs` unit tests:
  - a BPM-only prototype/track fails coverage;
  - small-N prototypes with one or two eligible optional families require all
    of those families and remain reachable when fully observed;
  - three or more optional features with less than 50% eligible optional
    Fisher/timbral weight still fail;
  - count and weight coverage at/above both thresholds pass;
  - full coverage preserves the pre-change distance formula;
  - omitted eligible features do not make normalized distance artificially
    smaller;
  - complete valid timbral families contribute count and fixed weight;
  - malformed vectors, invalid mean distances, and non-finite values do not
    count.
- `src/tools/tests.rs` integration test:
  - a fresh Stratum-only cache entry cannot stage a Medium audio-only genre.
- Structural patterns: use `make_audio`/`make_evidence` in
  `src/classify.rs:1516+`, the finite-value tests in
  `src/audio_profile.rs:1015+`, and cache fixture setup in
  `src/tools/tests.rs:2408+`.

## Done criteria

All of the following must hold:

- [ ] `rg "danceability\.unwrap_or|rhythm_regularity\.unwrap_or" src/classify.rs` returns no matches.
- [ ] Missing danceability cannot satisfy an energy-bucket predicate.
- [ ] Missing rhythm regularity cannot produce `Broken` or `Irregular`.
- [ ] BPM-only/no-enrichment classification returns no genre and
      `Insufficient` confidence.
- [ ] Calibrated audio votes require `min(3, eligible_optional_features)`
      observed families and at least 50% of each prototype's eligible optional
      discriminative weight; BPM-only prototypes cannot vote.
- [ ] Sparse optional distance is normalized by
      `eligible_optional_weight / observed_optional_weight` without scaling
      the BPM contribution;
      omitting a feature cannot make a track look closer merely by removing its
      contribution.
- [ ] Fully observed prototypes with one, two, or at least three eligible
      optional families preserve the old distance/vote calculation.
- [ ] The MCP auto-stage regression observes zero staged changes.
- [ ] Existing complete-audio and enrichment-backed classification tests pass.
- [ ] `cargo fmt --check`, `dprint check`, clippy, full crate tests, release
      build, `--version`, and `--help` exit 0.
- [ ] `git diff --name-only` contains only in-scope files and the optional
      `plans/README.md` status update.
- [ ] No code writes to Rekordbox `master.db`; staging still uses
      `ChangeManager` and XML export.

## STOP conditions

Stop and report back if:

- The current code no longer uses the defaults shown above, or a prior change
  has already introduced a missing-evidence contract.
- Making `EnergyBucket` optional requires changing the public genre taxonomy,
  source weights, or cache schema.
- Existing tests reveal that a documented workflow intentionally treats
  missing danceability/rhythm as observed values.
- A maintainer requires `rhythm_regularity=None` from a genuinely beatless
  track to count as `Broken`; provenance must first be added to the analyzer
  output with an explicit schema/cache decision.
- Preserving enrichment-only confidence would require weakening the new
  BPM-only regression.
- The tool-level test would require a real Rekordbox database, external
  network request, or private audio file; use synthetic SQLite/cache fixtures
  only.
- Any fix appears to require a direct Rekordbox write or bypassing
  `ChangeManager`/XML.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- Any new optional audio feature used by a rule must have an explicit
  `Some(...)` requirement; absence is never a numeric value.
- Review future changes to `SCALAR_FEATURE_NAMES`, prototype Fisher weights,
  and timbral families together with the count gate, weight-coverage gate, and
  distance normalization. The always-present Rekordbox BPM must remain excluded
  from the optional feature count and optional coverage numerator/denominator;
  its Fisher contribution remains in distance only and must never make missing
  optional evidence look sufficiently covered.
- Reviewers should scrutinize every conversion from `Option<f64>` and every
  `EnergyBucket` comparison for accidental fallback semantics.
- `None` remains unknown until the analyzer carries a reason. If a later schema
  distinguishes `too_few_beats` from extraction failure/no cache, add a
  separately tested beatless-track rule rather than reverting to a blanket
  default.
- This plan deliberately does not tune present-value thresholds or source
  weights. The relative family gate and 50% coverage are fail-closed safety
  policies; empirical accuracy tuning requires a separate calibrated data set.
