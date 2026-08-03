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

The only product-supported output parents are:

| Parent  | Frozen margin threshold |
| ------- | ----------------------: |
| Ambient |    0.025198863598072052 |
| House   |    0.003815435334147077 |
| Reggae  |     0.29864359521448636 |
| Techno  |      0.4383864429068181 |

An internal Breakbeat, Electro, or Trance prediction always abstains regardless
of margin. It is not redirected to another parent. An internal supported-parent
prediction below that parent's threshold also abstains. Thresholds are the
unchanged Plan 069 deployment thresholds selected from complete outer
out-of-fold development predictions.

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
