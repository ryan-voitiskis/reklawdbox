# Plan 070: Validate a supported-parent Genre Intelligence preview

> **Status:** In progress; product scope frozen before holdout inference
> **Objective:** Determine whether the strongest stable subset of the frozen
> Genre Intelligence classifier is independently accurate and useful enough for
> a preview-first release without representing unsupported parent genres as
> solved.

## Why this is a distinct candidate

Plans 068 and 069 correctly rejected seven-parent release candidates. The
target-aware CLAP result nevertheless established a useful product boundary:
Ambient, House, Reggae, and Techno remained precise under honest nested
calibration, while Breakbeat and Trance could not receive stable offers and
Electro fell below its per-parent gate. The seven-parent candidate did not pass
and must never be described as having passed.

This plan treats the four stable parents as the complete output contract of a
new, deliberately partial preview. It does not retune the classifier, lower a
quality floor, reinterpret an error, or claim new development evidence. The
choice of supported parents is informed by exposed development results, so the
already-sealed Plan 066 holdout is the sole independent release evidence.

## Frozen model and output contract

Reuse candidate C exactly:

- the 716 accepted and 575 model-ready development rows;
- the seven-way internal label order: House, Ambient, Techno, Breakbeat,
  Reggae, Electro, Trance;
- candidate A's 140 cache-native and v0.33 features;
- the pinned 512-value CLAP representation;
- full-development PCA64;
- class-balanced one-versus-rest ridge with penalty 10 and an unpenalized
  intercept; and
- top-score-minus-second-score confidence.

The only product-supported output parents are Ambient, House, Reggae, and
Techno.

An internal Breakbeat, Electro, or Trance prediction always abstains regardless
of margin. It is not redirected to another parent. An internal supported-parent
prediction below that parent's threshold also abstains.

### Pre-inference leakage amendment

The first identity-only input audit stopped before writing holdout features and
found one normalized artist group shared by the old sealed roster and the newer
Plan 068 development corpus. This was possible because truth expansion occurred
after the roster was sealed. No holdout embedding, prediction, margin, or genre
outcome existed when the overlap was found.

To preserve an artist-isolated independent test, exclude every development row
whose normalized artist or release group occurs in the sealed holdout. The
exclusion is deterministic from private identity fields, reports only aggregate
counts, and is frozen before scoring. Re-run the unchanged target-aware nested
calibration on the remaining development rows. All four supported parents must
receive a deployment threshold under the existing minimum-eight-offer and
90%-precision calibration rule; otherwise stop without holdout inference.

The resulting four thresholds replace the initial Plan 069 deployment values,
which were invalidated before holdout inference because their calibration pool
contained the overlapping artist. The supported parent set, seven-way internal
model, features, PCA64, ridge penalty, confidence, and calibration rule do not
change. Freeze the reduced-development manifest, thresholds, evaluator source,
and every input hash before the first holdout model inference.

## Isolation and calibration freeze

The checksum-bound input preparation excluded exactly one development row for
the one overlapping artist group. Exact path, release-group, and accepted-truth
path overlap were all zero; all 60 audio files exist. Preparation replayed
byte-identically without exposing an identity value.

Pre-score record:

- holdout preparation implementation commit: `597cdd0`;
- supported-preview evaluator implementation commit: `50d4868`;
- evaluator source SHA-256:
  `d38467ae505d1f572ef4e1fd2d6b75ab674023139f512d7c9d9ee727aae4617f`;
- private supported-preview config SHA-256:
  `24005487e745d1a02f7f840697f91d4511bf78ff0294f492d00df0d9345312d7`;
- private development-exclusion artifact SHA-256:
  `575b95fceada7565e3297a0420896bb32af83479a1bd5ff69c2b3b814e0c6c32`;
- private holdout feature-manifest SHA-256:
  `d970b54a4da4c370e9a5a21524393c4845c6b70ed5a09276540f09a4dbb0c152`;
- private holdout representation-manifest SHA-256:
  `94e513d7d9400d31eb5562dfd1a910c8606ad92455e15f41a9a5c5966ae7fc0f`;
- private holdout-input summary SHA-256:
  `8423585a07fe00d4c6127d912931d6321ca5337dde2c18f53edb691f8d93d2a5`;
- focused preparation and calibration suite: ten tests passed; and
- holdout feature, embedding, or prediction produced at freeze: no.

## Isolation-corrected calibration result

The frozen calibration passed and replayed byte-identically. The nested view
offered 346 of 574 retained development rows at 92.20% precision and 60.28%
coverage. Every fold cleared 89%, each supported parent cleared its 80% floor,
and paired precision improved over v0.33 by 21.48 percentage points. The
deployment view offered 375 rows at 91.73% precision and 65.33% coverage; all
four supported parents received a threshold and every frozen check passed.

The exact full-fit thresholds are now:

| Parent  | Frozen margin threshold |
| ------- | ----------------------: |
| Ambient |    0.025198863598072052 |
| House   |  0.00022861019473163768 |
| Reggae  |     0.28868385871208535 |
| Techno  |      0.4383864429068181 |

Result SHA-256 is
`6bf4b95790b2edd41fed5f3752c2093ef249a2762cfbdbb8b1e27abd2c76e31c`.
Both result copies are mode 0600. The holdout remained feature-, embedding-,
and prediction-free throughout calibration.

## Pre-inference freeze

The 60-row cache-native and CLAP feature passes completed without a missing
value, skip, or retry and each replayed byte-identically. The decoded-audio
audit covered all 575 source-development tracks, including the subsequently
excluded row, and all 60 holdout tracks. Every decoded stream was unique within
its partition and cross-partition overlap was zero.

Inference freeze record:

- inference implementation commit: `43f36ed`;
- inference source SHA-256:
  `f74809e595cf93e74f7e97673e7c508c06bb1a38e8476950935dbc8ca25c89ea`;
- private inference config SHA-256:
  `dab08a7c83df310bfaa83035ffa7d0ff417c9e708ef781ad7bd1ab2694a2d51d`;
- cache-native feature artifact SHA-256:
  `c92c3d90e5e5b8358c6397a09f4d49ddb3f2054b6c05f20df3a9a6d6f9cd603a`;
- cache-native feature summary SHA-256:
  `771f8c38d5ba7264b98111b52b8cede6ee58040492a5e4dfa33c3ed1efd62f8a`;
- CLAP feature artifact SHA-256:
  `809b73934f942d8ff3d6402f61ae09e9f58eb317a0eeeeaea3dff61ae3378aab`;
- CLAP feature summary SHA-256:
  `edfbc85335c63c3017fe185d720a5984a6d501df8e429e78686b5ab751d5051a`;
- decoded-audio isolation artifact SHA-256:
  `0e989353d7ae0cf7ca0978557225935753ff5c595f9af38722981eb3dcc903c6`;
- CLAP weights SHA-256:
  `1cd3c601bc4afe0fa87be3de4c13dd2cfadd249fac1e29acf74a9b296c3219bb`;
- private artifact mode: 0600; and
- genre model, prediction, margin, or offer produced at freeze: no.

For context only, the four supported targets made 349 nested offers on exposed
development data. Their aggregate offered precision was 92.26%, with per-target
precision of 92.31% Ambient, 92.98% House, 87.50% Reggae, and 90.63% Techno.
This descriptive slice is not a new development gate and cannot justify release
without the independent holdout.

## Holdout boundary

Use exactly the unopened 60-row Plan 066 roster sealed before any OpenL3 or CLAP
inference:

- artifact SHA-256:
  `532ce77378154949f2f02e5283c9c12ec528639f3be912aa2ddba2ba71b35589`;
- checksum-verified recovered artifact SHA-256:
  `1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37`;
- roster SHA-256:
  `e90b400645d89b287aab4300465fd0893314830bc6ec8b6ab22b5f9de4fbfdf9`;
  and
- 60 unique normalized artists and artist-release groups.

Prepare cache-native and CLAP inputs without printing identities, current genre,
sampling stratum, predictions, margins, or supported-output status. Verify exact
path joins, decoded-audio hashes, development artist/release exclusion, and
mode-0600 private artifacts. Fit the full model and seal all 60 internal
predictions, margins, offers, and abstentions before any listening.

Replay feature extraction, fitting, and inference byte-identically. Committed
source and a private config must pin every input, model byte, supported parent,
threshold, and output hash before producing a review export.

## Blind review and release gate

Export only offered rows for listening in batches of at most six. The operator
may see identity and hear audio, but must not see current genre, sampling
stratum, predicted parent, margin, threshold, feature, neighbour, or model hint.
Record human verdict and confidence through the existing strict ledger path.

The exact candidate passes only if:

- at least 30 of the 60 holdout rows receive offers;
- aggregate offered precision is at least 90%;
- every supported parent with at least five offers reaches at least 80%
  precision;
- no development, artist, release, decoded-audio, or prior-review leakage is
  found; and
- the complete inference and aggregate evaluation replay byte-identically.

One failure retires the exact model, supported-output set, and thresholds. Do
not change a target, threshold, verdict, or holdout row after seeing results.
The holdout roster cannot be replaced or supplemented.

## Product work after a pass

A holdout pass authorizes implementation, not silent metadata mutation. The
first release surface must:

- call the feature an experimental broad-genre preview;
- list exactly four supported outputs and explain that all other cases abstain;
- keep the 614.5 MB CLAP model optional with explicit download, licence,
  attribution, storage, latency, CPU/MPS, offline, and removal behaviour;
- version the taxonomy, model, thresholds, feature schema, and cache;
- expose preview through CLI and MCP with confidence and abstention reasons;
- stage accepted metadata only through `ChangeManager` and manual XML import;
- preserve six-row default active review with an explicit limit capped at
  twenty; and
- document measured holdout evidence and limitations without extrapolating to
  unsupported parents.

## Stop condition

This plan ends when the exact candidate either fails independent review and is
retired, or passes and the preview-first product workflow is implemented and
verified. No additional model or calibration candidate begins from holdout
errors.
