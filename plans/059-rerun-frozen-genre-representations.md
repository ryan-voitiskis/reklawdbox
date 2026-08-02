# Plan 059: Rerun frozen genre representations on expanded truth

> **Status:** Complete on 2026-08-02; all trainable candidates bounded negatives
> **Objective:** Determine once whether the unchanged trainable representations
> merit a new sealed holdout after the verified corpus expansion.

## Evidence boundary

Plans 056–058 exposed the earlier 619-row development corpus. Their outputs are
bounded development evidence and cannot become a sealed holdout. The operator
subsequently imported 51 references approved independently of those outputs.
The live `genre_verified` playlist was then verified read-only against its XML:

- 696 unique playlist paths and 696 canonical truth rows;
- 670 rows with fresh, valid, scorable Stratum and Essentia evidence;
- 26 rows missing both analysis backends and no invalid analysis rows; and
- zero path or genre mismatches against the approved export.

The added truth materially changes sparse development support: Dub 1 to 10,
Garage 3 to 11, IDM 3 to 10, Minimal 1 to 10, Electro 32 to 41, and Hardcore 0
to 9 usable rows. Tech House remains at 16. The live playlist contains 42 raw
Electro labels; one is among the 26 rows without complete analysis. This is
enough new information to
justify one unchanged rerun, but it remains development evidence.

## Frozen corpus and folds

1. Read the live `genre_verified` playlist through the existing read-only
   adapter and resolve taxonomy aliases before scoring.
2. Require fresh, valid, scorable Stratum and Essentia rows. Do not hydrate or
   analyze missing rows as part of this plan.
3. Remove current Rekordbox genre from classifier inputs.
4. Rebuild the deterministic five folds over the complete 670-row snapshot.
   Artist, remixer, release, and related-version connections remain in one
   leakage group.
5. Do not load or persist stored profiles. Do not write Rekordbox, cache, tag,
   XML, or audio state.
6. Keep manifests, paths, identities, features, row predictions, and folds
   outside Git. Commit aggregate results and checksums only.

## Frozen candidates

Run the unchanged Plan 056 baseline and fold-trained Fisher profiles. Preserve
the original profile promotion gate:

1. macro F1 improves by at least 0.05 absolute;
2. macro recall improves by at least 0.05 absolute;
3. exact accuracy improves by at least 0.03 absolute;
4. same-family accuracy does not decrease by more than 0.02 absolute;
5. no genre with at least ten usable rows loses more than 0.15 recall; and
6. at least 80% of held-out rows have a fold-trained truth prototype.

Run the unchanged Plan 056 Discogs-EffNet extraction and record direct style
projection as a diagnostic control. Re-evaluate the unchanged 70/20/10 fixed
fusion because its fold-trained embedding and arrangement centroids can change.
Preserve its original gate:

1. macro F1 improves by at least 0.08 over the expanded-corpus baseline;
2. macro recall improves by at least 0.08;
3. exact accuracy improves by at least 0.05;
4. every fold improves macro F1;
5. same-family accuracy does not decrease; and
6. no genre with at least ten rows loses more than 0.10 recall.

Re-evaluate exactly the two unchanged Plan 058 class-balanced ridge adapters:

- style + baseline + arrangement; and
- style + baseline + arrangement + training-fold PCA64 embeddings.

Use the same fixed penalty, preprocessing, fold-local fitting, gate, and
within-adapter tie-break as Plan 058. Do not search parameters, weights,
features, aliases, thresholds, exceptions, or family definitions.

## Excluded candidate

Do not promote or rerun the Plan 057 hard router. Its decisions are independent
of training truth. It achieved 330 exact rows on the old 619-row snapshot; even
perfect predictions on all 51 additions, plus the most favorable effect from
the one corrected existing truth, cannot reach its frozen 60% exact-accuracy
gate on 670 rows.

The direct style projection is also diagnostic only. The expansion added no
Breakbeat or Deep Techno truth, so its deterministic predictions cannot repair
the earlier supported-genre recall failures for those genres.

## Selection and stop rule

Inspect gate outcomes and aggregate per-genre errors only after all frozen runs
complete. If multiple trainable candidates pass their own original gate,
nominate the one with the highest macro F1, then macro recall, then exact
accuracy, then the lower-dependency implementation.

- If a candidate passes, freeze it and write a separate plan for one newly
  sourced, leakage-isolated holdout. Do not implement or ship it yet.
- If none passes, record bounded negatives and stop representation tuning on
  this exposed corpus. The next plan should instead specify a read-only genre
  audit MVP that ranks listening candidates without automatically classifying
  or writing metadata.

## Verification

- Run deterministic Rust and Python research tests for grouping, folds,
  metrics, genre removal, train-fold isolation, gates, adapters, and privacy.
- Run the opt-in private evaluations only with explicit temporary outputs.
- Confirm corpus fingerprints agree across every stage.
- Record model, metadata, feature-artifact, and aggregate-result checksums.
- Inspect the Git diff for track IDs, paths, titles, artists, releases, fold
  assignments, or row predictions before committing.
- Run the standard workspace gate and the Plan 038 incomplete-corpus gate.

## Done criteria

This plan is complete when the frozen candidates have run once, aggregate
results and their gates are recorded, no private evidence enters Git, and the
result is either nominated for a new sealed-holdout plan or retained as a
bounded negative with representation tuning stopped.

## Recorded result

All frozen trainable candidates failed. The aggregate report is
[Expanded Genre Representation Evaluation](../docs/genre-classification/expanded-genre-representation-evaluation.md).

- Fold-trained Fisher profiles reduced exact accuracy by 1.19 percentage
  points, macro recall by 8.52 points, and macro F1 by 7.20 points.
- The fixed EffNet fusion reached 61.19% exact accuracy, 48.25% macro recall,
  45.68% macro F1, and 78.81% same-family accuracy, but exceeded the recall-loss
  guard for Breakbeat, Deep Techno, and IDM.
- The style/baseline/arrangement adapter exceeded the recall-loss guard for
  House and IDM.
- The PCA64 adapter reached 60.45% exact accuracy but missed the macro-F1 gate
  and exceeded the IDM recall-loss guard.

No candidate is selected and no sealed holdout is warranted. Representation
tuning on this exposed corpus stops. The next justified engineering work is a
separately bounded, read-only genre-audit MVP that uses model disagreement to
prioritize small listening batches without treating predictions as truth or
writing metadata.
