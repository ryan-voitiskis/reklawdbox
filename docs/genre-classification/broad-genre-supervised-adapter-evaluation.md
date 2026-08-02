# Supervised Broad Genre Adapter Evaluation

**Run date:** 2026-08-02

**Status:** Development gate failed; adapter retired unchanged

## Question and boundary

Plan 064 tested one fixed class-balanced ridge adapter over the unchanged broad
taxonomy. It combined broad EffNet style scores, the mapped v0.33 suggestion,
four arrangement descriptors, and a training-partition PCA64 projection of the
EffNet embedding.

The complete feature and fitting recipe was committed before the result was
inspected. Confidence was calibrated with nested out-of-fold predictions
inside each outer training partition. Current Rekordbox genre, stored profiles,
and private identity were not inputs. No audio was decoded, and no Rekordbox
metadata, cache row, tag, staged change, or XML was written.

## Aggregate results

| Configuration | Offers | Coverage | Offered precision | Accuracy | Macro recall | Macro F1 |
|---|---:|---:|---:|---:|---:|---:|
| Unselective supervised adapter | 670 | 100.00% | 72.09% | 72.09% | 59.41% | 53.68% |
| Nested selective adapter | 334 | 49.85% | 87.72% | 43.73% | 37.62% | 45.28% |

The adapter improved unselective broad accuracy by 1.04 percentage points over
the direct style projection. Relative to direct margin selection, the nested
adapter added 20.15 percentage points of coverage but lost 1.22 percentage
points of offered precision. It did not reach release-grade precision.

## Fold stability

| Fold | Threshold | Inner coverage | Held-out coverage | Held-out precision |
|---:|---:|---:|---:|---:|
| 0 | 0.372690 | 42.29% | 50.00% | 94.20% |
| 1 | 0.296825 | 52.79% | 46.21% | 83.61% |
| 2 | 0.337696 | 47.31% | 46.56% | 88.52% |
| 3 | 0.255849 | 58.07% | 50.38% | 83.33% |
| 4 | 0.322218 | 49.25% | 55.80% | 88.31% |

Every selected inner threshold retained at least 90% inner out-of-fold
precision. Two untouched outer folds then fell below 85%. This is the
instability the nested design was intended to expose.

## Target structure

Ambient, House, Reggae, and Garage selective offers exceeded 95% precision.
Techno reached 85.71% precision on 49 offers. Four supported targets failed the
75% precision guard:

| Target | Truth support | Offers | Offered precision |
|---|---:|---:|---:|
| Breakbeat | 18 | 5 | 60.00% |
| IDM | 10 | 6 | 16.67% |
| Minimal | 10 | 6 | 0.00% |
| Tech House | 16 | 13 | 46.15% |

This is not repaired by accepting the strong majority targets: the product
contract requires a broad suggestion to remain honest across every target it
offers often enough to assess.

## Frozen gate

| Check | Required | Observed | Result |
|---|---:|---:|---|
| Offered precision | 90% | 87.72% | Fail |
| Coverage | 50% | 49.85% | Fail |
| Every-fold precision | 85% | 83.33% minimum | Fail |
| Supported-target precision | 75% | Four failures | Fail |
| Precision improvement over unselective | +10 pp | +15.63 pp | Pass |

## Decision

The supervised broad adapter is a bounded negative and does not advance to a
listening holdout. The exposed corpus must not be used to tune its ridge
penalty, PCA size, feature weights, or confidence threshold.

The result also closes the same-representation adapter path. The next useful
experiment must ask a new audio-feature or representation question. The
already-cached beat-relative kick pattern is the smallest mechanism-grounded
feature question: it may help distinguish straight and broken rhythmic
families without rerunning audio analysis. It cannot be assumed to resolve the
House/Techno boundary, where both genres commonly use four-on-the-floor kicks.

## Reproducibility

- private aggregate result SHA-256:
  `4299c029062b122a1a54394c6baf879497a065e5f26c5f87df82cc5d6e205fdc`
- private manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- private feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
