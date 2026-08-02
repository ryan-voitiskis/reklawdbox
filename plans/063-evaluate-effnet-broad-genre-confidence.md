# Plan 063: Evaluate EffNet broad-genre confidence

> **Status:** Complete on 2026-08-02; direct EffNet broad confidence is a
> bounded negative
> **Objective:** Test whether the already-frozen Discogs-EffNet style output can
> produce release-grade selective broad-genre suggestions under the unchanged
> Plan 062 taxonomy.

## Why this plan exists

Plan 062 showed that conservative label contraction improves the v0.33
classifier's precision but reaches only 82.32% precision at 24.55% coverage.
The rule is retired unchanged.

The previously evaluated Discogs-EffNet representation produced much stronger
fine-label and coarse-family development signal, but its old evaluation did not
define the user-facing broad taxonomy or measure selective precision and
coverage. This plan asks that narrower question without decoding audio,
rerunning inference, changing the broad mapping, or tuning a fine classifier.

## Frozen inputs

Reuse the existing 670-row Plan 059 artifacts:

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

The two currently absent audio files remain valid frozen feature rows because
their mel inference and truth were sealed while the files were present. Do not
refresh, remove, or substitute them for this evaluation.

## Frozen representation

Use only the existing direct style scores from the feature artifact. Do not use
the embedding, arrangement features, v0.33 recommendation, current Rekordbox
genre, stored profile, or the previously failed 70/20/10 fusion.

For each broad target, take the maximum style score among its mapped canonical
fine genres. This matches the underlying multi-label style-head semantics and
avoids giving larger parent groups an automatic sum advantage.

The unselective prediction is the broad target with the largest score. Break
ties by the frozen canonical broad-target order. Rows whose truth is unmodeled
remain excluded.

## Frozen confidence rule

Define confidence as the non-negative margin between the largest and
second-largest broad scores.

Choose a threshold independently for every held-out fold:

1. Use only the other four folds as training rows.
2. Consider every unique training margin plus a threshold above the maximum
   that offers no rows.
3. A threshold is eligible only if it offers at least
   `max(40, ceil(10% of training rows))` training predictions and their offered
   precision is at least 0.90.
4. Select the eligible threshold with the greatest training coverage. Break a
   tie by higher precision, then the higher threshold.
5. If no threshold is eligible, offer no predictions in that held-out fold.
6. Apply the selected threshold once to the held-out fold. Never revise it from
   held-out performance.

There are no target-specific thresholds, priors, weights, exceptions, BPM
rules, or post-result calibration.

## Metrics

Report the same aggregate selective metrics as Plan 062:

- eligible rows, offers, abstentions, coverage, correct offers, offered
  precision, and accuracy;
- macro recall and macro F1;
- fold thresholds, training coverage/precision, and held-out
  coverage/precision; and
- per-target support, offers, precision, recall, abstentions, and leading
  confusions.

Also report an unselective broad projection as a diagnostic. Private row
predictions, identities, paths, folds, scores, and margins remain outside Git.

## Pre-registered development gate

The cross-fitted selective candidate advances only if every condition passes:

1. offered precision is at least 0.90;
2. coverage is at least 0.50;
3. every fold's offered precision is at least 0.85;
4. every broad target with at least ten truth rows and at least five offers has
   offered precision of at least 0.75;
5. offered precision improves on the unselective EffNet broad projection by at
   least 0.10; and
6. all input hashes, corpus fingerprints, folds, truth indices, and the broad
   semantic checksum match their frozen values.

## Outcomes

- **Pass:** freeze the model, mapping, aggregation, threshold-selection
  algorithm, and result checksum; then prepare the Plan 062 sealed broad-genre
  holdout in listening batches of four to six.
- **Fail:** record a bounded negative. Do not tune thresholds or parent groups
  on these rows. Use aggregate failure structure to decide whether one new
  representation or audio-feature experiment is justified.

Neither outcome authorizes a production dependency, cache change, MCP/CLI
surface, Rekordbox write, tag write, or release.

## Verification

- Unit-test hash/fingerprint rejection, mapping completeness, max aggregation,
  deterministic ties, fold isolation, minimum training offers, threshold
  selection, no-eligible-threshold abstention, selective metrics, and gate
  logic.
- Require a byte-identical replay.
- Inspect the committed result for private identities, paths, predictions,
  scores, margins, or folds.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

This plan is complete when the frozen evaluation has run once, replayed
byte-identically, and recorded an aggregate pass or bounded negative. A holdout
is prepared only after a pass.

## Recorded result

The aggregate report is
[Broad Genre EffNet Evaluation](../docs/genre-classification/broad-genre-effnet-evaluation.md).

The unselective projection reached 71.04% broad accuracy. Cross-fitted margin
selection increased offered precision to 88.94% but reduced coverage to 29.70%.
It failed three frozen checks:

- offered precision was below 90%;
- coverage was below 50%; and
- fold 1 offered precision was 84.78%, below the 85% fold floor.

The per-target precision guard passed, as did the required improvement over the
unselective projection. Strong selective results for Ambient and Reggae did not
repair weaker House and Techno precision or the absence of useful offers for
several smaller targets.

The result replayed byte-identically. No holdout is warranted, and the margin
thresholds, target mapping, and direct projection must not be tuned against
these rows.

Reproducibility record:

- private aggregate result SHA-256:
  `c89d5f8db82d6926ef919c1497693ee9f72614355efd3c5893e422844fd08e79`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`

The remaining justified candidate is one separately frozen supervised broad
adapter. It may reuse the already-fixed class-balanced ridge penalty and
training-fold PCA machinery from Plan 058, but it must use nested fold-local
confidence calibration and receive no parameter search.
