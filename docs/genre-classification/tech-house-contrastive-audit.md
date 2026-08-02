# Tech House contrastive retrieval audit

## Outcome

Plan 055 completed as a **bounded negative** on 2026-08-02. The imported
development corpus was sufficient to test the proposed retrieval method, but
no pre-registered configuration passed the frozen gate. No listening batch,
Genre change, classifier rule, DSP output, cache field, CLI, or MCP surface was
created.

The strongest result came from adding existing arrangement and dynamic
variation evidence to the contrastive ranker. It materially improved ranking
quality and reduced false positives, but held-out recall, positive margins, and
fold consistency still failed. This is useful evidence, not a configuration to
ship or tune post hoc.

## Corpus and evidence boundary

The private manifest contained 254 development rows:

- 16 ear-verified Tech House positives across 16 independent release groups;
- 227 confident hard negatives: 147 House, 52 Techno, 20 Deep Techno, and 8
  Minimal;
- 11 ambiguous boundary rows used for interpretation only; and
- 819 previously exposed candidate IDs held outside the scoring universe.

All 16 positives were resolved against the live read-only Rekordbox library
after import. Current Full Stratum and Essentia evidence was available for all
positives. Seven stale hard-negative rows were excluded rather than mixed with
current evidence, leaving 247 scored rows: 16 positives, 220 hard negatives,
and 11 boundaries.

The positives were frozen before scoring into three independently motivated
strains: six early/stripped, five deep/driving, and five modern/punchy. Genre
and musical key had zero similarity weight. Boundaries never entered training,
threshold selection, or headline metrics.

## Experiment 1: existing cached features

The audit compared the frozen best-single-seed baseline with pooled density,
per-strain density, and per-strain contrastive density. Density used the mean
of the nearest three positive references and nearest five references from each
adjacent genre. A candidate's complete release leakage group was excluded from
its reference set.

The tested feature blocks were timbre only, scalar groove/production axes only,
and the existing Plan 053 combined weights. Normalization was read-only and
recomputed in memory from 3,048 current cached timbral vectors because the
persisted provenance was stale.

The most informative configurations were:

| Metric | Best-single-seed baseline | Best existing contrastive | Arrangement-augmented contrastive |
| --- | ---: | ---: | ---: |
| Average precision | 0.178 | 0.461 | 0.623 |
| AP improvement over baseline | 0.000 | 0.283 | 0.445 |
| Positive contrast margins | 12/16 | 5/16 | 7/16 |
| Deep/driving median percentile | 0.832 | 0.991 | 1.000 |
| Early/stripped median percentile | 0.852 | 0.768 | 0.852 |
| Modern/punchy median percentile | 0.777 | 0.982 | 0.982 |
| Cross-validated recall | 0.688 | 0.688 | 0.688 |
| Cross-validated precision | 0.169 | 0.200 | 0.297 |
| Deep Techno false-positive rate | 0.200 | 0.300 | 0.200 |
| House false-positive rate | 0.300 | 0.186 | 0.107 |
| Minimal false-positive rate | 0.125 | 0.375 | 0.125 |
| Techno false-positive rate | 0.135 | 0.173 | 0.115 |
| Folds won over baseline | 0/16 | 9/16 | 9/16 |

No existing-feature configuration passed. The best existing contrastive result
passed the AP and strain-percentile criteria, but failed positive margins,
held-out recall, per-genre false-positive control, and the required 11 of 16
fold wins.

## Experiment 2: bounded feature audit

Two theory-linked proxy families were frozen before opening their results. Each
was added once at a fixed 0.20 pair-similarity weight to the best existing
contrastive formulation; feature families were not combined and weights or
gates were not searched.

### Kick placement and section balance

This family used kick density, coarse kick-placement mass, beat-row balance,
and main-groove kick-band balance. It was ineligible: only 15 of 16 positives
had a complete valid vector because one lacked a valid main-groove section
measurement. The missing positive was not dropped and no result was computed.

The cached kick histogram also cannot stand in for percussion swing: it records
grid-quantized kick placement, not timing residuals for the percussion layer.
Section kick-band RMS includes the kick itself, so it cannot stand in for
sustained between-kick sub-rumble either.

### Arrangement and dynamic variation

This family used loudness range, dynamic complexity, spectral-flux variation,
intensity variation, main-groove and breakdown occupancy, and section-transition
density. Coverage was complete for all 247 scored rows. One constant dimension
was omitted by the frozen normalization, leaving six active dimensions.

Compared with the best existing contrastive result, it:

- increased average precision by 0.162, from 0.461 to 0.623;
- improved or preserved every strain's median percentile;
- reduced cross-validated false-positive rates for Deep Techno by 0.100, House
  by 0.079, Minimal by 0.250, and Techno by 0.058; and
- brought every adjacent-genre false-positive rate to or below 0.20.

It nevertheless produced only 7 of the required 12 positive margins, retained
11/16 held-out recall (0.688 rather than at least 0.70), and won only 9 of 16
folds rather than the required 11. It therefore did not justify implementation
or a listening experiment.

The current cache has no defensible measurement for beat-synchronous sidechain
recovery, kick-disjoint sub-rumble, percussion microtiming, or vocal/hook
density. Those hypotheses remain untested; this audit does not count weak
proxies as evidence for them.

## Frozen gate and decision

The gate required all of the following:

1. at least 12 of 16 positive contrast margins;
2. every strain median percentile at least 0.75;
3. average-precision improvement at least 0.10;
4. recall at least 0.70 with every adjacent-genre false-positive rate at most
   0.20; and
5. wins in at least 11 of 16 release-group folds.

The arrangement-augmented result passed criteria 2, 3, and the false-positive
part of criterion 4. It failed criteria 1 and 5 and the recall part of criterion
4. The configuration is retired unchanged. A blind batch would turn a
development near-miss into post-hoc tuning, so none was exported.

## Reproducibility and private artifacts

The committed harness is test-only and requires explicit environment variables
to read private manifests. Track IDs, paths, release groups, listening verdicts,
and row-level scores remain outside Git.

Private artifact SHA-256 values:

- existing-feature manifest:
  `7071e3474563ff1095e9540a63e2d956f47ae29211dfe3b8c35014a8ede6911b`;
- existing-feature result:
  `d4340a22d474a2b715a3bb6377a61d1d2d668a8c8456e882a5f0b43e5d43a264`;
- feature-audit manifest:
  `2fd795be070fc08e8f68c4ae661eca5902e7092b8eaed66c033ccbd87224a2ac`;
  and
- feature-audit result:
  `745ce3ac63fce649abf725bce9c154eb3c0ca6298bcac0a56ea3c82d70e93a53`.

Future work must start a new plan and experiment version. The most useful next
evidence would be a genuinely measured audible mechanism, not further weighting
of these development results. Any new DSP proposal needs independent fixtures,
an analysis-cost budget, a cache-version decision, and a fresh held-out corpus.
