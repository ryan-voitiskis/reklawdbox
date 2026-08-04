# Plan 071: Develop open-set Genre Intelligence

> **Status:** In progress; protocol frozen before candidate implementation
> **Objective:** Determine whether explicit non-target negatives can produce a
> broad parent-genre preview that remains precise on collection-wide music,
> rather than only separating a closed set of supported genres.

## Why this plan exists

Plan 070 independently rejected the exact four-output CLAP/PCA64/ridge preview.
It offered on 31 of 60 sealed rows but matched only 16 primary human parents
(51.61%). Ambient reached 4/5, House reached 10/23, and the high/medium-only
sensitivity remained only 15/27. The failure is valid, replayed byte-for-byte,
and permanently consumes that holdout.

The failed model trained on only 575 rows from seven supported parents. The
same frozen development corpus contains another 141 verified rows spanning
eleven parent genres, but those rows were excluded from training and
calibration. That made the experiment a closed-set classifier followed by a
confidence threshold, while the independent holdout exercised the real
open-set product problem.

This plan tests that structural hypothesis using development evidence only. It
does not inspect track-level Plan 070 errors, retune the retired thresholds,
remove a target because of its holdout result, or reuse the consumed holdout as
training or model-selection data.

## Frozen evidence boundary

Use the existing mode-0600 development corpus exactly:

- corpus SHA-256:
  `0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1`;
- accepted-corpus fingerprint:
  `07a754c42ae676eb7f6fcbc02ee1b5748e3153e155311d4559d3d749fbdd6cf1`;
- 716 accepted rows across 18 canonical parent genres;
- 575 rows across the seven already-supported positive targets; and
- 141 rows across eleven other parents, all retained as explicit rejection
  negatives.

Do not append the 31 newly reviewed Plan 070 offers before evaluating either
candidate. Exclude all 60 rows in the consumed Plan 066/070 roster from this
development cycle and from the new independent holdout. The consumed artifacts
remain historical evidence only:

- recovered roster SHA-256:
  `1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37`;
- Plan 070 evaluation SHA-256:
  `6d5a4b1e1df7e2063fdd927fa2531d257b2badd95bedf062f31bf39602b9a026`.

The seven possible output parents stay frozen in the existing order:

1. House
2. Ambient
3. Techno
4. Breakbeat
5. Reggae
6. Electro
7. Trance

Every other verified parent is a negative for all seven outputs. `Experimental`
remains unmodeled. A prediction of a supported parent for a row whose truth is
Tech House, Disco, Drum & Bass, R&B, Minimal, or any other non-target parent is
an error, not a near match.

## Fresh independent holdout

Before fitting either candidate, seal a new 60-row identity-only roster using a
new fixed seed. Selection may use current Rekordbox genre only as a hidden
sampling stratum; it is never truth. Exclude:

- every accepted development path, normalized artist, and release group;
- every path, normalized artist, and release group in the consumed 60-row
  holdout;
- every track in `genre_verified` or an earlier genre research/review playlist;
- missing files, blank artists, duplicate artists, duplicate release groups,
  unmapped genres, and `Experimental`.

Use these desired hidden-stratum quotas, chosen from the pre-plan eligibility
audit rather than model output:

| Sampling stratum | Desired rows |
| ---------------- | -----------: |
| Ambient          |            5 |
| Breakbeat        |           10 |
| Downtempo        |            1 |
| Drum & Bass      |            1 |
| Electro          |            1 |
| Hip Hop          |            1 |
| House            |           12 |
| IDM              |            1 |
| Minimal          |            2 |
| Pop              |            2 |
| R&B              |            8 |
| Reggae           |            3 |
| Techno           |           12 |
| Trance           |            1 |

Select scarce strata first, then fill any quota shortfall from the remaining
eligible rows in fixed-seed order, with no more than 15 rows from one sampling
stratum. Stop if the deterministic selector cannot produce 60 unique artists
and release groups. Write only aggregate counts to logs; keep identities and
sampling fields private at mode 0600.

The roster is sealed before fitting or scoring a candidate. No holdout feature,
embedding, prediction, margin, or offer may be produced until one candidate
passes every development gate below.

## Label-blind inputs

Prepare two aligned 716-row private manifests:

- truth, normalized artist, normalized release group, and a deterministic
  five-fold assignment; and
- row ID plus file path only for feature extraction.

An artist and every release by that artist must remain in one fold. Assign folds
deterministically to balance the seven positive targets and the combined
non-target population. Each fold must contain positives for every output and at
least twenty non-target rows.

Extract features without reading truth:

- the unchanged 140 cache-native/v0.33 features from Stratum 21 and Essentia 3;
- the unchanged locally pinned 512-value CLAP representation using three evenly
  spaced ten-second excerpts; and
- training-partition-only PCA64 for CLAP during every fold.

Use the current and audit baseline manifests with SHA-256 values
`66952c90940801d3d9e1c1004d03ea584c1fb190214ec3854bae37c28d97faca`
and
`d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e`.
Re-extract all 716 CLAP rows into a new resumable work directory so one artifact
has one manifest, order, decoder contract, and source-hash chain. Replay cache
and representation extraction byte-for-byte before evaluation.

## Two preregistered candidates

No third formulation or hyperparameter search is allowed in this plan.

### O1: pooled-negative multiclass ridge

- Classes are the seven output parents plus one heterogeneous `Other` class.
- Every non-target parent maps to `Other` for fitting only; its canonical truth
  remains intact for scoring.
- Use class-balanced one-versus-rest ridge least squares, penalty 10, an
  unpenalized intercept, cache-native features, and training-only CLAP PCA64.
- `Other` can win internally but is never emitted.
- For each output parent, choose a separate threshold from inner out-of-fold
  rows predicted as that parent. The threshold must reach at least 90% exact
  precision with at least eight offers.
- An outer row is offered only when its winning class is an output parent and
  its top-minus-second score margin clears that parent's inner threshold.

### O2: independent binary ridge with collision abstention

- Fit seven independent class-balanced binary ridge models with the same
  penalty, intercept, base features, and training-only CLAP PCA64.
- Each target's positives are that parent; all other 715-row truths are
  negatives as applicable.
- Choose each target threshold from inner out-of-fold binary scores at at least
  90% exact precision and at least eight offers.
- Offer only when exactly one target clears its threshold. Zero qualifiers and
  multi-target collisions both abstain; scores are never compared across binary
  models.

## Development evaluation

Use five outer artist-isolated folds. For every outer fold, fit models on four
folds and choose thresholds only from nested predictions within those four
folds. The outer fold remains untouched until its model and thresholds are
fixed. Concatenate the five outer results once.

Score exact canonical parents across all 716 rows. Non-target rows are part of
the denominator whenever a candidate offers. Report:

- offers, abstentions, coverage, exact offered precision, and accuracy;
- precision and recall per suggested output;
- per-fold offers, coverage, and precision;
- non-target support, false offers, and false-offer rate;
- number of output parents with at least eight offers;
- zero-, one-, and multi-qualified counts for O2;
- the current v0.33 result on the same rows; and
- paired precision improvement wherever both systems offer.

A candidate is development-eligible only if its nested outer-fold result meets
all of these frozen gates:

- at least 180 offers and at least 25% coverage across all 716 rows;
- at least 90% aggregate exact offered precision;
- no more than a 10% false-offer rate across the 141 non-target rows;
- every outer fold has at least twenty offers and at least 85% precision;
- every output with at least eight offers has at least 80% precision;
- at least four output parents have at least eight offers; and
- paired precision improves on v0.33 by at least five percentage points.

Calibrate deployment thresholds from all frozen outer out-of-fold predictions
only after nested evaluation. A deployment output parent must have at least
eight calibration offers at 90% precision. At least four parents must remain
deployable. These same thresholds are serialized for any holdout inference;
they are not development evidence beyond the nested result.

Replay every candidate result byte-for-byte. If neither candidate passes, stop
with a bounded negative and do not extract or infer the fresh holdout. If one
passes, select it. If both pass, choose higher nested aggregate precision, then
higher coverage, then O1 as the fixed final tie-break.

## Independent release gate

After selecting and serializing one development-eligible candidate:

1. Freeze its exact source, input hashes, feature schema, model formulation,
   output parents, and thresholds.
2. Prepare fresh holdout feature manifests without identity or sampling values.
3. Verify zero path, artist, release-group, prior-review, and decoded-audio
   overlap with development and the consumed holdout.
4. Extract and replay cache-native and CLAP features label-blind.
5. Fit the full development model and freeze all 60 predictions before any
   listening.
6. Export only offered tracks in prediction-blind batches of at most six.
7. Freeze every human verdict before the first prediction join.

The candidate passes only if:

- at least 30 of 60 rows receive offers;
- aggregate exact-primary-parent precision is at least 90%;
- every emitted parent with at least five offers reaches at least 80%
  precision;
- the isolation audit passes; and
- inference and evaluation replay byte-for-byte.

All offers remain in the denominator. Ambiguous and skipped rows are incorrect;
alternatives never receive credit; confidence does not change the primary gate.
Report a separate high/medium-confidence sensitivity view only.

## Product and active-learning boundary

An independent pass authorizes a preview implementation, not automatic
metadata writes. The product must:

- advertise only the parents that survive development calibration and the
  independent gate;
- expose confidence and explicit abstention reasons;
- keep CLAP optional with its download, licence, storage, latency, offline, and
  removal behavior documented;
- stage accepted genres only through `ChangeManager` and manual XML import;
- default active-learning review to six tracks with an explicit maximum of
  twenty; and
- append only high/medium blind labels to a versioned truth ledger after they
  are no longer part of an active holdout.

## Stop condition

This plan ends with one of three honest outcomes:

1. both open-set candidates fail development and are retired;
2. one candidate passes development but fails the fresh independent holdout and
   is retired; or
3. one candidate passes both stages and the preview plus active-learning loop
   is implemented and verified.

Do not change the taxonomy, candidate formulation, feature set, ridge penalty,
folds, thresholds, gates, holdout roster, or scoring policy after observing a
result.
