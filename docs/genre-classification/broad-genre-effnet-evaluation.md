# Broad Genre EffNet Evaluation

**Run date:** 2026-08-02

**Status:** Development gate failed; direct style confidence retired unchanged

## Question and boundary

Plan 063 tested whether the already-frozen Discogs-EffNet style scores could
produce release-grade broad suggestions under the unchanged Plan 062 mapping.
The aggregation, confidence rule, cross-fold threshold procedure, and gates
were committed before broad scores were inspected.

No audio was decoded and no inference was rerun. The evaluation reused the
sealed 670-row Plan 059 feature artifact. It did not use embeddings,
arrangement features, v0.33 predictions, current Rekordbox genre, or stored
profiles. It wrote no Rekordbox metadata, cache row, audio, tag, staged change,
or XML.

## Method

Each broad score was the maximum direct style score among its mapped fine
genres. Confidence was the margin between the largest and second-largest broad
scores.

For each held-out fold, the other four folds selected the maximum-coverage
margin threshold that retained at least 90% training precision and a minimum
of 40 or 10% of training rows, whichever was larger. The selected threshold
was applied once to the held-out fold. There were no target-specific
thresholds, priors, or exceptions.

## Aggregate results

| Configuration | Offers | Coverage | Offered precision | Accuracy | Macro recall | Macro F1 |
|---|---:|---:|---:|---:|---:|---:|
| Unselective direct style projection | 670 | 100.00% | 71.04% | 71.04% | 53.29% | 51.42% |
| Cross-fitted margin selection | 199 | 29.70% | 88.94% | 26.42% | 21.41% | 27.15% |

The representation is materially better at broad labels than the v0.33
projection, which reached 48.65% unselective precision in Plan 062. Its raw
confidence margin nevertheless could not provide both release-grade precision
and useful coverage.

## Fold stability

| Fold | Threshold | Training coverage | Held-out coverage | Held-out precision |
|---:|---:|---:|---:|---:|
| 0 | 0.364996 | 22.56% | 30.43% | 90.48% |
| 1 | 0.273245 | 40.89% | 34.85% | 84.78% |
| 2 | 0.296964 | 36.18% | 32.82% | 88.37% |
| 3 | 0.374663 | 24.12% | 18.32% | 95.83% |
| 4 | 0.320161 | 32.71% | 31.88% | 88.64% |

The threshold varied substantially across folds. Fold 1 missed the frozen 85%
precision floor; three additional folds remained below the overall 90% target.

## Target structure

Selective Ambient and Reggae offers were 100% precise in this development run.
House reached 83.72% precision on 43 offers, and Techno reached 81.94% on 72
offers. Breakbeat, Garage, IDM, and Tech House received no correct selective
offers. The aggregate 88.94% therefore reflects uneven target utility rather
than broad, stable coverage.

## Frozen gate

| Check | Required | Observed | Result |
|---|---:|---:|---|
| Offered precision | 90% | 88.94% | Fail |
| Coverage | 50% | 29.70% | Fail |
| Every-fold precision | 85% | 84.78% minimum | Fail |
| Supported-target precision | 75% | No qualifying failure | Pass |
| Precision improvement over unselective | +10 pp | +17.90 pp | Pass |

## Decision

Direct EffNet style confidence is a bounded negative and does not advance to a
holdout. Its mapping and margin thresholds receive no post-result tuning.

The broad representation is sufficiently stronger than v0.33 to justify one
supervised broad adapter using the already-fixed Plan 058 ridge and
training-fold PCA machinery. That candidate must use nested fold-local
confidence calibration, because selecting a threshold from in-sample training
scores would leak fit quality into the confidence estimate.

## Reproducibility

- private aggregate result SHA-256:
  `c89d5f8db82d6926ef919c1497693ee9f72614355efd3c5893e422844fd08e79`
- private manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- private feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
