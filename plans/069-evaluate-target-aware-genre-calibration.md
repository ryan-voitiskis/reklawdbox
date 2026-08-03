# Plan 069: Evaluate target-aware broad-genre calibration

> **Status:** In progress; candidate preregistered before scoring
> **Objective:** Determine whether honest per-parent abstention can turn the
> frozen CLAP classifier into a release candidate without hiding unstable roots
> or exposing the sealed Plan 066 holdout prematurely.

## Why this plan exists

Plan 068 completed three bounded representation families. CLAP was clearly the
strongest: 91.39% precision at 62.61% nested coverage and 90.13% at 68.70% with
one global deployment threshold. It failed because confidence is not
comparable across predicted parents. Ambient and House remained highly precise
at broad coverage, while Breakbeat and Trance admitted unreliable offers at the
same margin.

A target-aware selective classifier is not a target-specific genre rule. It
uses the unchanged model score and asks only whether each predicted parent's
margin has demonstrated enough precision to be offered. A parent without a
stable threshold remains an explicit abstention. This is preferable to
discarding a useful classifier because one hard root shares its score scale,
or pretending the hard root is supported because its truth count passed a
sampling floor.

## Evidence boundary

Reuse candidate C without changing:

- the 716-row accepted corpus and 575-row balanced development scope;
- the seven frozen parent labels and exact row order;
- five 115-row artist- and release-isolated folds;
- candidate A's 140 cache-native and v0.33 features;
- the pinned 512-value CLAP representation;
- training-partition PCA64;
- class-balanced one-versus-rest ridge with penalty 10 and unpenalized
  intercept; and
- top-score-minus-second-score confidence.

Current genre, artist, release, provider metadata, and holdout identity remain
absent from model features. The Plan 068 results are exposed development
evidence and may motivate this calibration design, but no Plan 066 holdout row,
prediction, or verdict may be inspected before the development gate passes.

## Frozen target-aware calibration

For each outer fold, produce inner out-of-fold predictions and margins using
only the other four folds, exactly as candidate C did. Within those inner
predictions, independently for each predicted parent:

1. consider every observed margin as a possible threshold;
2. require at least eight offers for that predicted parent;
3. require at least 90% precision among those offers;
4. choose maximum offers, then higher precision, then higher threshold; and
5. if no threshold qualifies, make no outer-fold offer for that parent.

Apply the resulting seven thresholds unchanged to the untouched outer fold.
There is no pooled fallback threshold. Threshold selection must not inspect the
outer fold, true parent prevalence outside the active training partition, or
the current genre.

For the deployment view, use candidate C's complete outer out-of-fold
predictions to select one threshold per predicted parent under the same
eight-offer and 90%-precision rule. These are the only thresholds a later
full-fit model may use on the release holdout. A target with no deployment
threshold is an explicit unsupported-output parent even though it remains a
training class and can win internally.

## Frozen development gate

Both nested and deployment views must meet every Plan 068 check:

- at least 90% aggregate offered precision;
- at least 65% coverage across the 575-row release scope;
- at least 85% precision in every outer fold with at least one offer;
- at least 80% precision for every predicted parent with at least eight offers;
  and
- at least five percentage points of paired precision improvement over v0.33.

Also require at least four parents to receive a deployment threshold. Report
coverage against all 716 accepted rows as a diagnostic. Do not add a threshold,
lower a minimum, exclude an error, or change a target after scoring.

## Independent release gate

Only a complete development pass may fit the full model and infer the sealed
60-row Plan 066 holdout. Freeze all predictions before blind review. Review in
batches of at most six with identity and audio only; never show current genre,
prediction, threshold, confidence, sampling stratum, or neighbour hints.

The release holdout requires:

- at least 30 offers among 60 rows;
- at least 90% aggregate offered precision;
- at least 80% precision for every parent with at least five offers;
- no artist, release, decoded-audio, or prior-review leakage; and
- a byte-identical inference replay.

One failed independent evaluation retires the exact model and thresholds. No
holdout-driven threshold or target change follows.

## Product boundary after a pass

A holdout pass proves classification quality, not automatic release readiness.
CLAP's 614.5 MB model requires explicit packaging, license-attribution, download
size, first-run, CPU/MPS latency, and fallback UX review. The first product
surface remains preview-only, names each output-supported parent, explains
abstentions, and stages metadata only through `ChangeManager` and manual XML
import. The existing active-review ledger remains the only verdict-ingestion
path.

## Stop condition

This plan has one development candidate. A failure is a rigorous negative and
does not begin another calibration or representation sweep. A pass advances
only to the already-sealed independent holdout.
