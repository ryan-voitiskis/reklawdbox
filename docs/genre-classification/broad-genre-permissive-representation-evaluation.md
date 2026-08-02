# Permissive Broad-Genre Representation Evaluation

**Run date:** 2026-08-02

**Status:** Development gate failed; sealed holdout remains unopened

## Question and boundary

Plan 066 asked whether one production-plausible music representation could
turn the useful Plan 065 kick evidence into a release-grade, selective broad
genre candidate. It evaluated exactly two preregistered additions: OpenL3 music
embeddings and LAION CLAP audio embeddings. Each candidate used its own
training-partition PCA64 projection; the representations were never combined.

The 60-track future holdout was sealed before either representation was
inferred. Current Rekordbox genre was not a classifier input. Audio, Rekordbox,
tags, caches, staged metadata, and XML were not changed. Track identities,
paths, features, predictions, margins, and thresholds remain private.

## Input-contract preflight

The available-audio roster contains 668 ordered rows from the prior 670-row
development corpus. An initial evaluator preflight stopped before loading a
representation because the current manifest had 86 regenerated fold
assignments and two changed baseline recommendations. All 668 truth labels
matched.

No candidate score was produced by that attempt. Before scoring, the evaluator
was corrected and recommitted to preserve the exact Plan 065 fold assignments
and baseline recommendations joined by path. This kept the experiment focused
on the new representation.

## Development results

Nested thresholds were calibrated entirely within each outer training
partition. These are the primary development results.

| Candidate | Unselective accuracy | Offers | Coverage | Offered precision | Worst fold precision | Result |
|---|---:|---:|---:|---:|---:|---|
| CLAP | 74.40% | 403 | 60.33% | 91.56% | 86.84% | Fail: supported targets |
| OpenL3 | 73.50% | 398 | 59.58% | 87.94% | 83.54% | Fail: precision, fold, supported targets |

CLAP was the strongest candidate. Relative to the 670-row Plan 065 diagnostic,
it improved selective coverage by 6.30 percentage points, selective precision
by 0.96 points, and unselective accuracy by 1.27 points. The comparison is
diagnostic because two unavailable rows are absent here. More importantly,
all five CLAP outer folds cleared the 85% stability floor.

| CLAP fold | Eligible | Offers | Coverage | Offered precision |
|---:|---:|---:|---:|---:|
| 0 | 138 | 86 | 62.32% | 95.35% |
| 1 | 132 | 82 | 62.12% | 87.80% |
| 2 | 130 | 73 | 56.15% | 93.15% |
| 3 | 130 | 76 | 58.46% | 86.84% |
| 4 | 138 | 86 | 62.32% | 94.19% |

## Supported-target guard

The gate requires at least 75% precision for every target with at least ten
truth rows and five offers. CLAP failed that guard despite passing every
aggregate and fold check.

| Candidate and threshold | Supported-target failures |
|---|---|
| CLAP, nested | Breakbeat 5/7 (71.43%); Trance 8/11 (72.73%); IDM 1/7 (14.29%); Minimal 0/5 (0%) |
| CLAP, global deployment | Garage 5/7 (71.43%); Breakbeat 5/7 (71.43%); Trance 11/15 (73.33%); IDM 1/7 (14.29%); Minimal 1/7 (14.29%) |
| OpenL3, nested | Garage 6/9 (66.67%); Breakbeat 3/6 (50%); IDM 1/6 (16.67%); Minimal 1/5 (20%); Tech House 6/12 (50%) |
| OpenL3, global deployment | Garage 5/7 (71.43%); Breakbeat 3/5 (60%); Tech House 6/11 (54.55%) |

CLAP's nested candidate was strong on several well-supported targets: Ambient
81/81, House 144/151, Electro 22/24, Reggae 19/19, and Techno 60/65. Tech House
also passed its guard at 5/6. This is useful evidence that the representation
improves the dominant broad roots. It does not justify hiding severe failures
in smaller roots behind a strong aggregate.

## Frozen deployment threshold

The single global thresholds were selected from outer out-of-fold margins so a
future full-fit model would have a fixed abstention rule. They are deployment
calibration diagnostics, not an independent validation set.

| Candidate | Offers | Coverage | Offered precision | Worst fold precision | Result |
|---|---:|---:|---:|---:|---|
| CLAP | 445 | 66.62% | 90.11% | 85.19% | Fail: supported targets |
| OpenL3 | 382 | 57.19% | 90.05% | 86.30% | Fail: supported targets |

Neither candidate passed both the nested and deployment gates, so no full-fit
model was created and no holdout prediction was inferred.

## Decision

This is a bounded negative, not evidence that broad classification is
unpromising. CLAP materially improved aggregate performance and fold stability,
but the 10–20-row roots are too unreliable for the proposed all-target release
contract. The result most strongly supports a data-first next step: design and
freeze a larger, artist- and release-isolated truth corpus for the sparse and
unstable broad roots before fitting another candidate.

Do not tune per-target thresholds, target mappings, PCA size, ridge penalty, or
representation combinations on these exposed rows. Preserve the sealed
60-track holdout for a future candidate that first passes a separately frozen
development gate.

## Reproducibility

- rows: 668
- broad targets in the frozen taxonomy: 26
- evaluator source SHA-256:
  `4bb3e1c21bc5a7a95fe172b92657f319fdf4fff1930b49890b16c2ede67ed770`
- OpenL3 feature SHA-256:
  `d9c06b2df65199d98e17277a268e69732e41c7e7b76d6f9e2c82824461b8097c`
- CLAP feature SHA-256:
  `097443ac6ec6f0195ce8904643ec74703b3a81c50ed9d0610213b7674970d59a`
- private aggregate result SHA-256:
  `265df46ff10873355f933b0347229b8c2c282de0f3745c07be08e6599fc433dd`
- result replay: byte-identical
- selected candidate: none
