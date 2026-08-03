# Plan 068: Build Genre Intelligence V1

> **Status:** In progress; blind truth batch B04 awaiting review
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

Reviews use an adaptive batch size. Ambiguous boundary work defaults to no more
than six tracks. A preregistered, coherent selection may contain seven to twenty
tracks when the expected information gain justifies the extra listening load;
no blind batch exceeds twenty tracks. The operator may stop or skip at any
point and sees identity and audio only—not the current genre, sampling stratum,
provider genre, model prediction, confidence, neighbour genres, or hidden
target.

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
- an active-review query returning six or fewer diverse rows by default, with
  an explicit requested limit capped at twenty;
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
enter development truth. This prerequisite was satisfied by the verified
recovery recorded below.

## Retired B02 attempt

An attempted batch B02 used the persistent pre-holdout
`genre-reference-candidates-v2.xml` export. Aggregate preflight found nine rows
outside `genre_verified` in the weak Minimal and IDM roots, all protected from
holdout leakage by prior membership in the frozen
`genre_reference_candidates` exclusion playlist. A selector and six-row export
were frozen before identities were inspected.

The identity check then showed that absence from `genre_verified` and other
playlist exports did not prove absence of prior human review. These candidates
came from the earlier Plan 038 reference workflow, and multiple selected rows
already had conversational listening assessments. Treating them as fresh blind
truth would violate this plan's explicit repeated-review boundary.

Batch B02 was therefore retired before presentation. No listening verdict was
requested, no row entered the truth ledger, and the user-facing XML was removed
from the export directory. Its private artifacts are retained only as audit
evidence. The selector is removed rather than left as an apparently valid
entry point. A replacement batch must use either the exactly reconstructed
unexposed Plan 067 roster or newly sourced tracks whose post-seal provenance
proves they cannot belong to either holdout.

## Retired-roster recovery

The original Plan 060 audit and Plan 067 roster files were lost from temporary
storage, but their frozen recipes, aggregate counts, source hashes, current
library snapshot, historical review exports, and roster checksums remained.
The recovery script rebuilds identity-only exclusion inputs from those sources;
it never reads a model feature or prediction. It fails unless the read-only
library snapshot and both historical roster checksums match.

The live snapshot reproduced the recorded SHA-256
`553dcfd0c5526f2ca8309f1a53097ac58749c8d54216879c80390c355264e653`.
The recovered Plan 066 60-row roster reproduced
`e90b400645d89b287aab4300465fd0893314830bc6ec8b6ab22b5f9de4fbfdf9`,
and the recovered Plan 067 48-row roster reproduced
`9cf4cdbd67bc701063d886e991e7f4f57a0b675844423584b3027f0bce5418a9`.
The Plan 067 target counts also matched exactly. A deterministic replay was
byte-identical.

One unselected row from the historical candidate universe cannot be recreated
from the current audit inputs: the recovered Plan 067 universe has 489 eligible
rows rather than the historical 490. This does not change either selected
roster, as proven by both frozen roster checksums, and is retained explicitly
in private recovery provenance rather than guessed. Replaying B01 from the
recovered roster produced the same six identities as both its imported XML and
the append-only truth ledger.

Private recovery record:

- recovered Plan 066 artifact SHA-256:
  `1468cd2cda5465a7b5d7aebbb8d736800f51454cfc2ae14b4bd96b093d04fb37`;
- recovered Plan 067 artifact SHA-256:
  `cf0787eadbb8979ac915a796e2642e2ad697cbe638cec0fc47b7a6aca64b5532`;
- original Plan 067 artifact SHA-256 retained in provenance:
  `7a188602d547052cc2ede517d74458d77bdd69509aefc2c67e3dac1fab3ff00f`;
  and
- recovered artifacts and mappings are mode 0600 outside Git.

## Batch B03 pre-review result

B03 is the replacement for the retired B02 attempt. It uses only untouched
rows from the checksum-verified retired Plan 067 roster and excludes every B01
path, normalized artist, and release group. Before identities were inspected,
the selector froze a coherent twenty-row batch covering the two Plan 066
supported targets that failed their per-target gate and four smaller roots:

- Breakbeat: four;
- Disco: three;
- Downtempo: two;
- Drum & Bass: three;
- Electro: two; and
- Trance: six.

These are private sampling strata, not truth labels. The operator sees only
identity and audio, and may classify, mark ambiguous, skip, or stop at any
point. All twenty paths, artists, and release groups are distinct. A second
selection run was byte-identical.

The exporter re-read and matched all twenty live identities, confirmed no
staged changes before and after export, and wrote only a playlist XML plus
blank review material. The review projection contains exactly identity and
response fields; it contains no sampling stratum or model output. All private
artifacts and review files are mode 0600.

Reproducibility record:

- roster SHA-256:
  `3330b4d362419087de783edf43f54c22823e623b93fe0063c381be9cd05b50f2`;
- byte-identical pre-export mapping SHA-256:
  `541a5050ba3ccde976eea394b81cbf68855b4062fdaf75975e04915740014545`;
- exported private mapping SHA-256:
  `2ac262da93f5a8f935eacd0401abcd37a5bc382cabcd3f22871b21090f57ab59`;
- playlist XML SHA-256:
  `9a6ccdc1a7b968b8fcfcdc0e833f7d9a57daf62c289a0af61984e47bce422174`;
- blank review sheet SHA-256:
  `5c0d079e7c0f9043192ca5bf5f411e791dab87bb369804adb36e791fcb9fab85`;
  and
- blind review guide SHA-256:
  `81ce73b2a4c8363c6ab6f1e4a7a706a1b060333525e3e08edc1426a6ecddbb7d`.

## Batch B03 review and ingestion result

The operator completed all twenty rows without seeing sampling strata or model
outputs. Canonicalization changed only representation, not the listening
decision: case variants were normalized, Deep House became parent House, Deep
Techno became parent Techno, and Hard trance became parent Trance. Original
genre, confidence, and alternative wording is retained alongside the canonical
fields. Jungle alternatives on Drum & Bass verdicts remain raw subgenre notes
rather than being represented as a competing parent.

Fifteen high- or medium-confidence labels became model-eligible truth. Four
low-confidence labels remain in the audit ledger but not the model snapshot,
and one `unsure` response became an explicit ambiguous outcome with Breakbeat
and Downtempo alternatives. One uncertainty note entered the TSV's alternatives
cell because its empty alternatives field was omitted; the private verdict
preserves the exact source cell and copies the sentence into notes.

Ingestion record:

- reviewed rows: 20;
- model-eligible rows added: 15;
- outcomes: 19 labels and 1 ambiguous;
- eligible additions: Breakbeat 2, Disco 2, Drum & Bass 2, House 2, Techno 2,
  and Trance 5;
- completed review sheet SHA-256:
  `0a6916666ad13b960f29adb7dae3361932fc07dee22bb6b25789209a2ea56a01`;
- private verdict input SHA-256:
  `1f28bc2efbe7453083cfa13d3b1fa7f41777a7e9e632f25e2000c14ba70813b0`;
- private ledger SHA-256:
  `cded1b49020001485e3aa0497321bef99aaee51607765009d9c48c7ea63b8cf0`;
- private snapshot SHA-256:
  `0d2765dfe9c10b2a55764123ca45fe242232b0db5758f9e7474f02aa7ac70971`;
- model-ready truth fingerprint:
  `5942ffb2a632f15850d8d78b4d3aaf4d5b9e52b5d6618221f7270840e8c39674`;
  and
- idempotent replay additions: 0.

## Combined support audit and B04 freeze

A deterministic builder now joins the unchanged 668-row development corpus to
eligible blind-review truth, maps fine labels to the frozen parent taxonomy,
and applies the preregistered artist-diversity cap. It retains all accepted rows
for audit but places only support-qualified parents in the model-ready release
scope. Its parent-taxonomy semantic checksum matches the historical Plan 062
through Plan 066 mapping exactly.

The combined corpus contains 688 accepted rows. After deterministic diversity
balancing, 555 rows across seven parents are model-ready: Ambient, Breakbeat,
Electro, House, Reggae, Techno, and Trance. The twelve-parent support gate
therefore still fails. In particular, Tech House needs four rows, IDM needs ten,
and Downtempo needs fifteen; other nearby roots include Garage, Minimal,
Hardcore, Disco, and Drum & Bass. No candidate training is authorized yet.

Combined-corpus record:

- accepted corpus fingerprint:
  `2672df82289392da421a990fb999e37af0c3663fb69dc56c47cba6faa81c9b1a`;
- diversity-balanced model-ready fingerprint:
  `aff9286fd1f2637559883c8e60c7761d0f02786572bffa8e22046c58143ee3b7`;
- private artifact SHA-256:
  `4c71348627af625b122fea206a340b8011c19c5a6cff2b81e06b1d11d70515f4`;
- byte-identical replay: yes; and
- private artifact mode: 0600.

Before any B04 identity is inspected, freeze a model-directed sampling batch
using the already-exposed v0.33 recommendation only as a private sampling
stratum. The recommendation and its confidence are not truth and are absent
from the review sheet. No confidence filter is applied. Exclude the development
corpus, the sealed Plan 066 holdout, every truth-ledger row including ineligible
outcomes, retired B02, and every prior genre-review XML. Exclude their paths,
normalized artists, and artist-release groups. Require one path, artist, and
release group per selected row.

B04 fixed quotas are:

- Tech House: four, which can close its row deficit;
- IDM: ten, which can close its row deficit and materially improve artist and
  release diversity; and
- Downtempo: six, as the first bounded contribution toward its larger deficit.

The frozen private pool contains 452 candidates after 921 path, 496 artist, and
613 release-group exclusions. Its relevant private sampling counts are Tech
House 17, IDM 37, and Downtempo 23. Twelve rows have metadata changes relative
to the old audit snapshot; the pool requires the same live track ID and path,
records the drift count, and uses current live identity. No audio is missing.

B04 candidate-pool record:

- source audit artifact SHA-256:
  `d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e`;
- pool fingerprint:
  `94bd0521fb29f358c23b3d9b4b8da3039c2cb8b6e0520a5ff78cdcdea3419e4f`;
- private pool artifact SHA-256:
  `78d4679e1945f0e5c687c92cd4fb85e925ed5cf2ad847d9e0a062abd5518d9a7`;
- byte-identical pool replay: yes; and
- private artifact mode: 0600.

## Batch B04 pre-review result

The pool builder, fixed quotas, selector, and regression tests were committed
before the first selection. B04 then selected twenty distinct paths, normalized
artists, and artist-release groups. A second selection run was byte-identical.
Sampling strata, source recommendations, and their confidence remain private
and are absent from the review material.

The exporter re-read and matched all twenty live track identities, required an
empty change set before and after export, and wrote only a playlist XML plus a
blank review sheet and listening guide. The XML contains exactly the selected
paths. All private mappings and review files are mode 0600.

Reproducibility record:

- selector-freeze commit: `f806550`;
- roster SHA-256:
  `56fbb77dc079a039b8cb764df6aedb53b7ab13abceb46334c7336cd7dae16260`;
- byte-identical pre-export mapping SHA-256:
  `710cf557b75e6d9c69ac7b449b62c832b6192f167a705e3c9f11af74b4393b58`;
- exported private mapping SHA-256:
  `ff8bb683f798bc9c81db4d9fbb85584e363fb234f1e6f89c2b3381a8b9ffc0a4`;
- playlist XML SHA-256:
  `eaee76dfc41a1006a27776b6895ffcae25d4391ffe958cb238da33e63b3c31b9`;
- blank review sheet SHA-256:
  `939c15f2d96aef99496ebb832716c005e0c4887a29a83da5578fe229738cbb63`;
- blind review guide SHA-256:
  `88777dfec5d28c2c200146563e0bac1ac1140a8c66d3f53b399eba5c65e14347`;
- live identity matches: 20; and
- zero staged changes before and after export: yes.

## Done criteria

This plan is complete only when either:

1. a candidate passes development and independent release gates, the complete
   preview-first classifier and active-review loop are integrated and verified,
   and Genre Intelligence V1 is prepared for release; or
2. the bounded, sufficiently supported experiments fail, their negative result
   is documented, and the best-supported next direction is recorded without
   contaminating remaining evaluation data.
