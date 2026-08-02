# Broad Genre Representation Evaluation

**Run date:** 2026-08-02

**Status:** Development evidence only; no configuration passed its frozen gate

## Question And Boundary

This evaluation asked whether broad Reklawdbox genre classification improves
with either:

1. the existing Fisher audio-profile representation, calibrated outside each
   evaluation fold; or
2. an isolated pretrained Discogs-EffNet audio representation.

The source of truth was the operator ear-verified `genre_verified` playlist.
Current Rekordbox genre was removed from every classifier input before scoring.
Artist, remixer, release, and related-title connections were kept in one of
five deterministic folds. Stored classifier profiles were neither read nor
written. The run changed no classifier, cache, tag, XML, audio, or Rekordbox
state.

This is a collection-specific development evaluation, not a final claim of
generalization. The corpus was exposed by the experiment and cannot later be
called a sealed holdout.

## Corpus

- playlist rows: 645
- canonical truth rows: 645
- usable rows with fresh Stratum and Essentia evidence: 619
- excluded rows: 26, all missing both fresh audio-analysis backends
- truth genres represented: 27
- connected leakage groups: 203
- largest group: 41 rows
- fold row counts: 126, 122, 121, 122, and 128
- fold group counts: 39, 40, 40, 41, and 43

The corpus is imbalanced. `Deep House` and `House` each have about 140 rows,
while 15 of the 27 truth genres have fewer than ten rows. Macro metrics retain
those sparse genres deliberately; exact accuracy alone would overstate broad
coverage.

## Stage A: Existing Fisher Profiles

The baseline used the current classifier without an audio-profile registry.
The comparison calibrated the existing profile registry from the other four
folds only.

| Metric | Baseline | Fold-trained profiles | Delta |
|---|---:|---:|---:|
| Exact accuracy | 40.23% | 44.75% | +4.52 pp |
| Macro recall | 29.84% | 26.86% | -2.98 pp |
| Macro F1 | 28.96% | 25.21% | -3.75 pp |
| Same-family accuracy | 60.74% | 69.14% | +8.40 pp |
| High/medium-confidence precision | 60.21% | 68.28% | +8.07 pp |
| Manual-review rate | 69.14% | 76.58% | +7.43 pp |

Truth-profile coverage was 572/619, or 92.41%. The profile arm improved exact
and same-family accuracy, but reduced both frozen primary macro metrics. It
therefore failed the pre-registered promotion gate and was not tuned further.

## Stage B: Discogs-EffNet

Stage B used Essentia's official dynamic-batch
`discogs-effnet-bsdynamic-1.onnx` model in an isolated ONNX Runtime environment.
The model consumed 12 evenly spaced MusiCNN mel patches per track: mono 16 kHz,
512-sample frames, 256-sample hops, 96 mel bands, and 128-frame patches. Patch
probabilities were averaged; patch-normalized embeddings were averaged and
normalized again at track level.

The frozen mapping projected 217 of the model's 400 Discogs style classes into
46 modeled Reklawdbox genres. `Ambient Techno`, `Future Garage`, and
`Gospel House` occurred in truth but had no direct style-head target.

Two configurations were evaluated without weight search:

- direct style projection; and
- 70% style projection, 20% fold-trained embedding-centroid similarity, and
  10% fold-trained arrangement-centroid similarity. A genre missing a local
  fold centroid contributed neutral similarity 0.5; weights were not
  renormalized.

| Metric | Baseline | Style projection | Fixed fusion |
|---|---:|---:|---:|
| Exact accuracy | 40.23% | 60.42% | 62.36% |
| Macro recall | 29.84% | 48.01% | 49.69% |
| Macro F1 | 28.96% | 41.23% | 43.08% |
| Same-family accuracy | 60.74% | 78.35% | 79.32% |

Both configurations cleared the aggregate improvements, improved macro F1 in
every fold, and improved same-family accuracy. Neither passed the frozen
per-genre recall safeguard:

| Genre | Support | Baseline recall | Fixed-fusion recall | Delta | Leading fixed-fusion errors |
|---|---:|---:|---:|---:|---|
| Breakbeat | 18 | 22.22% | 11.11% | -11.11 pp | Techno 6; Dubstep 4; Downtempo 2 |
| Deep Techno | 19 | 42.11% | 5.26% | -36.84 pp | Techno 16; Dub Techno 2 |
| Electro | 32 | 65.62% | 50.00% | -15.62 pp | Techno 7; Ambient 4; Downtempo 3 |

Electro illustrates why the frozen guard matters: its F1 improved from 55.26%
to 62.75% because precision increased, but recall still fell materially. Deep
Techno mostly collapsed into its broader Techno family, which explains how the
aggregate and same-family results can be strong while a useful subtype becomes
worse.

The fixed fusion is therefore a **bounded negative**, not a selected or
deployable configuration.

## Follow-up Development Screens

Two frozen follow-ups tested the structure implied by Stage B. Both reused the
exposed corpus and therefore could only nominate a future holdout candidate.

The Plan 057 hard router let Discogs-EffNet choose the family, then retained the
baseline fine label only when it belonged to that same family. It repaired all
three supported-genre recall guards, but fell below its aggregate thresholds:

| Exact | Macro recall | Macro F1 | Same-family |
|---:|---:|---:|---:|
| 53.31% | 41.60% | 36.06% | 78.35% |

Plan 058 then fitted two class-balanced ridge heads inside the same group
folds. The stronger exact-accuracy variant added a training-fold PCA64
projection of the 1280-dimensional embedding:

| Configuration | Exact | Macro recall | Macro F1 | Same-family |
|---|---:|---:|---:|---:|
| Style + baseline + arrangement | 56.54% | 39.41% | 32.98% | 73.67% |
| Plus embedding PCA64 | 62.68% | 35.20% | 33.03% | 77.87% |

Neither adapter regressed a genre with at least ten rows by more than the
allowed 0.10 recall, but neither achieved the required macro-F1 gain. The
embedding variant also missed the macro-recall gate. Both are bounded
negatives.

## Decision

The pretrained representation contains substantially more useful broad genre
information than the current profile system, but none of the three decision
strategies is ready to ship. The flat head loses important specificity; hard
routing restores specificity while discarding too much broad gain; the frozen
linear adapters trade balanced genre performance for exact accuracy.

Further representation tuning on these rows stops. The next useful action is a
truth reconciliation: a read-only audit found 51 unambiguous operator-approved
references absent from `genre_verified`, heavily concentrated in genres that
are sparse or absent in this snapshot. A playlist-preserving XML export was
created with 696 total verified tracks and seven Minimal corrections. It
excludes every explicitly ambiguous or boundary example. Rekordbox remains
unchanged until the operator imports that file.

After import, the project should confirm the expanded truth distribution and
fresh audio coverage, then pre-register whether an unchanged representation
rerun is warranted. That run would still be development evidence, not a sealed
holdout. Any later deployable candidate needs a newly sealed,
leakage-isolated evaluation set accumulated in listening batches of four to six
tracks.

## Reproducibility Record

- Stage A aggregate result SHA-256:
  `698a91a85a477cb4da342c34ee2f9193fe83fd24bcb0b757266ec2b5e6a797e3`
- model SHA-256:
  `a280825b334797cf677939db8cd5762c0392aedd0ca6415dbc1cd083f045e43c`
- official metadata SHA-256:
  `a2e85b2e7372d5f8e0f35bdd6aeae1139f101087d183d0b2fb60b0ea0f01a0ff`
- private feature artifact SHA-256:
  `c601bf1951c1470c83253af61c2c8f14742ce39ea73035a29925992951460d6f`
- Stage B aggregate result SHA-256:
  `64365de48f75a3ffaf7efaf43293560ad5e30a5608da27432a4299c5c5910e98`
- Plan 057 hard-router result SHA-256:
  `aa68e8da56ad07a6f89eae0ba4b2349c040fa2e44eaf8124352a4aef06a34a82`
- Plan 058 supervised-adapter result SHA-256:
  `1de33d1461f9bf0ab42c6e11da626ff75341b3a8e9cdd43c18a4d6e6f857c771`
- isolated runtime: ONNX Runtime 1.28.0; NumPy 2.5.1

Private manifests, audio-derived arrays, row predictions, identities, paths,
and fold assignments are not committed.

## Primary Model References

- [Essentia model catalog](https://essentia.upf.edu/models.html)
- [Official dynamic-batch model metadata](https://essentia.upf.edu/models/feature-extractors/discogs-effnet/discogs-effnet-bsdynamic-1.json)
- [TensorflowPredictEffnetDiscogs reference](https://essentia.upf.edu/reference/std_TensorflowPredictEffnetDiscogs.html)
