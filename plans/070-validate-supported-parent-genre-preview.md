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

## Sealed inference result

The full-development PCA64 and seven-way ridge model produced 31 offers and 29
abstentions. This clears the frozen minimum of 30 reviewable offers without
revealing a per-track prediction, margin, supported parent, current genre, or
sampling stratum. The independently serialized model matched the existing
reference evaluator at every prediction and margin.

Inference replay reproduced both artifacts byte-for-byte:

- private fitted-model SHA-256:
  `86df1955b89212cbef0269e68c18e588cfc0694d3dd34799be8625a0678bddb3`;
- private sealed-prediction SHA-256:
  `4e95f5cb364d2eb4966b5a3d0d1cbcdc2a30843f2b91475a86d4f350a40847b5`;
- holdout offers: 31 of 60;
- holdout abstentions: 29 of 60;
- all predictions frozen before listening: yes;
- identity values exposed by inference: no; and
- private artifact mode: 0600.

The inference roster is now prediction-open but verdict-unopened. Review
materials must be derived from the exact 31 offered rows and remain blind to
every model and sampling field.

## Blind holdout review preparation

All 31 offered rows were assigned to one deterministic prediction-blind order
and partitioned before listening into batches of 6, 6, 6, 6, 6, and 1. The six
private mappings contain identity and opaque source-row IDs only; prediction,
margin, threshold, supported parent, current genre, and sampling fields are
absent. The preparation replay reproduced the manifest SHA-256
`bd06892fd97cef7b904676edd069d56546019ba742992c033ff277c9e0f81838`.

H01 through H04 have been completed without joining their verdicts to
predictions. H05 is the only newly exported pending batch. Every export contains
six live identity-verified tracks, a review sheet with exactly six initially
blank verdict rows, and a guide with no hidden field. Exports left zero staged
Reklawdbox changes.

H01 export record:

- pre-export private mapping SHA-256:
  `9f005e203e71b495524955c07173c48d777e711fc0c41e9848f695a549945560`;
- provenance-enriched private mapping SHA-256:
  `63bbd28d12892c7f69d20df835003fac5c91b5ca4e1d77de831fffa9a99ae304`;
- XML SHA-256:
  `cb9596b259b861b89ccd849eb16d5f0ece62341724092c93fceec5d4f893d516`;
- review TSV SHA-256:
  `268d542fc888e688d9e627fe876277c1fc172d408e87ea5cd21cad4fdca6d6ce`;
- guide SHA-256:
  `379eb3df385415a6bcabd2a132849036609b06fd2dcc327b4e06df5a96e4cc59`;
  and
- all export and mapping artifacts: mode 0600.

H01 review is sealed without joining the verdicts to predictions. The operator
provided five labels and one genuinely ambiguous verdict. Mixed medium-high
confidence was normalized conservatively to medium, while all raw wording and
listening notes remain intact. Verdict conversion replayed byte-identically.

H01 verdict record:

- completed review TSV SHA-256:
  `2e1bbb09ac5716b5a19b37477f3d24fdfd3a5831bab4524e1bc42d73775feced`;
- private verdict artifact SHA-256:
  `9cbe4c0e342172be34cb8fad15755da73c3d0e9c6f3f34202da3d15823ee7855`;
- outcomes: five labels and one ambiguous;
- prediction fields read during conversion: no; and
- both review and verdict artifacts: mode 0600.

H02 export record:

- pre-export private mapping SHA-256:
  `a19555dcc30030e19d8b3d4c0bbb4ee8da9a3cb5707c609b892c2e7a765bf566`;
- provenance-enriched private mapping SHA-256:
  `02c1fbc99522ce75dfb0d1a7dc40f69538687ff76db4cc104ad173905db39a55`;
- XML SHA-256:
  `96094b10bd6c53ab03221aaa3ad74318b87b36c14a65033b64b94b7caeb0a51b`;
- review TSV SHA-256:
  `3ea34b02a27e30bad493e9e11c841533ba07d21595d68daedc737eefecd54f32`;
- guide SHA-256:
  `7dfa3e4522cb68dd08d38043348f29dc5b9ada382bb699d352910db408e64587`;
  and
- all export and mapping artifacts: mode 0600.

H02 review is sealed without joining the verdicts to predictions. All six rows
received a single broad-parent label. The operator's Gabber wording and tempo
observation remain preserved as notes while the canonical verdict is Hardcore.
Verdict conversion replayed byte-identically.

H02 verdict record:

- completed review TSV SHA-256:
  `e3d3275e0e95a3498c8f9ece5667f6874b5372d5db969930cf3979c34313860a`;
- private verdict artifact SHA-256:
  `632b68d52120256e4af7d8d38761aed38cdf96e404837f62028c2c58d3add0e6`;
- outcomes: six labels;
- prediction fields read during conversion: no; and
- review, verdict, and replay artifacts: mode 0600.

H03 export record:

- pre-export private mapping SHA-256:
  `bcffc2ad3d249209da90ac2c483d1b70714c7d262c7e5407dde6973b5331c530`;
- provenance-enriched private mapping SHA-256:
  `37422721a106405aca9fe835016778a9ec273d9fc93e29623c32d5e5487a43c3`;
- XML SHA-256:
  `5e4d728a4b1772eff8c189ca98d1857287a0be38e9909f7623a3666a94525c56`;
- review TSV SHA-256:
  `c2176331c0e7e2f11d46fb10bee79e250cd417aa4f03180c3846c3044ba30050`;
- guide SHA-256:
  `05fe48c9184f0ec25b8e01efd5cca560e5cc65a6a8f2ae425db561c5066fa4ef`;
  and
- all export and mapping artifacts: mode 0600.

H03 review is sealed without joining the verdicts to predictions. All six rows
received a single broad-parent label. The operator's raw Dub wording maps to
the frozen Reggae parent, and the suspected Rekordbox BPM error remains a note
rather than a metadata change. Verdict conversion replayed byte-identically.

H03 verdict record:

- completed review TSV SHA-256:
  `ba9caba12d41dd76ef6638c854cb065aef79bf181847d277aa54d908e6ae8857`;
- private verdict artifact SHA-256:
  `afa7ba31ec428ddf5ed3c864c3d86bec3768f7a232248606eb37e183083d0c41`;
- outcomes: six labels;
- prediction fields read during conversion: no; and
- review, verdict, and replay artifacts: mode 0600.

H04 export record:

- pre-export private mapping SHA-256:
  `236710aacf0f76e480c30122af11b5e14ffa1983625156920618957de78c252a`;
- provenance-enriched private mapping SHA-256:
  `4c6e19539db98fcf2d63555c47782043203fef0e453b686fe565571cdde47b3f`;
- XML SHA-256:
  `4f8f4c8cecde13b5aef259a9c831885419316f86b2a5ae86558e1659a1520d0d`;
- review TSV SHA-256:
  `c3e326ee336f939e492fe1148a379dfb78238e6951fd8dd29487d902af4a68dd`;
- guide SHA-256:
  `c0075059eeec91d108c5eacb2dfcd41bec770d64bc8942ebeaa820c58372db88`;
  and
- all export and mapping artifacts: mode 0600.

H04 review is sealed without joining the verdicts to predictions. All six rows
received a single broad-parent label. The operator's Deep House description
remains a note under the frozen House parent, and the low-confidence House
verdict remains low confidence. Verdict conversion replayed byte-identically.

H04 verdict record:

- completed review TSV SHA-256:
  `840c26cd3d28f24a6f311f133b421f17439542e0ebea161070fbf97506360fe7`;
- private verdict artifact SHA-256:
  `06de49370375aa75068a970aefd36568dbe2e11587467712494d22264a28519b`;
- outcomes: six labels;
- prediction fields read during conversion: no; and
- review, verdict, and replay artifacts: mode 0600.

H05 is the only newly exported pending batch. It contains six live identity-
verified tracks, six blank verdict rows, no hidden model or sampling field, and
left zero staged Reklawdbox changes.

H05 export record:

- pre-export private mapping SHA-256:
  `903a95c1eee4bcc6943438be4b5c37c85d462ac18b7f8841f467c5eb01fa83c9`;
- provenance-enriched private mapping SHA-256:
  `dffb7558175836ebb05683b8b3034f40d78358f2b5f75cd4384c94eb75f92135`;
- XML SHA-256:
  `de2e426b3dc489a84f625327019340c1938d304b23f28985daff8d036ade4ed2`;
- review TSV SHA-256:
  `69da9776eb00eff0fe8179afe68c105222fe64dc2cb4876e23e89da3b6f01c93`;
- guide SHA-256:
  `8a8270e0339ab699ae3996e7cc0aedfd11c4c3254a60d115cce54631553bad2b`;
  and
- all export and mapping artifacts: mode 0600.

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

Freeze scoring before joining any verdict to a prediction. Every one of the 31
offers remains in the aggregate denominator. A row is correct only when its
single primary canonical parent verdict exactly matches the frozen suggested
parent. Confidence is preserved but does not change that rule. Ambiguous or
skipped rows are not credited, and plausible alternatives never count as an
exact match. This deliberately treats an offered but genuinely ambiguous track
as a product error rather than rescuing it after listening. Report a separate
high/medium-confidence sensitivity view, but never substitute it for the frozen
all-offer release gate.

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
