# Kick-Rhythm Broad Genre Evaluation

**Run date:** 2026-08-02

**Status:** Development gate failed; kick evidence retained, classifier retired

## Question and boundary

Plan 065 tested whether the already-cached Stratum schema 21 kick-pattern
evidence added independent broad-genre signal to the frozen Plan 064 adapter.
The 74 added values represented availability, coarse kick pattern, confidence,
kicks per bar, analysis basis, and an L1-normalized 64-bin beat-relative
histogram.

The feature schema, private artifact, model, preprocessing, nested confidence
calibration, and release gate were committed before genre-conditioned results
were inspected. The run reused existing caches and embeddings; it decoded no
audio and wrote no Rekordbox metadata, cache row, tag, staged change, or XML.

## Aggregate results

| Configuration | Offers | Coverage | Offered precision | Accuracy | Macro recall | Macro F1 |
|---|---:|---:|---:|---:|---:|---:|
| Plan 064 selective adapter | 334 | 49.85% | 87.72% | 43.73% | 37.62% | 45.28% |
| Kick-augmented selective adapter | 362 | 54.03% | 90.61% | 48.96% | 36.32% | 43.85% |

The new rhythm evidence added 28 offers while increasing offered precision by
2.88 percentage points. Both primary aggregate thresholds passed. Unselective
broad accuracy also improved from 72.09% to 73.13%.

The gain was not uniform. Macro recall fell by 1.30 points and macro F1 by 1.43
points. The feature helped majority-target selection more than balanced target
coverage.

## Fold stability

| Fold | Threshold | Inner coverage | Held-out coverage | Held-out precision |
|---:|---:|---:|---:|---:|
| 0 | 0.306784 | 52.07% | 55.80% | 97.40% |
| 1 | 0.231816 | 62.08% | 55.30% | 83.56% |
| 2 | 0.332153 | 51.02% | 46.56% | 88.52% |
| 3 | 0.253891 | 60.67% | 53.44% | 87.14% |
| 4 | 0.319767 | 53.20% | 58.70% | 95.06% |

Every inner threshold retained at least 90% nested out-of-fold precision.
Fold 1 nevertheless fell below the frozen 85% outer-fold floor. A global 90.61%
result therefore overstates stability across independent artist and release
groups.

## Target structure

Ambient, House, Reggae, Hardcore, and Techno had strong selective precision.
Garage was the formal supported-target failure:

| Target | Truth support | Offers | Correct | Offered precision |
|---|---:|---:|---:|---:|
| Garage | 12 | 7 | 5 | 71.43% |

One additional correct Garage offer would have crossed the numeric guard, but
changing the gate or targeting Garage after seeing the result would invalidate
the preregistration.

Small-offer diagnostics also matter. Breakbeat produced one correct offer out
of three, and Minimal produced none out of four. They did not activate the
formal five-offer precision guard, but they show that aggregate success is not
yet broad, balanced usefulness. Tech House improved to four correct offers out
of five.

## Frozen gate

| Check | Required | Observed | Result |
|---|---:|---:|---|
| Offered precision | 90% | 90.61% | Pass |
| Coverage | 50% | 54.03% | Pass |
| Every-fold precision | 85% | 83.56% minimum | Fail |
| Supported-target precision | 75% | Garage 71.43% | Fail |
| Precision improvement over unselective | +10 pp | +17.47 pp | Pass |

## Decision

The classifier is a bounded negative and does not advance to a listening
holdout. The kick evidence itself is a positive engineering result: it supplied
independent information and moved both aggregate objectives in the right
direction. It should remain available to a future model.

The current exposed-corpus sequence now stops. The next classifier effort
should begin with an independent evaluation design and either:

1. a genuinely stronger pretrained music representation whose model and
   runtime license are suitable for distribution; or
2. a new truth corpus designed before fitting, with deliberate coverage of
   rhythm families and the House, Techno, Tech House, Minimal, Garage, and
   Breakbeat boundaries.

This result does not justify shipping an automatic genre writer. The broad
taxonomy and explicit-abstention contract are ready to reuse, but a new model
still needs a passing development gate, sealed holdout, production
implementation, and public-contract review.

## Reproducibility

- private aggregate result SHA-256:
  `3ca050377077938d76abdb6239a1652a7c7088ef356419157d15bea56107ad59`
- private manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- private EffNet feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- private kick artifact SHA-256:
  `0b5842935ddbf09e58321a10dce97811790fd77465246cb4eef27a8e9b9d341e`
- kick feature semantic SHA-256:
  `321b994e907896597ee949358ad8817c3c05a4b912b79d9c80521f40f8cd46a5`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
