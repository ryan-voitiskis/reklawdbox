# Broad Genre Deterministic Evaluation

**Run date:** 2026-08-02

**Status:** Development gate failed; parent-consensus rule retired unchanged

## Question and boundary

Plan 062 asked whether a conservative broad-genre projection could make the
existing v0.33 evidence pipeline release-grade without adding a model runtime.
The mapping, selection rule, and gates were committed before this evaluation
ran.

The broad mapping is separate from the mixing-oriented `GenreFamily`. Clear
subgenre lineages collapse to roots, while cross-cutting or already-broad
genres retain themselves. `Experimental` remains unmodeled.

Current Rekordbox genre was removed from every classifier input. Stored audio
profiles were disabled. The run read the live database and caches without
writing Rekordbox, cache, audio, tags, staged metadata, or XML. Track
identities, paths, folds, row predictions, and evidence remain private.

## Corpus integrity

The live `genre_verified` snapshot had 668 usable and broad-eligible rows. The
prior development snapshot had 670. A private identity comparison found two
removed paths—one Electro row and one Jungle row—with no additions or
truth-label changes.

The result is robust to that drift. If both missing rows were restored and
counted as perfect candidate offers, candidate precision could reach only
82.53% and coverage 24.78%, still well below the 90% and 50% gates.

## Aggregate results

| Configuration | Offers | Coverage | Offered precision | Accuracy | Macro recall | Macro F1 |
|---|---:|---:|---:|---:|---:|---:|
| Unselective v0.33 broad projection | 666 | 99.70% | 48.65% | 48.50% | 36.57% | 35.27% |
| Current High/Medium projection | 204 | 30.54% | 73.04% | 22.31% | 20.38% | 27.85% |
| Frozen parent consensus | 164 | 24.55% | 82.32% | 20.21% | 15.88% | 23.41% |

The parent-consensus rule increased precision by rejecting cross-parent
candidate sets. It also rejected many correct recommendations and did not
recover enough low-confidence same-parent disagreements to offset that loss.
It therefore covered less of the corpus than the existing High/Medium filter.

## Candidate fold stability

| Fold | Eligible | Offers | Coverage | Offered precision |
|---:|---:|---:|---:|---:|
| 0 | 138 | 40 | 28.99% | 92.50% |
| 1 | 132 | 41 | 31.06% | 92.68% |
| 2 | 131 | 20 | 15.27% | 95.00% |
| 3 | 130 | 39 | 30.00% | 51.28% |
| 4 | 137 | 24 | 17.52% | 87.50% |

Three folds were precise but sparse. One fold collapsed to nearly coin-flip
precision. This instability independently rules out a holdout nomination.

## Frozen gate

| Check | Required | Observed | Result |
|---|---:|---:|---|
| Offered precision | 90% | 82.32% | Fail |
| Coverage | 50% | 24.55% | Fail |
| Every-fold precision | 85% | 51.28% minimum | Fail |
| Supported-target precision | 75% | Techno 64.15% on 53 offers | Fail |
| Precision improvement over unselective | +10 pp | +33.67 pp | Pass |

## Decision

The deterministic parent-consensus rule is a bounded negative. It is not a
production feature, receives no threshold or genre-specific tuning, and does
not advance to a sealed holdout.

The broad taxonomy remains useful. The result shows that label contraction
cannot repair the underlying evidence quality or confidence calibration in
v0.33. The next justified experiment is to evaluate the already-frozen
Discogs-EffNet representation as broad evidence, using this exact mapping and a
confidence rule frozen before its broad scores are inspected.

## Reproducibility

- rule: `broad-parent-consensus-v1`
- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- private aggregate result SHA-256:
  `db69a9bee309f1c08e109b643c30bc6103f5bd6c9040f2906578f0c7d33d6f90`
- live corpus fingerprint:
  `sha256:b88911c7b24bbeecd1d59607ceb5e873ca29ff6f15052e77913635e5832471f1`
