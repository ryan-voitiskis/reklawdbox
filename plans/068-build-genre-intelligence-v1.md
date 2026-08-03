# Plan 068: Build Genre Intelligence V1

> **Status:** In progress; blind truth batch B01 ingested
> **Objective:** Release a calibrated parent-genre classifier that is materially
> more useful than Reklawdbox v0.33 and pair it with an auditable active-learning
> loop that turns explicit human verdicts into better development truth.

## Why this plan exists

Plans 062–067 found meaningful broad-genre signal but no release candidate.
The strongest all-target representation cleared its aggregate precision and
coverage gates but failed several smaller targets. A later Ambient, House, and
Techno candidate passed development gates and then offered only 14 predictions
on its 48-track independent roster, below the frozen utility floor. Neither
result justifies a product surface.

The consistent limitation is development-truth breadth and isolation, not the
absence of musical signal. Genre Intelligence V1 therefore treats truth
acquisition, model evaluation, user review, and safe metadata export as one
versioned product loop rather than a sequence of one-off classifier studies.

## Product contract

Genre Intelligence V1 must:

1. suggest a canonical parent genre or abstain;
2. expose calibrated confidence and the reason for abstention without
   presenting similarity as probability or truth;
3. select small, diverse, high-value review batches for explicit human
   verdicts;
4. retain verdict provenance and corpus versions so every model can be traced
   to its exact development truth;
5. keep evaluation rows permanently isolated from training and review-driven
   tuning;
6. stage no Rekordbox change implicitly; accepted metadata still flows through
   preview, `ChangeManager`, and XML for manual import; and
7. ship an initial parent-genre capability while preserving a path to later
   subgenre classification.

This is a selective classifier. A truthful abstention is useful behaviour, but
coverage must be high enough to improve real collection work.

## Baseline and preserved evidence

- Reklawdbox v0.33 is the product baseline.
- The 668-row Plan 066 development corpus is exposed development truth.
- The unopened 60-track Plan 066 roster remains the release holdout. Its
  identities, labels, and predictions must remain sealed until a candidate
  passes every development gate below.
- The Plan 067 model is permanently retired. Its 48-track roster is no longer
  holdout evidence and may become development truth only through blind human
  review after the negative result was committed.
- Plan 067 predictions must not be inspected or used for selection.
- Prior review rows and development artists/releases remain exclusions for the
  Plan 066 release holdout even after new truth is acquired.

## Versioned truth contract

Every accepted truth row must retain, outside Git when it contains private
library identity:

- stable track and decoded-audio identity;
- artist and release grouping used for leakage control;
- canonical parent-genre verdict;
- verdict confidence and any plausible alternatives;
- provenance (`operator_blind_review`, later approved sources, or a separately
  named process);
- batch and corpus version;
- review timestamp; and
- a supersession link when a later explicit verdict replaces it.

`ambiguous` and `skip` are first-class outcomes and never become training
labels. Current Rekordbox genre, store genre, provider tags, model predictions,
and playlist membership may select candidates but are never accepted as truth
without an explicit verdict.

Corpus manifests are append-only by default. A deterministic builder produces
each model-ready snapshot and a content hash. Development rows are grouped by
normalized artist and release in every split.

## Active-learning review loop

Reviews are blind batches of at most six tracks. The operator sees identity and
audio only—not the current genre, sampling stratum, provider genre, model
prediction, confidence, neighbour genres, or hidden target.

Each response is one of:

- one canonical parent genre with high, medium, or low confidence;
- `ambiguous` plus plausible alternatives; or
- `skip`.

Selection priorities, in order, are:

1. parent genres absent from accepted development truth;
2. supported targets that failed a frozen per-target precision gate;
3. targets below the support floor;
4. uncertainty near a frozen candidate's abstention boundary;
5. artist, release, representation, and collection diversity.

No selector may use a sealed-holdout row. Model-directed selection starts only
after the selector, candidate, threshold, and exclusion manifest are frozen.
Repeated reviews of the same audio require an explicit adjudication protocol,
not silent majority voting.

## First frozen truth batch

Batch `genre-intelligence-truth-v1-b01` repurposes the exact retired Plan 067
roster with artifact SHA-256
`7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f`.
The roster is historical experiment material, not holdout evidence.

Before any track identity is inspected, freeze these sampling quotas:

- Breakbeat: three rows, because it failed the Plan 066 supported-target gate;
- Trance: two rows, because it failed the same gate; and
- Pop: one row, because accepted development truth currently has no Pop root
  and only one eligible retired-roster row exists.

Within each sampling stratum, select by SHA-256 of the fixed seed, stratum,
track ID, and path. Require six distinct paths, normalized artists, and release
groups. Fail rather than relax a quota or diversity condition. Sampling labels
remain private and do not become truth.

The exporter must re-read all six live records, require exact identity and path
agreement, confirm no staged changes before and after export, and create only a
playlist XML plus blank review material. It must not compare or display the
live genre.

## Development support gate

Before another model experiment, define the initial release scope as at least
12 canonical parent genres. Every in-scope parent must have at least:

- 20 high- or medium-confidence accepted rows;
- 15 normalized artists; and
- 12 artist/release groups.

No more than 20% of one target's rows may come from a single normalized artist.
Targets below these floors remain explicit unsupported/abstain outcomes rather
than being merged opportunistically after results are known.

## Frozen model-development gate

Once truth support passes, preregister at most three bounded candidate families.
The first candidate should reuse the strongest frozen Plan 066 representation
before adding a materially different model. Candidate selection must use only
artist- and release-grouped development folds.

A candidate advances only if both nested and deployment-threshold views meet:

- at least 90% aggregate offered precision;
- at least 65% aggregate coverage across the declared release scope;
- at least 85% offered precision in every outer fold; and
- at least 80% offered precision for every release-supported target with at
  least eight offers.

Thresholds, feature versions, target mappings, support rules, and the release
scope are frozen before scoring. Failed candidates are retired; do not rescue
them with post-result per-target tuning.

## Independent release gate

Only a candidate that passes every development gate may open the sealed Plan
066 release holdout. The release decision requires:

- at least 30 offers among its 60 rows;
- at least 90% offered precision overall;
- at least 80% precision for every evaluated target with at least five offers;
- no artist, release, decoded-audio, or prior-review leakage; and
- a byte-identical inference replay from committed inputs and source.

One failed independent evaluation retires that exact candidate and threshold.
Do not lower the threshold, remap targets, or substitute another representation
on the exposed release rows.

## Product integration

After an independent pass, implement the smallest complete user workflow:

- versioned model and taxonomy manifests;
- cache-versioned label-blind inference;
- CLI and MCP preview surfaces that distinguish suggestion, confidence, and
  abstention;
- an active-review query returning at most six diverse rows;
- explicit verdict ingestion with conflict and supersession handling;
- optional staging of accepted genre changes only through `ChangeManager`;
- XML export for manual Rekordbox import; and
- documentation explaining supported roots, limitations, privacy, and model
  provenance.

No private audio, library identity, or operator verdict is committed or added
to mandatory tests. Public fixtures must be synthetic or legally distributable.

## Verification

- Unit-test deterministic batch selection, fixed quotas, diversity, exclusion
  manifests, failure without relaxation, and absence of hidden labels from
  review exports.
- Replay every private selection, corpus build, feature extraction, training,
  and inference artifact byte-identically where serialization permits.
- Verify live playlist identities through the read-only database and require
  empty staged changes before and after XML export.
- Test artist/release isolation and permanent holdout exclusions.
- Compare against the exact v0.33 baseline on the same eligible rows.
- Run the standard workspace gate, the maintained Plan 038 corpus gate, and
  the public-surface semantic review before release.

## Stop conditions

Pause model work and return to truth expansion when development support or
per-target stability fails. After three preregistered candidate families fail
on a sufficiently supported corpus, record a rigorous negative result and
choose the next direction from observed error structure. Do not continue model
or threshold sweeps merely because compute is available.

## Batch B01 pre-review result

The selector consumed the exact retired Plan 067 roster after the plan,
selection rules, exporter, and tests were committed. It selected six distinct
paths, normalized artists, and release groups under the frozen three/two/one
sampling quotas without inspecting Plan 067 predictions. A second selection
run was byte-identical.

The exporter matched all six identities and paths against the live read-only
library. `preview_changes` was empty before and after `write_xml`, which wrote a
playlist-only XML with six collection rows and six playlist references. The
blank review sheet and guide contain identity and response fields only; hidden
sampling strata and model outputs are absent. Track identities, paths, sampling
strata, and future verdicts remain outside Git.

Reproducibility record:

- roster SHA-256:
  `05e21bd2a42d233047f52104732563a64556921e0d9c750fd3a4d30c917e74c0`
- byte-identical pre-export private result SHA-256:
  `b5f31402c6d44147ef3495122b2f88bc9ab7d981d8e64a2da876c7cc8b756f29`
- exported private mapping SHA-256:
  `3a729e9e5478c7c0ba14e2873d68c4143740a6ff277c82d5c4e7fbf18bc165e1`
- playlist XML SHA-256:
  `c7fe3e785d4beb28a7a49bd31d76bc0515c1f38bfed1dd1fb252b8024b8b683c`
- blank review sheet SHA-256:
  `589905eb8c21a65c39ead1691b0865b178556323694c74153485774bd113717f`
- blind review guide SHA-256:
  `2c267250e57b2a610f5e026357bd11609236666d85ba7c34ca020ac0b2f94fe4`

## Batch B01 review and ingestion result

All six operator verdicts were returned without exposing the sampling strata or
Plan 067 predictions. Five rows received model-eligible high- or
medium-confidence parent labels: Breakbeat two, House two, and Trance one. One
unusually mixed row was explicitly skipped as unsuitable truth and remains in
the audit ledger without a training label. The two `medium to high` responses
were normalized conservatively to `medium`, and the clear `Trace` typo was
normalized to `Trance` while retaining the original wording and listening
notes privately.

The versioned private ledger records exact live identity, source-file SHA-256,
normalized decoded-PCM SHA-256, the original and normalized confidence,
alternatives, notes, provenance, eligibility, and explicit supersession state.
Its first replay appended zero records and reproduced the same corpus
fingerprint. Both the ledger and deterministic model-ready snapshot are mode
0600 and remain outside Git.

Ingestion record:

- reviewed rows: 6;
- model-eligible rows: 5;
- outcomes: 5 labels and 1 skip;
- eligible genres: Breakbeat 2, House 2, Trance 1;
- private verdict input SHA-256:
  `790e7362538c26c5acc210de37535aa1dd77bd5aa80951dc0ef6bcf2654ffd30`;
- private ledger SHA-256:
  `806192e8ae8bdb2bbd3553cbfc1422bc92097ae3a61d02691a4a96a40dc459f5`;
- private snapshot SHA-256:
  `ac3f1e2d286726cca910dc8adf759a46ab8fc4745718e1885fb3d1acd9d93d2a`;
- model-ready corpus fingerprint:
  `f692100bbb0aec6fe2a329a4c70c7c6941930fb79faf24694f31d4274498c25f`;
  and
- idempotent replay additions: 0.

Before batch B02, reconstruct or independently replace the retired Plan 067
source roster with all Plan 066 holdout exclusions intact. The temporary roster
was lost on host restart. Do not select a convenience replacement from the
live library because an unidentified Plan 066 release-holdout row could then
enter development truth.

## Done criteria

This plan is complete only when either:

1. a candidate passes development and independent release gates, the complete
   preview-first classifier and active-review loop are integrated and verified,
   and Genre Intelligence V1 is prepared for release; or
2. the bounded, sufficiently supported experiments fail, their negative result
   is documented, and the best-supported next direction is recorded without
   contaminating remaining evaluation data.
