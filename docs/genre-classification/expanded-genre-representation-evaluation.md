# Expanded Genre Representation Evaluation

**Run date:** 2026-08-02

**Status:** Development evidence only; every trainable candidate failed its
frozen gate

## Question and boundary

This was the single expanded-corpus rerun pre-registered in Plan 059. It asked
whether the unchanged trainable representations from Plans 056 and 058 improve
after 51 independently approved references were added to `genre_verified`.

The evaluation remained read-only. Current Rekordbox genre was removed from
classifier inputs, every fitted component was trained outside its evaluation
fold, and artist, remixer, release, and related-version groups did not cross
folds. Stored profiles were neither read nor written. Private identities,
paths, features, fold assignments, and row predictions remain outside Git.

This corpus is exposed development truth. It cannot later become a sealed
holdout.

## Corpus

- playlist and canonical truth rows: 696
- usable rows with fresh, valid Stratum and Essentia evidence: 670
- excluded rows: 26, all missing both analysis backends
- invalid or unscorable analysis rows: 0
- represented truth genres: 28
- connected leakage groups: 252
- largest leakage group: 41 rows
- fold row counts: 138, 132, 131, 131, and 138
- fold group counts: 48, 51, 49, 50, and 54

The expansion repaired several sparse development genres. Usable support is now
Dub 10, Electro 41, Garage 11, Hardcore 9, IDM 10, Minimal 10, and Tech House
16. The live playlist has 42 raw Electro labels; one lacks complete analysis.

## Aggregate results

| Configuration | Exact | Macro recall | Macro F1 | Same-family | Gate |
|---|---:|---:|---:|---:|---|
| Unchanged classifier baseline | 39.70% | 32.90% | 30.85% | 60.90% | reference |
| Fold-trained Fisher profiles | 38.51% | 24.38% | 23.65% | 61.34% | fail |
| Direct EffNet style projection | 59.85% | 47.09% | 44.55% | 78.81% | diagnostic only |
| Fixed 70/20/10 EffNet fusion | 61.19% | 48.25% | 45.68% | 78.81% | fail |
| Style + baseline + arrangement adapter | 55.97% | 46.04% | 39.42% | 73.43% | fail |
| Plus embedding PCA64 adapter | 60.45% | 42.98% | 38.83% | 77.61% | fail |

Every EffNet configuration improved macro F1 in all five folds, exact accuracy,
macro recall, and same-family accuracy over the baseline. Those broad gains are
real development signal, but they do not override the frozen genre-level
safeguards.

## Decisive failures

Fisher profiles failed four of their six checks. They reduced aggregate exact
accuracy, macro recall, and macro F1. Supported-genre recall fell from 50% to 0%
for Dub and from 58.54% to 36.59% for Electro.

The fixed EffNet fusion cleared every aggregate and fold check but exceeded the
allowed 10-point recall loss for three supported genres:

| Genre | Support | Baseline recall | Fusion recall | Delta | Leading fusion errors |
|---|---:|---:|---:|---:|---|
| Breakbeat | 18 | 22.22% | 11.11% | -11.11 pp | Techno 6; Dubstep 4; Downtempo 2 |
| Deep Techno | 19 | 42.11% | 5.26% | -36.84 pp | Techno 16; Dub Techno 2 |
| IDM | 10 | 40.00% | 0.00% | -40.00 pp | Techno 5; Downtempo 2; Drum & Bass 2 |

The adapter without embeddings exceeded the same guard for House and IDM.
House recall fell from 26.81% to 16.67%; IDM fell from 40% to 10%. The PCA64
adapter also reduced IDM recall to 10%. Its macro-F1 improvement was 7.98
points, just below the required 8 points, so the recall failure was not its only
failed condition.

## Decision

No configuration is selected, no production implementation is authorized, and
no sealed holdout is warranted. Additional parameter, weight, threshold,
feature, alias, or genre-specific exception searches on these 670 exposed rows
would be post-hoc tuning and are out of bounds.

The stable conclusion across both corpus snapshots is that Discogs-EffNet adds
useful broad-family and review-prioritization evidence but is not a safe
autonomous fine-genre classifier. The next useful product experiment is a
read-only genre audit: rank high-information disagreements for small human
listening batches, preserve ambiguous verdicts, and never write metadata from a
model prediction.

## Reproducibility record

- corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
- Stage A aggregate SHA-256:
  `d0da123426ad997cbbf1417d5b8c1d72751cae4f523565ae6e00015ea7d99c86`
- private manifest SHA-256:
  `a56baa00a1114e9838bb3eed5dc9be7a4e18c0b85f1ab7dfdb052fa7eeb8ffd9`
- private feature artifact SHA-256:
  `5e4dd072b135fad9ec4f591333b5374a9009db26188bd724efd804a4d5946fcd`
- Stage B aggregate SHA-256:
  `57099b002344c4840a80f75db5b73b81bc8679336a6f8d8c5b8ff749511d62da`
- supervised-adapter aggregate SHA-256:
  `e3a38d9e9fa8bf35beed9cfbfd260822c10745377cc203776c8e3e2967d7f43f`
- model SHA-256:
  `a280825b334797cf677939db8cd5762c0392aedd0ca6415dbc1cd083f045e43c`
- model metadata SHA-256:
  `a2e85b2e7372d5f8e0f35bdd6aeae1139f101087d183d0b2fb60b0ea0f01a0ff`
- isolated runtime: ONNX Runtime 1.28.0; NumPy 2.5.1

The direct style projection and the Plan 057 hard router were not promotion
candidates for this rerun. Their deterministic old predictions could not
repair the already exposed Breakbeat and Deep Techno failures; the hard router
could not mathematically reach its frozen 60% exact-accuracy gate on the
expanded row count.
