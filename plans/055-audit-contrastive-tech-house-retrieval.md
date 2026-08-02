# Plan 055: Audit contrastive Tech House retrieval before adding DSP

> **Executor instructions:** Read this plan in full before changing code. Use
> an isolated worktree from `main`, preserve the concurrent audio-integrity
> work, and keep all private track identities and listening verdicts outside
> Git. This is an offline experiment, not permission to expose an MCP tool,
> change the classifier, write Rekordbox metadata, or modify `stratum-dsp`.

## Status

- **Priority:** P1
- **Effort:** M for the existing-feature audit; L only if a DSP prototype is
  independently justified
- **Risk:** MED
- **Category:** classification evidence / contrastive retrieval
- **Planned at:** `main` commit `d5603cb`, Reklawdbox v0.33.0, 2026-08-02
- **Predecessor:** Plan 053, completed as a bounded negative result on branch
  `codex/053-discover-mislabeled-genres`

## Decision

Do not run another listening batch with Plan 053's best-single-seed ranker.
First test whether the existing cached features can separate ear-verified Tech
House from adjacent House, Techno, Minimal Techno, and Deep Techno when the
problem is formulated contrastively.

Tech House is the first target because it now has 16 ear-verified development
anchors spanning materially different strains. Minimal remains a separate
target: apply the winning method to it only after the Tech House experiment
passes, so results from the two boundaries cannot tune each other.

## Evidence from Plan 053

The final frozen review page produced no confident Tech House positives in
either the ranked or control cohort. Best-match aggregation
`max(similarity(candidate, seed))` repeatedly selected adjacent House and
Techno tracks that resembled one seed in tempo, energy, brightness, rhythm, or
broad timbre. The control margin did not rescue the ordering because positive
similarity remained primary.

This result rules out post-hoc threshold tuning of that experiment. It does
not yet show that the cached feature set is insufficient; it may show that a
single-neighbour decision rule is too permissive for a heterogeneous genre.

## Non-goals

- Do not merge or cherry-pick the experimental Plan 053 MCP surface.
- Do not auto-classify, stage Genre changes, write audio tags, or mutate
  Rekordbox's database.
- Do not use current Genre as a positive similarity feature.
- Do not treat retailer or web canonicality as ear verification.
- Do not open another review batch until the audit configuration is frozen.
- Do not add sidechain, sub-rumble, swing, kick-pattern, or cache-schema fields
  during the existing-feature audit.
- Do not claim collection-wide accuracy from this small development corpus.

## Frozen evidence roles

Create a private manifest at
`/tmp/reklawdbox-tech-house-contrast-audit-v1.json`. Record exact Rekordbox ID,
recording/version, release leakage group, and one role per track:

- **positive:** the 16 ear-verified Tech House anchors;
- **hard negative:** confident ear-verified House, Techno, Minimal Techno, or
  Deep Techno examples;
- **boundary:** ambiguous tracks, used for reporting only; and
- **excluded:** every seed, candidate, control, or release sibling exposed in
  earlier Plan 053 review batches.

Before computing scores, assign each positive to one independently motivated
strain such as early/stripped, deeper/minimal-adjacent, or modern/polished.
Freeze those assignments using listening and scene/release context, not the
feature-space clusters being evaluated. A crossover track may be marked as a
boundary between strains but must not be duplicated as two observations.

Require at least three independent release groups in every strain used for
cross-validation. Keep boundaries out of training, threshold selection, and
headline metrics.

## Experiment 1: existing-feature audit

### Inputs and coverage

1. Resolve every private-manifest recording against the live read-only
   library and reject identity or version ambiguity.
2. Require the current Full Stratum and Essentia cache identities and a valid
   timbral vector for every scored row.
3. Report missing, stale, degraded, and leakage-excluded counts separately.
4. If a small number of fixed rows need analysis, refresh only those paths at
   low priority with one worker after confirming no competing audio workload.
   Do not hydrate or rescan the whole collection.

### Pre-registered formulations

Evaluate the same normalized feature blocks under a small fixed matrix:

1. Plan 053 baseline: maximum similarity to any positive seed.
2. Pooled positive density: mean of the nearest three positive seeds.
3. Per-strain positive density: maximum, across strains, of the mean of the
   nearest three seeds within that strain.
4. Per-strain contrastive density: formulation 3 minus the strongest adjacent
   genre density.

For a candidate `x`:

```text
positive_density_s(x) = mean(top_3 similarity(x, positive seeds in strain s))
positive_density(x) = max_s positive_density_s(x)
negative_density_g(x) = mean(top_5 similarity(x, hard negatives in genre g))
negative_density(x) = max_g negative_density_g(x)
contrast_margin(x) = positive_density(x) - negative_density(x)
```

When a fold leaves fewer than three eligible seeds in a strain, use all
remaining seeds and report the reduced `k`. Exclude the held-out recording's
entire release leakage group from every density calculation.

Test three feature blocks without expanding the matrix after results are
opened:

- timbre only;
- scalar groove/production axes only: BPM, rhythm regularity, energy, and
  brightness; and
- the existing combined Plan 053 weights.

For contrastive formulations, order by `contrast_margin` first and
`positive_density` second. A high match to one positive cannot outrank dense
negative evidence merely because it resembles that seed.

### Validation protocol

Use leave-one-release-group-out cross-validation. In each fold, hold out one
positive release group and a genre-balanced set of hard negatives; fit any
normalization or threshold on the remaining development rows only.

Report:

- held-out positive percentile among hard negatives, macro-averaged by strain;
- area under the precision-recall curve;
- recall and false-positive rate at the selected threshold;
- false-positive rate separately for House, Techno, Minimal Techno, and Deep
  Techno; and
- fold-by-fold comparison with the frozen best-single-seed baseline.

Boundaries receive scores for interpretation but never count as a positive or
negative. Preserve per-axis contributions so failures remain audible and
explainable rather than becoming an opaque composite.

### Gate to a listening experiment

Freeze an existing-feature formulation only if all of the following hold:

1. At least 12 of 16 held-out positives have a positive contrast margin.
2. Every represented strain has a median held-out positive percentile of at
   least 0.75 among hard negatives.
3. Precision-recall area improves by at least 0.10 absolute over the frozen
   best-single-seed baseline.
4. A cross-validated threshold reaches at least 0.70 Tech House recall while
   keeping each adjacent genre's false-positive rate at or below 0.20.
5. The chosen formulation wins in at least two thirds of release-group folds,
   rather than relying on one unusually easy strain or release.

Treat these as experiment gates, not production accuracy promises. If no
configuration passes, retain the result unchanged and proceed only to the
bounded feature audit below.

## Experiment 2: feature audit only if required

Do not begin by implementing the existing sidechain/sub-rumble proposals.
Their scene hypotheses are useful, but sidechain may describe modern Tech
House while missing early London material, and sparse arrangement alone can
describe Minimal or Techno without identifying either genre.

Measure candidate signals offline on the same leakage-safe folds, one family
at a time:

- sidechain or beat-synchronous duck-and-recovery depth;
- sustained sub-rumble between kick transients;
- kick/bass temporal interaction and low-frequency occupancy;
- swing and microtiming of percussion; and
- vocal/hook density and long-form arrangement microvariation.

For every signal, state the audible mechanism, expected strains, expected hard
negatives, missingness behavior, and failure cases before opening results. A
feature is worth an implementation proposal only when it:

1. separates positives from at least one named adjacent genre across
   release-group folds;
2. does not simply separate modern from early Tech House;
3. materially improves the Experiment 1 gate without degrading another
   represented strain; and
4. is stable enough to justify analysis cost and a future cache-version bump.

Use an isolated research harness and private result files. No new DSP output,
cache field, classifier rule, or MCP schema lands in this plan. If a signal
passes, write a separate implementation plan with corpus fixtures, performance
budget, cache migration, and held-out validation.

## Experiment 3: one small blind batch

Only after a formulation passes and is frozen:

1. Select six ranked candidates from currently broad House/Techno labels.
2. Draw four genre- and BPM-stratified random controls from the same universe.
3. Exclude every development track, prior review row, boundary, and release or
   artist-version leakage sibling.
4. Shuffle the ten rows with a fixed random seed and keep the mapping in
   `/tmp`; export a playlist only after an explicit operator request.
5. Ask for one of `positive`, `boundary`, `negative`, or `unsure`, with short
   listening notes encouraged but not required.

Pass only if the six ranked rows contain at least five confident positives and
their positive hit rate is at least twice the control hit rate. Boundaries do
not count as positives. Retire the frozen configuration unchanged if it fails.

If it passes, every development and review row remains training material.
Source a fresh independent holdout before calibrating or changing a profile;
exclude all release siblings and score that holdout only after the profile is
frozen.

## Delivery sequence

1. Import and verify the separately approved XML that normalizes the 16 Tech
   House anchors and unions them into `genre_verified`.
2. Freeze the private evidence manifest, strain assignments, exclusions,
   feature matrix, and release-group folds.
3. Implement only the smallest offline adapter needed to read current cached
   features into pure evaluation functions. Do not add an MCP or CLI surface.
4. Add synthetic mandatory tests for density calculation, margin-first
   ordering, leakage exclusion, missing evidence, and deterministic folds.
5. Run Experiment 1 and record an aggregate, identity-free report under
   `docs/genre-classification/`.
6. Either freeze a passing formulation or run the bounded feature audit.
7. If a formulation passes, create one ten-track blind batch and stop for the
   operator's listening verdicts.

## Verification

For plan-only changes:

```bash
git diff --check
```

For an offline audit implementation, discover the exact current test filters
and at minimum run:

```bash
cargo fmt --check
dprint check
cargo test -p reklawdbox classification -- --nocapture
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

If implementation code is proposed for merge, run the full workspace gate in
`AGENTS.md`. Public MCP/help/SOP contract gates are intentionally out of scope
because this plan must not add a public surface.

## STOP conditions

Stop and report if:

1. private IDs, paths, listening verdicts, or review mappings enter Git;
2. a fold includes the held-out track, its release sibling, or an exposed
   prior-review row in its seed or control set;
3. incomplete/degraded evidence is silently scored alongside complete rows;
4. strain assignments or the experiment matrix are changed after metrics are
   opened without creating a new experiment version;
5. a boundary is converted into label truth to improve a metric;
6. the current Genre or musical key is needed as a positive scoring weight;
7. whole-library analysis or overlapping heavy compute becomes necessary;
8. the audit requires a classifier, cache-schema, DSP-output, MCP, tag, or
   Rekordbox mutation; or
9. the blind batch exceeds ten tracks or reuses any development evidence.

## Done criteria

This plan is complete when one of two outcomes is recorded without private
library data:

- **successful formulation:** a leakage-safe existing or independently
  validated feature formulation passes the offline gate, then passes one
  frozen ten-track blind batch; or
- **bounded negative:** every pre-registered existing-feature formulation
  fails, no independently tested feature justifies implementation, and no
  production or metadata surface is changed.

In either case, preserve the operator's listening decisions as the source of
genre truth and keep all development rows out of any later sealed holdout.
