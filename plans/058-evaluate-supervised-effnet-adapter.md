# Plan 058: Evaluate a supervised EffNet genre adapter

> **Status:** Complete on 2026-08-02; both configurations bounded negatives
> **Objective:** Test whether a fixed shallow supervised adapter can combine
> Discogs-EffNet's broad signal with Reklawdbox's existing fine decisions.

## Evidence Boundary

Plans 056 and 057 exposed the same 619-row, five-fold development corpus.
Plan 056's flat mapping gained broad accuracy but regressed three supported
genres. Plan 057's hard family router repaired those regressions but discarded
too much aggregate gain. This plan is post-hoc development screening on the
same exposed rows. A passing result can justify a fresh sealed holdout only; it
cannot authorize production behavior.

The configurations, fitting rule, regularization, and gate below are frozen
before adapter outputs are inspected. No parameter or feature search follows
this run.

## Shared Inputs

For every row, use only Plan 056 artifacts and the unchanged baseline output:

- 53 projected canonical Discogs-EffNet style scores;
- one-hot encoding of the baseline classifier recommendation, including a
  zero vector for abstention;
- loudness range, dynamic complexity, spectral-flux mean, and onset rate; and
- for the second configuration only, the normalized 1280-dimensional
  Discogs-EffNet embedding.

Current Rekordbox genre remains absent from classifier inputs. The truth label
is used only to fit training folds and measure held-out folds.

## Fixed Adapter

Fit a class-balanced one-versus-rest ridge least-squares head independently in
each outer fold:

1. candidate classes are truth genres present in the other four folds;
2. missing arrangement values use training-fold means;
3. every input column is centered and scaled using training-fold statistics;
4. sample weight is inversely proportional to training class frequency and is
   normalized to mean 1;
5. add an unpenalized intercept; and
6. use a fixed ridge penalty of 10.0 for every other coefficient.

Evaluate exactly two configurations:

- **style + baseline + arrangement**; and
- **style + baseline + arrangement + embedding PCA64**, where PCA is fitted
  inside the training fold, uses the top 64 centered right-singular vectors,
  and is neither whitened nor tuned.

If both pass, select the higher macro-F1 configuration, then exact accuracy,
then the configuration without embeddings. This tie-break is frozen.

## Development Gate

Relative to the unchanged Plan 056 baseline, a configuration passes only if:

1. macro F1 improves by at least 0.08;
2. macro recall improves by at least 0.08;
3. exact accuracy improves by at least 0.05;
4. every fold improves macro F1;
5. same-family accuracy does not decrease; and
6. no genre with at least ten rows loses more than 0.10 recall.

Passing means `development_candidate_for_new_holdout`. If both fail, record
bounded negatives and stop representation work until new truth or a separately
justified feature question exists.

## Safety And Privacy

- The harness is isolated research code and adds no production dependency.
- Rekordbox, audio files, and the Reklawdbox cache remain read-only.
- Private features and row outputs stay outside Git.
- Only aggregate metrics, frozen methodology, and checksums may be committed.
- No CLI, MCP, cache schema, tag, XML, or classifier behavior changes.

## Recorded Result

Neither frozen adapter passed:

| Configuration                  |  Exact | Macro recall | Macro F1 | Same-family |
| ------------------------------ | -----: | -----------: | -------: | ----------: |
| Style + baseline + arrangement | 56.54% |       39.41% |   32.98% |      73.67% |
| Plus embedding PCA64           | 62.68% |       35.20% |   33.03% |      77.87% |

Both improved fold macro F1, exact accuracy, and macro recall over baseline;
neither regressed any genre with at least ten rows by more than 0.10 recall.
The first missed the macro-F1 gate. The embedding configuration missed both
macro gates. Its higher exact accuracy came with weaker balanced genre
coverage.

Result SHA-256:
`1de33d1461f9bf0ab42c6e11da626ff75341b3a8e9cdd43c18a4d6e6f857c771`.
No configuration is selected. Representation tuning on this exposed snapshot
stops here.
