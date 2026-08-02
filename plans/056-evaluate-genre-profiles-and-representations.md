# Plan 056: Evaluate genre profiles and pretrained representations

> **Status:** Complete on 2026-08-02; both stages recorded as bounded negatives
> **Objective:** Improve broad genre classification using leakage-resistant,
> offline evidence before changing production behavior.

## Why this plan exists

Plan 055 showed that targeted Tech House retrieval improved substantially with
arrangement evidence but still failed its frozen gate. The project now has much
more ear-verified genre truth than when Plan 038 was written. The next useful
question is therefore whether the existing classifier representation works
across genres when evaluated honestly, not whether another hand-authored
Tech House rule can be tuned on exposed examples.

## Locked decisions

1. The product goal is broad genre-classification improvement. Tech House and
   Minimal are important boundaries, not the sole optimization target.
2. Plan 038 is gap-driven; this evaluation may identify future reference needs
   but cannot create a catalog-wide purchasing quota.
3. The live `genre_verified` playlist supplies development truth. It is not a
   sealed final holdout.
4. Current Rekordbox genre is removed from classifier inputs during evaluation;
   retaining the answer as an input would be circular.
5. Each evaluated row is scored only by a registry trained outside its fold.
6. Artist, release, and related-version groups cannot cross train/evaluation
   folds.
7. Stored profiles are never loaded, replaced, or persisted by the harness.
8. Results are aggregate in Git. Track IDs, paths, row predictions, fold
   assignments, and listening notes remain private.
9. No classifier, cache schema, DSP, CLI, MCP, audio tag, XML, or Rekordbox
   behavior changes unless a later implementation plan is separately approved.

## Stage A: current profile system

### Corpus

- Read `genre_verified` from the live Rekordbox database through the existing
  read-only adapter.
- Resolve taxonomy aliases before counting truth labels.
- Require fresh, valid Stratum and Essentia cache rows and the profile scorer's
  optional-feature coverage.
- Report every exclusion category and the per-genre usable counts.
- Build deterministic connected leakage groups from normalized credited artist,
  remixer, release identity, and related-title identity. Ignore generic values
  such as “Various Artists” when forming artist edges.
- Assign connected groups to five folds with deterministic genre-aware greedy
  balancing. Report group and fold distributions without identities.

### Comparison

For every usable row, compare:

- **baseline:** the current classifier with no audio-profile registry; and
- **profile:** the same classifier with a registry calibrated from the other
  four folds only.

Discogs and independent label/audio evidence remain available. The only
intentional input removal is current genre. No threshold or feature weight is
tuned during the run.

### Metrics

Report for both arms:

- exact accuracy with abstention counted as incorrect;
- macro recall and macro F1 across truth genres;
- same-family accuracy and same-family confusion rate;
- abstention and manual-review rates;
- high/medium-confidence precision;
- per-genre support, recall, precision, F1, abstention, and leading confusions;
- fold-level exact accuracy and macro F1; and
- profile availability/coverage for held-out rows.

### Pre-registered promotion gate

The existing profile representation justifies a production implementation plan
only if all conditions hold:

1. macro F1 improves by at least 0.05 absolute;
2. macro recall improves by at least 0.05 absolute;
3. exact accuracy improves by at least 0.03 absolute;
4. same-family accuracy does not decrease by more than 0.02 absolute;
5. no genre with at least ten usable rows loses more than 0.15 recall; and
6. at least 80% of held-out rows have a fold-trained prototype for their truth
   genre.

This is a development gate, not a claim of final generalization. Passing would
justify a new sealed-holdout plan; failing advances to Stage B without tuning
the Fisher profiles on these results.

## Stage B: Discogs-EffNet representation

Run this stage only if Stage A fails.

- Use the official dynamic-batch Discogs-EffNet style-classification feature
  extractor through an isolated ONNX environment. Do not alter Reklawdbox's
  managed Essentia runtime or cache schema. The frozen model is
  `discogs-effnet-bsdynamic-1.onnx`, SHA-256
  `a280825b334797cf677939db8cd5762c0392aedd0ca6415dbc1cd083f045e43c`;
  its official metadata SHA-256 is
  `a2e85b2e7372d5f8e0f35bdd6aeae1139f101087d183d0b2fb60b0ea0f01a0ff`.
- Record the exact official model URL, checksum, runtime versions, input
  preprocessing, window aggregation, and private feature artifact checksum.
- Extract embeddings only for the Stage A usable corpus. Do not analyze the
  whole collection.
- Reuse the exact Stage A leakage groups and folds.
- Extract exactly 12 evenly spaced patches per track across its full duration.
  Match Essentia's documented preprocessing: mono 16 kHz audio, 512-sample
  frames, 256-sample hops, 96 MusiCNN mel bands, and 128-frame model patches.
  Average the model's patch probabilities and L2-normalized embeddings.
- Pre-register two configurations only:
  - **style projection:** map the model's documented 400 Discogs style outputs
    into canonical genres using a frozen exact/alias table, take the maximum
    probability for synonyms, and select the highest-scoring canonical genre;
  - **fixed fusion:** 70% style projection, 20% cosine similarity to
    fold-trained mean embedding centroids, and 10% similarity to fold-trained
    centroids over loudness range, dynamic complexity, spectral-flux mean, and
    onset rate. Normalize and fit both centroid families inside each training
    fold only. A missing local genre centroid contributes neutral similarity
    0.5 and the frozen weights are not renormalized.
- Fit all normalization, neighbor/prototype state, and any linear decision
  layer inside each training fold.
- Compare against the unchanged Stage A baseline and profile result.

The Stage B promotion gate is frozen before embeddings are inspected:

1. macro F1 improves by at least 0.08 over the Stage A baseline;
2. macro recall improves by at least 0.08;
3. exact accuracy improves by at least 0.05;
4. every fold improves macro F1 over baseline;
5. same-family accuracy does not decrease; and
6. no genre with at least ten rows loses more than 0.10 recall.

If neither pre-registered configuration passes, record a bounded negative. Do
not search weights, add DSP features, or request listening labels from the same
development rows.

## Verification

The harness must have deterministic synthetic tests for grouping, fold
assignment, metric denominators, current-genre removal, and train-fold
exclusion. The private run is an ignored opt-in test and writes only to an
explicit temporary output path.

After documenting aggregate results, run the repository standard gate and the
Plan 038 incomplete-corpus gate. Inspect the staged diff for private identities
before committing.

## Outcomes

- **Stage A passes:** stop after documenting the result and write a separate
  sealed-holdout/implementation plan.
- **Stage A fails, Stage B passes:** stop after documenting the representation
  result and write a separate integration and sealed-holdout plan.
- **Both fail:** retain both results as bounded negatives and use per-genre
  errors to choose one small, gap-driven reference or feature investigation.

## Recorded Result

Both stages failed their frozen gates. The aggregate report is
[Broad Genre Representation Evaluation](../docs/genre-classification/broad-genre-representation-evaluation.md).

- Fold-trained Fisher profiles improved exact accuracy by 4.52 percentage
  points and same-family accuracy by 8.40 points, but reduced macro recall by
  2.98 points and macro F1 by 3.75 points.
- The fixed Discogs-EffNet fusion improved exact accuracy by 22.13 points,
  macro recall by 19.85 points, macro F1 by 14.12 points, and same-family
  accuracy by 18.58 points. It nevertheless reduced recall by more than the
  allowed 0.10 for Breakbeat, Deep Techno, and Electro.
- No configuration is selected. No production classifier or stored profile
  behavior changes under this plan.

The evidence supports one next direction: treat the pretrained representation
as broad family evidence and evaluate a separate fine-genre decision inside
families. Plan 057 recorded the first hard-router test, and Plan 058 recorded
the fixed supervised-adapter follow-up; neither advanced to a sealed holdout.
