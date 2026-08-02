# Plan 060: Build a read-only genre-audit MVP

> **Status:** Awaiting six operator verdicts; blind batch ready on 2026-08-02
> **Objective:** Test whether imperfect independent genre signals can prioritize
> likely metadata errors for efficient human review without automatic writes.

## Why this plan exists

Plan 059 confirmed that Discogs-EffNet contains strong broad genre signal but
is not a safe autonomous fine-genre classifier. Its fixed fusion reached 61.19%
exact accuracy and 78.81% same-family accuracy, while still collapsing useful
genres. The safe product question is therefore retrieval, not classification:
can agreement between the existing classifier and the pretrained model produce
a better small listening queue than unassisted browsing?

This is a development experiment. It cannot authorize automatic tagging,
stored profiles, a production model dependency, or a claim of collection-wide
accuracy.

## Frozen candidate universe

Read the live library and analysis cache through existing read-only adapters.
Include only tracks that:

- are not Rekordbox factory samples;
- have a non-empty genre resolving to the canonical taxonomy;
- have a resolvable, existing audio file;
- have fresh, valid, scorable Stratum and Essentia evidence; and
- are absent from every already exposed review or truth playlist listed below.

The frozen exclusion playlists are:

- `genre_verified`;
- `genre_reference_candidates`;
- `genre_discovery_blind_v1`;
- `genre_discovery_v2_tech_house_batch_01`;
- `genre_discovery_v3_tech_house_batch_01`;
- `minimal_candidates`;
- `minimal_research_candidates_v2`; and
- `tech_house_research_candidates_v2`.

Current Rekordbox genre is retained only as the label being audited. It is
cleared before running the existing classifier and contributes no model input.

## Frozen signals

For each eligible row compute:

1. the existing Reklawdbox classifier with no stored profile registry;
2. direct canonical projection of the unchanged Discogs-EffNet style head; and
3. the unchanged 70/20/10 fusion, with embedding and arrangement centroids
   fitted once on all 670 Plan 059 development rows.

The model, metadata, preprocessing, 12-patch aggregation, taxonomy mapping,
features, and weights remain identical to Plan 059. No threshold, alias,
feature, genre exception, or weight search is permitted after candidate output
is inspected.

## Frozen ranked cohort

A ranked candidate must satisfy every condition:

- all three signals recommend the same canonical target;
- the target differs from the canonical current genre;
- baseline confidence is high or medium;
- the target had at least eight Plan 059 development rows; and
- fixed-fusion held-out precision for the target was at least 0.60.

Order candidates lexicographically by:

1. cross-family mismatch before within-family mismatch;
2. high baseline confidence before medium;
3. higher held-out target precision;
4. larger fixed-fusion top-two score margin;
5. larger direct-style top-two score margin; and
6. SHA-256 of the frozen seed and stable track identity.

Select four ranked rows greedily. Require distinct target genres and distinct
artist/release groups, with no more than two targets from one classifier
family. Fail rather than relax the rule if four rows are unavailable.

## Frozen controls and blinding

Select two controls from the same universe. A control must have all three
signals agree with its canonical current genre, must satisfy the same target
support and precision floor, must have high or medium baseline confidence, and
must match a ranked target genre. Choose the closest BPM control for two
distinct ranked targets, then the frozen hash; artist/release groups must be
unique across the whole six-track batch.

Shuffle all six rows deterministically. Keep cohort, current label, target,
model scores, paths, IDs, and mapping outside Git. Export only a playlist with
unchanged track metadata and a blind review sheet containing position, code,
artist, title, verdict, confidence, alternatives, and notes. The reviewer
should hide Rekordbox's Genre column while listening.

The model target and ranked/control cohort stay hidden until verdicts for the
whole batch are returned. `ambiguous` is a valid verdict and does not become
classifier truth.

## Pilot gate

Continue this audit rule to a second batch only if:

1. at least two of the four ranked rows receive a confident verdict matching
   the hidden target; and
2. no more than one of the two controls receives a confident verdict that
   contradicts its current genre.

If the gate fails, record a bounded negative and stop this rule. Do not tune it
against the six exposed rows. If it passes, the reviewed rows may become
development truth after a separate XML reconciliation; they never become a
sealed holdout.

## Safety and privacy

- Rekordbox, audio files, tags, and Reklawdbox cache remain read-only.
- `write_xml` may create a playlist-only export after confirming no staged
  metadata changes before and after.
- Private manifests, feature matrices, rankings, mappings, identities, paths,
  and verdicts remain outside Git.
- The committed result may contain aggregate counts, rules, versions, and
  checksums only.
- The isolated EffNet runtime remains a research dependency, not a supported
  Reklawdbox runtime dependency.

## Verification

- Unit-test full-development centroid fitting, external scoring, margins,
  eligibility, deterministic ranking, diversity constraints, controls, and
  failure without quota relaxation.
- Test that current genre is removed from classifier input and excluded
  playlist members never enter the manifest.
- Confirm the playlist export leaves `preview_changes` empty before and after.
- Inspect committed diffs for IDs, paths, track, artist, title, album, review
  verdict, cohort mapping, or row predictions.
- Run the standard workspace and Plan 038 corpus gates before committing.

## Done criteria

The MVP is ready for review when the frozen harness produces one deterministic
four-ranked/two-control batch, the playlist and blind sheet are verified, no
state is mutated, and only the operator's six listening verdicts remain.

## Pre-review result

The frozen rule completed without relaxation. From 3,129 non-sample library
rows, the manifest excluded 827 unique rows already present in truth or review
playlists, three missing files, 588 empty genres, 70 noncanonical genres, and 29
rows lacking complete analysis. The resulting private scoring universe contains
1,612 usable, previously unexposed rows.

Three-way agreement plus the target support/precision floor produced 14 ranked
and 17 control candidates across ten qualifying target genres. The diversity
and matched-control rules selected four ranked rows and two controls. An exact
replay produced the same feature and roster hashes and semantically identical
selected rows.

The playlist-only XML contains six collection records and six playlist
references. All six IDs, paths, artists, and titles matched the live database at
export time. `preview_changes` was empty before and after export. The blind
review sheet contains no current genre, hidden target, model score, or cohort.

Reproducibility record:

- development corpus fingerprint:
  `sha256:a71b4ecf096c7b5a7abd147c9d91d37845a10fb12e8da684000ac8dfe56f3061`
- candidate corpus fingerprint:
  `sha256:2090ddae22a09d3595c613b2eb203e147ce9f497ca349668be354959da84c1f7`
- private candidate manifest SHA-256:
  `cea1520a6bd930250f032629732f8b53edf4143bfd1b4aabe9d315eb588105be`
- private candidate feature SHA-256:
  `49c5e57aea256cb9a721d9a4215410511e725aaa2f3b8abcfbcb2b10308ca9a1`
- private roster SHA-256:
  `4dacf6e1204db9cb11544cd850450da000525f3c08a91bc128ab47ac10c43060`
- playlist XML SHA-256:
  `3ad9e7e69ffeee2411a8941e95fba56aa5006a60763f785a40612582c1524213`
- blind review sheet SHA-256:
  `7c4d6344106d458a93cc9bfb7d04ffa3891b94912d9bc75629084b540ee42ac7`

No pilot verdict is recorded yet. Hidden targets and cohorts must remain sealed
until all six listening decisions are returned.
