# Plan 064: Evaluate a supervised broad-genre adapter

> **Status:** Approved and pre-registered on 2026-08-02; development evaluation
> pending
> **Objective:** Test whether one fixed shallow adapter can turn the existing
> EffNet, v0.33, and arrangement signals into release-grade selective broad
> genre suggestions.

## Why this plan exists

Plan 062's deterministic contraction reached 82.32% precision at 24.55%
coverage. Plan 063's direct EffNet projection improved to 88.94% precision at
29.70% coverage, but House and Techno remained weak and fold performance was
unstable.

The direct EffNet representation nevertheless reached 71.04% unselective broad
accuracy. That is enough signal to justify one supervised broad adapter using
the already-fixed Plan 058 machinery. This is the final exposed-corpus adapter
experiment: no parameter, feature, threshold, or target-specific search follows
its result.

## Evidence boundary and frozen inputs

Reuse the 670-row Plan 059 artifacts and the unchanged Plan 062 taxonomy:

- private manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- private feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- Plan 059 result SHA-256:
  `57099b002344c4840a80f75db5b73b81bc8679336a6f8d8c5b8ff749511d62da`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
- Plan 062 broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`

The two currently absent audio files remain valid frozen feature rows. Do not
refresh, remove, or replace them. Current Rekordbox genre, stored profiles, and
the failed 70/20/10 fusion remain excluded.

## Frozen features

Evaluate exactly one feature vector per eligible row:

1. one maximum Discogs-EffNet style score for every modeled broad target;
2. one-hot encoding of the v0.33 recommendation after the same broad mapping,
   with an all-zero vector for abstention or an unmodeled label;
3. loudness range, dynamic complexity, spectral-flux mean, and onset rate,
   with missing values imputed from the active training partition; and
4. the normalized 1280-dimensional EffNet embedding projected onto the top 64
   centered right-singular vectors fitted inside the active training partition.

Every non-PCA feature column is centered and scaled from the active training
partition. Constant columns are removed. PCA is not whitened.

## Frozen adapter

Fit the Plan 058 class-balanced one-versus-rest ridge least-squares head:

- classes are broad truth labels present in the active training partition;
- sample weights are inverse class frequency and normalized to mean one;
- the intercept is unpenalized;
- every other coefficient receives ridge penalty 10.0; and
- ties use the frozen broad-target order.

The predicted label is the largest ridge score. Confidence is the non-negative
margin between the largest and second-largest ridge scores.

## Nested fold-local confidence

For each outer held-out fold:

1. Reserve that fold completely.
2. Within the other four folds, make inner out-of-fold predictions by holding
   out each original fold in turn and fitting all preprocessing and the adapter
   on the remaining three folds.
3. Consider every unique inner margin. A threshold is eligible only if it
   offers at least `max(40, ceil(10% of outer-training rows))` inner predictions
   at precision of at least 0.90.
4. Select the eligible threshold with the most offers, breaking ties by higher
   precision and then higher threshold. If none qualifies, offer no prediction
   for the outer fold.
5. Refit all preprocessing and the adapter on all four outer-training folds.
6. Apply the selected threshold once to the untouched outer fold.

Never select a threshold from in-sample scores or outer-fold performance.
There are no target-specific thresholds, priors, weights, BPM rules, or
exceptions.

## Metrics and development gate

Report both the unselective outer out-of-fold predictions and the nested
selective candidate using the Plan 062 metrics. The selective candidate
advances only if every frozen check passes:

1. offered precision is at least 0.90;
2. coverage is at least 0.50;
3. every fold with offers has precision of at least 0.85, and every fold must
   make at least one offer;
4. every broad target with at least ten truth rows and at least five offers has
   precision of at least 0.75;
5. offered precision improves on this adapter's unselective precision by at
   least 0.10; and
6. every input hash, fingerprint, fold assignment, truth index, and semantic
   checksum matches its frozen value.

## Outcomes

- **Pass:** freeze the complete model and confidence recipe, then prepare a new
  60-track sealed listening holdout in batches of four to six. A holdout pass,
  implementation work, and public-contract review are still required before a
  release.
- **Fail:** record a bounded negative and stop adapter work on this exposed
  corpus. Do not tune against the result. The next proposal must ask a new
  audio-feature question or collect new truth; it cannot be another rearranged
  head over the same features.

Neither outcome writes Rekordbox, tags, audio, cache rows, staged changes, or
XML. Private row identities, paths, predictions, scores, margins, and features
remain outside Git.

## Verification

- Unit-test broad feature construction, training-partition imputation/PCA,
  deterministic ties, nested fold isolation, threshold selection, abstention,
  selective metrics, and frozen gate logic.
- Require a byte-identical replay.
- Inspect committed outputs for private row data.
- Run the standard workspace gate and the maintained Plan 038 corpus gate.

## Done criteria

This plan is complete when the frozen candidate has run once, replayed
byte-identically, and been recorded as a development pass or bounded negative.
Only a pass warrants a new listening holdout.
