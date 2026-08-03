# Plan 069: Evaluate target-aware broad-genre calibration

> **Status:** Complete; bounded negative, sealed holdout remains untouched
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

## Pre-score freeze

The target-aware evaluator was frozen before its first score at commit
`b7585317da4f9ff5ef8fc009a6d5cf9a5a436b8b` with source SHA-256
`f64878d00fddcf02070edc227e3a1bd208b86a90b718a1f5c1833bfc23ae2f2f`.
Its candidate-C config SHA-256 is
`b5a525ac98f2f58c5383a8301c1388d13188ff8461532b1e0f5aa4905a3ecfea`.
That config and evaluator bind the following inputs:

- development manifest:
  `caf76dbe8156943a139a8ab73e8d8b492a1d74bfe1b1e9c80898104ff21f5580`;
- feature manifest:
  `d50519a80812a8f5705a8db834ca2764618f0fde18d3ce99ad8e981724c60e24`;
- cache-native feature matrix:
  `e93610e70ad70b6c02640a7161a0bc5b444717bea2b6a521c526a601db7b72ab`;
- cache-native feature summary:
  `5e8769b4fe435214dfe12c8f16f7da5dcbd17e406f0120e970ef724ee6d05d61`;
- representation manifest:
  `676de6f150a811494f255a667d61fb449c149a9cfbf5fa968a74327e2afd0e67`;
- CLAP feature matrix:
  `8839242f64c7aa183e055ee3c15e1e359ea77a568c0b10a29b6475f93b81697b`;
  and
- CLAP feature summary:
  `9d6c34917fdbc6fac7261e90b4f6af252e187e0a691498b3552aae51123f7113`.

The focused evaluator suite passed nine tests before this freeze. The four-parent
breadth check applies to the deployment threshold set exactly as specified
below; nested folds remain governed by the unchanged Plan 068 quality gates.

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

## Development result

Target-aware calibration improved the operational shape of candidate C but did
not pass the frozen nested gate. The honest nested view offered 367 of 575 rows
at 91.28% precision and 63.83% coverage. Every outer fold cleared 85% precision
and paired precision improved over v0.33 by 20.96 percentage points, but
coverage remained 1.17 points below the 65% floor. More importantly, nested
Electro precision was only 72.22% on 18 offers, below the 80% per-parent floor.
Breakbeat and Trance correctly received no nested offers.

The deployment diagnostic passed every frozen development check in isolation:
it offered 410 rows at 91.71% precision and 71.30% coverage across six parents,
with no Breakbeat threshold. It does not override the nested failure because
its per-parent thresholds saw every development outcome. Coverage against all
716 accepted rows was 51.26% in the nested view and 57.26% in the deployment
view.

The exact evaluation replay was byte-identical. Result SHA-256 is
`6a9b6d6b2896fa8f396fd267cfe0f6625bf77e71899d2293c7d00cf5aad0064a`;
both result files are mode 0600. The sealed Plan 066 holdout received no
inference and remains unopened. Under this plan's stop condition, the exact
candidate is retired and no additional calibration or representation sweep
begins from this result.

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
