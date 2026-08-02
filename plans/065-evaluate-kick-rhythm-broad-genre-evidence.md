# Plan 065: Evaluate kick-rhythm evidence for broad genre

> **Status:** Approved and pre-registered on 2026-08-02; feature extraction
> and development evaluation pending
> **Objective:** Determine whether the already-cached beat-relative kick
> placement adds independent, release-relevant broad-genre signal.

## Why this plan exists

Plans 062–064 exhausted deterministic contraction, direct EffNet confidence,
and one supervised adapter over the same pretrained and arrangement inputs.
The last candidate reached 87.72% precision at 49.85% coverage but was unstable
across folds and weak for Breakbeat, IDM, Minimal, and Tech House. Its inputs,
head, and threshold recipe are retired unchanged.

The fresh Stratum schema 21 analysis already contains a low-band onset
histogram aligned to the beat grid. Kick placement is a genuinely new,
mechanism-grounded input: it may separate four-on-the-floor, broken-beat,
halftime, and sparse rhythmic structures. It should not be expected to solve
House versus Techno, because both commonly use four-on-the-floor kicks.

This is the final exposed-corpus experiment in this sequence. A failure stops
local feature combination; it does not trigger another parameter or feature
search.

## Evidence boundary

Reuse the unchanged private Plan 059 development rows and Plan 062 broad
taxonomy:

- manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- EffNet feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- Plan 059 result SHA-256:
  `57099b002344c4840a80f75db5b73b81bc8679336a6f8d8c5b8ff749511d62da`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- Stratum analyzer and schema: `stratum-dsp` version `21`.

Read the Reklawdbox-owned analysis database through a read-only SQLite
connection. Join only by the exact private manifest path and preserve manifest
order. The two absent audio files may use their still-frozen version-21 cache
rows; do not restore or decode them.

Feature extraction may inspect cache availability and validate shapes before
truth-conditioned scoring. It must produce a private ordered artifact and a
semantic source checksum. Freeze that artifact checksum in this plan before
running the classifier.

The label-blind extraction completed before classifier evaluation:

- rows: 670;
- rows with kick evidence: 668;
- rows with a version-21 cache entry but no kick result: 2;
- private kick artifact SHA-256:
  `0b5842935ddbf09e58321a10dce97811790fd77465246cb4eef27a8e9b9d341e`;
- kick feature semantic SHA-256:
  `321b994e907896597ee949358ad8817c3c05a4b912b79d9c80521f40f8cd46a5`;
  and
- ordered source-snapshot SHA-256:
  `cf3554365e2bc45b40d2630430b5814113711c2c0d46dfb3729f31b7b6911c7e`.

The aggregate, truth-independent pattern counts are 367 `four_on_floor`, 35
`broken_beat`, 265 `irregular`, one `sparse`, zero `halftime`, and two missing.
This availability record does not change the frozen feature or evaluation
rules.

## Frozen kick feature vector

For every row, append exactly 74 numeric values:

1. an availability indicator;
2. five one-hot pattern values in this order: `four_on_floor`, `broken_beat`,
   `halftime`, `sparse`, `irregular`;
3. detector confidence;
4. kicks per analysed bar;
5. two one-hot rate-basis values in this order: `main_groove`, `track`; and
6. the 64-bin kick histogram divided by its non-negative L1 sum.

A missing kick result is an all-zero vector. A present result must have a known
pattern and rate basis, finite non-negative confidence and kicks-per-bar, and a
finite non-negative 64-bin histogram. Fail on malformed evidence. Do not use
raw onset count because it is duration-dependent.

No truth label, current genre, EffNet score, v0.33 result, artist, release,
tempo, or model prediction influences feature extraction.

## Frozen candidate

Evaluate exactly the Plan 064 supervised broad adapter with the 74 kick values
appended before training-partition standardization:

- broad maximum style scores;
- mapped v0.33 broad one-hot;
- four arrangement descriptors;
- training-partition EffNet embedding PCA64;
- the frozen kick feature vector; and
- class-balanced one-versus-rest ridge least squares with penalty 10.0 and an
  unpenalized intercept.

All preprocessing remains inside the active training partition. Use the exact
Plan 064 outer-fold fitting, nested inner out-of-fold margin calibration,
minimum-offer rule, threshold tie-break, abstention behavior, metrics, and
deterministic broad-target order. There is one candidate and no ablation or
parameter search.

## Frozen gate

The candidate advances only if every Plan 064 release-development check passes:

1. offered precision is at least 0.90;
2. coverage is at least 0.50;
3. every outer fold makes an offer and has precision of at least 0.85;
4. every target with at least ten truth rows and at least five offers has
   precision of at least 0.75;
5. selective precision improves on the kick-augmented adapter's unselective
   precision by at least 0.10; and
6. every frozen input, semantic checksum, row order, fold, and truth index
   matches.

Report deltas from Plan 064 only as diagnostics. Do not relax the absolute gate
because the new feature improves one metric.

## Outcomes

- **Pass:** freeze the complete recipe and prepare a new 60-track,
  leakage-isolated blind listening holdout in batches of four to six.
- **Fail:** record a bounded negative. Do not tune the kick representation,
  ridge, threshold, target mapping, or feature set on these rows. Stop this
  local experiment sequence. Future classifier work must use a genuinely new
  representation with a plausible production license/runtime, or a new truth
  corpus designed before model fitting.

Neither outcome changes the production classifier or adds a runtime
dependency. Rekordbox, tags, audio, caches, staged changes, and XML remain
unchanged. Private paths, row features, folds, predictions, and margins stay
outside Git.

## Verification

- Unit-test read-only extraction, manifest ordering, missing evidence,
  one-hot ordering, histogram normalization, and malformed evidence rejection.
- Unit-test augmented fold isolation and retain the Plan 064 nested and gate
  tests.
- Require a byte-identical result replay from the frozen feature artifact.
- Inspect committed material for private identities and row-level data.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

This plan is complete when the feature artifact is frozen before scoring, the
single candidate is evaluated and replayed, and the pass or bounded negative is
recorded without post-result changes.
