# Plan 068: Build Genre Intelligence V1

> **Status:** In progress; candidate A retired and candidate B preregistered
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

The initial release scope is the complete set of support-qualified parent
genres. Every in-scope parent must have at least:

- 20 high- or medium-confidence accepted rows;
- 15 normalized artists; and
- 12 artist/release groups.

No more than 20% of one target's rows may come from a single normalized artist.
Targets below these floors remain explicit unsupported/abstain outcomes rather
than being merged opportunistically after results are known.

Training may begin when at least seven parents pass those floors and their
diversity-balanced rows cover at least 75% of all accepted development truth.
This two-part gate replaces the earlier twelve-parent prerequisite before any
new candidate is scored. The earlier rule made continued review depend on
weak candidate-sampling labels and did not measure how much accepted truth an
honest selective release could cover. Unsupported parents remain distinct
taxonomy outcomes and are forced abstentions; they are not negative examples,
merged labels, or silently discarded audit rows.

## Frozen model-development gate

Once truth support passes, preregister at most three bounded candidate families.
The original Plan 066 OpenL3 and CLAP feature artifacts were temporary and are
no longer present after the machine restart. Their recorded results remain
evidence, but reproducing either representation is a separate compute-heavy
candidate rather than a prerequisite. Candidate A therefore tests whether the
fresh, already-hydrated production analysis plus the existing v0.33 output can
support the now-frozen seven-parent scope. Candidate selection must use only
artist- and release-isolated development folds.

A candidate advances only if both nested and deployment-threshold views meet:

- at least 90% aggregate offered precision;
- at least 65% aggregate coverage across the declared release scope;
- at least 85% offered precision in every outer fold; and
- at least 80% offered precision for every release-supported target with at
  least eight offers.

The candidate must also improve offered precision by at least five percentage
points over v0.33 on the exact paired rows it offers. Report coverage both
within the release scope and against all accepted truth, with unsupported rows
counted as abstentions in the latter diagnostic. The formal 65% coverage gate
remains scoped because unsupported parents are outside the declared product
contract.

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

## Verdict preparation automation

The active-learning loop now includes a deterministic TSV-to-verdict
preparation step. It requires exact batch codes and displayed identity, rejects
blank or partial reviews, maps only the frozen fine-to-parent taxonomy and
explicit aliases, normalizes mixed confidence conservatively, treats the clear
confidence terms `certain` as high and `none` as null, and preserves the
original genre, confidence, alternatives, and notes. `none` remains invalid
for a label verdict. Unknown wording fails closed rather than being guessed.
Ambiguous outcomes require plausible alternatives, and supersession still
requires an explicit record ID.

An explicit repair option handles the observed TSV mistake where a free-form
note was placed in the alternatives cell: it preserves the raw cell and copies
the text to notes without inventing a genre. Replaying the completed B03 sheet
with its one explicit alias and one cell repair reproduced all twenty existing
private verdict rows exactly. Output is atomic, mode 0600, and remains outside
Git.

## Batch B04 review, ingestion, and support result

The operator completed all twenty B04 rows without seeing the sampling strata
or model output. The converter produced nineteen label outcomes and one
ambiguous outcome. Sixteen high- or medium-confidence labels became
model-eligible truth: Ambient 1, House 2, Minimal 3, Techno 9, and Trance 1.
Three low-confidence labels remain audit-only, and the `Unsure` row with
Ambient and Ambient Techno alternatives remains explicitly ambiguous.

The raw `certain` confidence is represented canonically as high and the raw
`none` confidence on the ambiguous row as null. Both original strings remain
in the private verdict artifact. Ingestion added twenty audit records once;
an immediate replay added zero.

B04 also falsified its model-directed acquisition assumption. None of the
twenty accepted primary labels matched the private Tech House, IDM, or
Downtempo sampling strata. The sampling source remains useful as a source of
boundary examples, but it is not efficient enough to drive the next review by
itself.

Ingestion record:

- completed review sheet SHA-256:
  `d5fb97710f05b9ee3f85e649e85572418aced68e7456159e492293af17a36348`;
- private verdict artifact SHA-256:
  `4db8caf6aee5452bbc9ba6f712d652bd34f212ca8c417e4b1f5226861eeacc90`;
- private ledger SHA-256:
  `33ab8330753f257531a1a236a08d60f28cb202d2590e1c1f376991db0bf97050`;
- private snapshot SHA-256:
  `5741aa4e504b4850f6bf81d3c6a0ff93b531af991e1b0a89612735b7aad4aa06`;
- active review records: 46;
- model-eligible review rows: 36;
- model-ready truth fingerprint:
  `f8b6f06370c793de8286e95ef04b9cc9f0465b7c47f067ca627105b2c3072ca9`;
  and
- idempotent replay additions: 0.

The rebuilt combined corpus contains 704 accepted rows. Diversity balancing
places 570 rows in the seven-parent release scope: Ambient, Breakbeat,
Electro, House, Reggae, Techno, and Trance. The artifact replay was
byte-identical and the Plan 066 holdout remains unopened.

Combined-corpus record:

- accepted corpus fingerprint:
  `1d835ac0e91f9aee57c3a2a5d2b311b26fa8a9c575f2aa62f5c0632d8976bc6f`;
- diversity-balanced model-ready fingerprint:
  `2342e75e51db8e3b2127dc5eff1ab41deb1bc347e09f8232025e0da42d3dd4c3`;
- private artifact SHA-256:
  `de3fbdc69ae86f7594dec7f90eeba7758d5c07fa1258f1443686c1f518720c23`;
- byte-identical replay: yes; and
- private artifact mode: 0600.

The nearest unsupported parents are Tech House (4 rows), Minimal (7 rows and
2 artists), Garage (8 rows and 4 artists), Disco (11 rows, 8 artists, and 4
release groups), and Hardcore (11 rows, 6 artists, and 3 release groups).
Training remains prohibited while fewer than twelve parents meet the frozen
support floor.

## Batch B05 preregistration

B05 replaces model-only acquisition with metadata-directed sampling. Current
Rekordbox genre is the first private sampling hint, and the frozen v0.33
recommendation is a fallback. Both are hidden from review and neither is
truth. The selector remains deterministic and fails rather than relaxing a
quota or diversity requirement.

Every accepted development, holdout, or previously reviewed path is excluded.
Previously exposed release groups are also excluded to prevent same-release
near duplicates. Artists are not excluded globally: an artist represented in
another parent may contribute a new track and release. Selection caps any
artist at three tracks, requires one path and release group per selected row, and
prioritizes the artist diversity needed by the target parent.

B05 fixed quotas are:

- Tech House: 6, including two rows beyond its four-row deficit;
- Minimal: 7, with at least two artists new to accepted Minimal truth; and
- Garage: 7, with at least four artists new to accepted Garage truth.

The twenty-row batch is justified by a single coherent support-expansion
question. Its candidate builder is pinned to audit artifact SHA-256
`d0ea2493d0f1eef4d416722ce94e282e4df69c992b99dae0df3777d7eb09501e`
and development-corpus artifact SHA-256
`de3fbdc69ae86f7594dec7f90eeba7758d5c07fa1258f1443686c1f518720c23`.
Freeze and record the candidate-pool checksum before selecting any identity.
The first aggregate feasibility check found only seven untouched,
release-independent Garage candidates. The original eight-row Garage quota
therefore failed closed. Minimal candidates span enough releases but are
concentrated in five artists; a two-track artist cap could select only six.
Before any candidate identity was selected, the final allocation became seven
Garage, seven Minimal, and six Tech House rows with a three-track artist cap.
If all seven Minimal labels are accepted, the most represented added artist can
occupy at most 15% of the resulting twenty-row parent corpus, below the frozen
20% cap.
Minimal is the most artist-constrained stratum and is selected first, followed
by Garage and Tech House. This deterministic order prevents the broader strata
from consuming the artist capacity needed to satisfy Minimal.

## Batch B05 pre-review result

The frozen pool contains 1,434 parent-source candidate variants representing
967 unique tracks after 941 path and 633 release-group exclusions. Relevant
availability is Garage 7 (2 current-tag and 5 v0.33 hints), Minimal 27 (6 and
21), and Tech House 25 (4 and 21). The pool replay was byte-identical.

The selector produced all twenty rows under the final frozen policy: Garage 7,
Minimal 7, and Tech House 6. Ten selections came from current-tag sampling and
ten from v0.33 fallback sampling. The roster contains twenty unique track IDs,
paths, and release groups; the maximum is three tracks from one artist. It adds
five artists not yet represented in accepted Garage truth, four for Minimal,
and six for Tech House if those labels are accepted. Selection replay was
byte-identical.

The exporter validator now reads the declared per-batch artist cap. Older
mappings without that declaration retain the historical one-track default. It
re-read all twenty live identities, required an empty change set before and
after export, and wrote only a playlist XML, blank review sheet, and listening
guide. The XML path multiset exactly matches the private roster, the review
identity columns match it, and all answer cells are blank. All artifacts are
mode 0600.

Reproducibility record:

- final selector-policy commit: `a23367b`;
- declared-cap exporter commit: `3420165`;
- candidate-pool fingerprint:
  `4c208784b95ffbf2b12722baa0af68fe650cfa0c790e2ffc3de2091083540c3e`;
- private candidate-pool SHA-256:
  `f0cfe75dd133d6968c0873f9431b95115ff8d547fef59d4e514164793bb5123a`;
- byte-identical candidate-pool replay: yes;
- roster SHA-256:
  `88b7c04e9650caec1850c25117a7d04486858a6af42a54ea3e309eaf6ddac07a`;
- byte-identical pre-export mapping SHA-256:
  `8acd3e61d289e9ea2af1694f1acad584f2bdb3178b42e0844b6e6d2fc086af8c`;
- exported private mapping SHA-256:
  `4cc76299a8f4cbe3b173835404b5212dc2705c5edfc04bc87ba46ce3c49872a5`;
- playlist XML SHA-256:
  `9e56b8359b770ad4f6ec8a7055f311f0aabbbd85f1643589e5be7edcdb628849`;
- blank review sheet SHA-256:
  `5e37f04aa2413305250555410732fa949ce15939c422e2d355b7813e6bd0026d`;
- blind review guide SHA-256:
  `8bda12e53a359afbf28114150e6e138b22393475cf79e407da81d28262185022`;
- live identity matches: 20; and
- zero staged changes before and after export: yes.

## Batch B05 retirement and B06 preregistration

B05 was retired before operator import or listening because an agent progress
update disclosed its aggregate sampling targets. The review sheet itself
remained label-blind, but the surrounding disclosure violated the stricter
operator-blind contract. No B05 verdict was prepared or ingested. Its pool,
mapping, XML, blank review sheet, and guide were moved from the export directory
to the private `retired-b05` audit directory with mode 0600 and remain excluded
from later selection.

Future sampling quotas are stored in a private, schema-validated config rather
than public progress text. The generic selector validates the batch/pool ID
pair, canonical parents, quotas, new-parent artist minima, stratum order,
twenty-row ceiling, and artist cap. It records the config checksum in the
private mapping. The B06 config was frozen before pool construction or identity
selection, is mode 0600, and has SHA-256
`87d2a0725b82e90f07bd8a9e053ee9faa8daaf70101cb1e4de35b81908e9504c`.
Its target allocation remains hidden until the operator completes the blind
review.

## Batch B06 pre-review result

B06 excludes all accepted, holdout, prior-review, and retired-B05 paths and
release groups. Its pool contains 1,340 private parent-source variants over 913
unique tracks. The pool and selection replays were byte-identical. The selected
roster contains twenty unique track IDs, paths, and release groups, respects
the private config's artist cap and diversity minima, and records the exact
private config checksum. Sampling targets and source composition remain hidden
until review completion.

The exporter re-read and matched all twenty live identities, required an empty
change set before and after export, and wrote only a playlist XML, blank review
sheet, and listening guide. The XML path multiset and review identity columns
exactly match the private roster, and every answer cell is blank. All private
artifacts are mode 0600.

Reproducibility record:

- private selection-config SHA-256:
  `87d2a0725b82e90f07bd8a9e053ee9faa8daaf70101cb1e4de35b81908e9504c`;
- candidate-pool fingerprint:
  `7433a4494fe9f2b4b69530fb43bf9064d40ca5aa50dc72d81cb12c535fc48bdd`;
- private candidate-pool SHA-256:
  `b1280b274eb95de27fa470dd03846160a3fc8c2d04c80638a42b6c9678d265f8`;
- byte-identical candidate-pool replay: yes;
- roster SHA-256:
  `29ad9aaffb451dfd36c19d7ba1c9809eb15637b24b554ee938e54babe325a9b3`;
- byte-identical pre-export mapping SHA-256:
  `272332815d1556379a1c3fe3f093f7c4646327466a6fdd7b12ae1bee2c3fe40a`;
- exported private mapping SHA-256:
  `3336047e64f33ed67d431c0a028f501855f811da000d4d8cea844499947dc543`;
- playlist XML SHA-256:
  `e34d65afca6d34edbd9c34b937a82a2a3006efe320f023248574b1eb5c90bd54`;
- blank review sheet SHA-256:
  `6c1abda3b17c7e6fbffad3b1fbb41e646156526828684fca1b780c2d513e4854`;
- blind review guide SHA-256:
  `dcc48a0e2490c95d4426eedd77f72170e26ddd46aa310bfe765e6ac64ca3406a`;
- live identity matches: 20; and
- zero staged changes before and after export: yes.

## Batch B06 review transcription

The operator supplied all twenty verdicts in prose in exact review order. They
were transcribed into the private TSV without changing identity fields. Fine
labels such as Jungle, Deep Techno, and Hard Techno remain raw while canonical
training labels use the frozen parent mapping. The operator's final House
decision for `5AM (Vocal)` supersedes the earlier Electro thought within the
same response. No unsupported alternative was inferred for `EZ Ryder`.

The strict converter now preserves and recognizes the natural confidence terms
`very low` and `low-medium`, normalizing both conservatively to low, and accepts
the composite ambiguous wording `unsure/ambiguous`. The operator then clarified
that the three Jungle-versus-Drum & Bass rows are high confidence at their
shared Drum & Bass parent. Their raw subtype uncertainty remains in notes and
does not weaken the parent verdict.

## Batch B06 ingestion and acquisition result

The final converter produced nineteen labels and one ambiguous outcome. Twelve
rows are model-eligible: Drum & Bass 6, House 2, Tech House 1, and Techno 3.
Seven low-confidence labels and the ambiguous row remain audit-only. Ingestion
added all twenty records once; an immediate replay added zero.

The completed review also exposes the sampling mechanism's limits. Of thirteen
rows sampled through the Drum & Bass target, eleven resolved to Drum & Bass,
one to Techno, and one was ambiguous. Of seven rows sampled through Tech House,
only one resolved to Tech House; three resolved to House and three to Techno.
Current Rekordbox genre supplied most of the useful Drum & Bass candidates,
while the v0.33 recommendation was not an efficient source of Tech House truth.
Another broad listening batch from the same candidate pool is therefore not a
good use of operator attention.

Ingestion record:

- completed review sheet SHA-256:
  `cb415486aaa606c66e3d2b32ada32d12658f7681dc84337f2413823b004c1a1c`;
- private verdict artifact SHA-256:
  `5a26379a1790a0f563cb1e84ea6f525764596bfbff6681f5ce11bf1f90997762`;
- private ledger SHA-256:
  `d3e725ac67891c2dc70004326d1a7e17877f3b8e2089c7fd49ec399b11c1119c`;
- private snapshot SHA-256:
  `30816114581b624a515aaeeaac6e0f551e7263a2811c616871365eaa272b7ce7`;
- active review records: 66;
- model-eligible review rows: 48;
- model-ready truth fingerprint:
  `b2ad3596115b2838372e1a292f3e96f82431678ba25096e9dc236397a52dc8ce`;
  and
- idempotent replay additions: 0.

The rebuilt corpus has 716 accepted rows. Diversity balancing places 575 rows
in seven supported parents: House, Ambient, Techno, Breakbeat, Reggae,
Electro, and Trance. That scope covers 80.31% of accepted truth before model
abstention, so it passes the revised seven-parent and 75% accepted-coverage
support gate. Every unsupported parent remains available for later active
learning and is a mandatory abstention in whole-corpus diagnostics.

Combined-corpus record:

- accepted corpus fingerprint:
  `07a754c42ae676eb7f6fcbc02ee1b5748e3153e155311d4559d3d749fbdd6cf1`;
- diversity-balanced model-ready fingerprint:
  `e1dfdf5006c5e214f6f6baeeb9d8d5e5a4a890fad69ff6e39fbc877ba6f8e1b8`;
- pre-contract-update private artifact SHA-256:
  `b272f0c868e090d427920cac42035a36b9fa2b211c58250d314e908ba804f8ac`;
- revised-support-contract private artifact SHA-256:
  `0e57411a6692bf0c66201fcd71c9919bb4f84a60cd6339f37e6bd95365b79fa1`;
- byte-identical replay: yes; and
- private artifact mode: 0600.

## Frozen candidate A: hydrated production evidence

Before extracting features or scoring a model, freeze one candidate using only
evidence already required by the product's full classification mode:

1. thirteen profile scalars in production order, substituting the fresh
   Stratum BPM for the live Rekordbox BPM so extraction remains cache-only;
2. MFCC mean coefficients 1-8, MFCC standard-deviation coefficients 1-5, and
   spectral-contrast bands 0, 2, and 4;
3. the frozen 74-value Plan 065 kick vector; and
4. an eight-value one-hot of the v0.33 recommendation mapped to the seven
   release parents plus unknown.

Each of the 29 scalar/timbral values has a paired missingness indicator. Value
imputation, standardization, and feature-column filtering occur inside each
training partition. The kick vector retains its own availability field. The
candidate does not use current Rekordbox genre, artist, release, file identity,
provider metadata, fine truth, or unsupported-parent identity as a feature.

Assign artists deterministically to five folds with a fixed hash tie-break and
a greedy objective that minimizes per-parent and total fold imbalance. Grouping
by normalized artist is stricter than release grouping and therefore keeps
every same-artist release together. Require every release parent in every fold
and fail without relaxing isolation.

Fit exactly the Plan 066 class-balanced one-versus-rest ridge head with penalty
10, an unpenalized intercept, and top-score-minus-second-score confidence. Use
nested inner out-of-fold threshold calibration inside every outer fold and the
unchanged minimum-offer rule. Also select one global deployment threshold from
all outer out-of-fold margins at 90% precision with at least 60 offers. The
nested and global views must both pass the frozen development gate. The
five-point paired improvement over v0.33 is measured only where both systems
have a mapped recommendation; missing baseline rows are reported and excluded
from that comparison, not counted in the candidate's favour.

No penalty, feature subset, fold, target-specific threshold, support rule, or
parent mapping changes after candidate A is scored. A failure retires this
exact recipe. The remaining two candidate-family slots stay unused until a
new representation question is preregistered from the observed error structure.

## Candidate A pre-score freeze

The candidate harness was committed before scoring. The private development
and label-blind feature manifests contain the same 575 stable row IDs in the
same order. The five deterministic folds contain exactly 115 rows each, every
fold contains all seven targets, and no normalized artist or release group
crosses a fold. The v0.33 recommendation inputs come from the frozen Plan 060
and Plan 066 evaluation manifests, where current genre was cleared before
classification; current Rekordbox tags therefore remain absent from candidate
features.

Label-blind extraction read only fresh Stratum 21 and Essentia 3 cache rows. It
found all 29 scalar/timbral values for all 575 rows, kick evidence for 573, and
a mapped release-scope v0.33 recommendation for 453. Unknown or out-of-scope
recommendations use the single frozen unknown indicator. A second extraction
produced byte-identical feature and summary artifacts.

Pre-score record:

- harness implementation commit: `0e008fa`;
- preparation source SHA-256:
  `6143a8b3fb949c814ceef7e2eed86ace4f64d947fa3f0c18cc51f042a28378a8`;
- feature-extractor source SHA-256:
  `832c04f0a640da57e933a2f81399f5c74de00494ac85997edc374f88ba24b3cc`;
- evaluator source SHA-256:
  `126737579981d82911842af9c647d794fe17834c52e074a269cfb0207cbf5d17`;
- private development-manifest SHA-256:
  `caf76dbe8156943a139a8ab73e8d8b492a1d74bfe1b1e9c80898104ff21f5580`;
- private label-blind feature-manifest SHA-256:
  `d50519a80812a8f5705a8db834ca2764618f0fde18d3ce99ad8e981724c60e24`;
- private feature-artifact SHA-256:
  `e93610e70ad70b6c02640a7161a0bc5b444717bea2b6a521c526a601db7b72ab`;
- private feature-summary SHA-256:
  `5e8769b4fe435214dfe12c8f16f7da5dcbd17e406f0120e970ef724ee6d05d61`;
- feature semantic SHA-256:
  `aca9bc0082a96d3414ea86cb2945e742b5852ed6a1433f75028d54ab91a1e42b`;
- ordered cache-source snapshot SHA-256:
  `35aff9c0b1ec3dcc86210d5bff1ce7435d6ff594438d8ac3fd5487fe0d908d5b`;
- private candidate-config SHA-256:
  `d532a4011665d2f13bc77b1b951cc00fc772b2d7d4a83cba851bb0dcad9f2500`;
- byte-identical manifest preparation and feature extraction: yes; and
- every private artifact mode: 0600.

## Candidate A result

Candidate A is a bounded negative. It crossed the 90% aggregate precision
floor in both views and every outer fold remained above 85%, but selectivity
was much too severe: nested coverage was 38.09% and deployment-threshold
coverage was 41.91%, both far below 65%. Whole-accepted-truth coverage was
30.59% and 33.66% respectively.

The nested view offered 219 rows at 91.32% precision. Techno was below its
target guard at 78.38% on 37 offers. The global threshold offered 241 rows at
90.04%; Techno fell to 76.74% on 43 offers and Trance to 69.23% on 13. The
candidate improved over v0.33 by 4.48 points in the nested paired comparison
and 4.59 points in the deployment comparison, narrowly below the frozen five-
point requirement. Its high aggregate precision is real, but it does not offer
enough useful work and remains weak at important dance-music boundaries.

The exact evaluator replay was byte-identical. No threshold, penalty, feature,
fold, or gate changed after inspection. The sealed release holdout remains
unopened.

Result record:

- private result SHA-256:
  `006aaf5e9e1a6a0928f0c42777d764e808b3a6e96c851431d2188ec958669044`;
- v0.33 mapped-parent baseline: 61.59% precision at 78.78% coverage;
- candidate unselective accuracy: 65.91%;
- nested result: 91.32% precision at 38.09% scope coverage;
- deployment result: 90.04% precision at 41.91% scope coverage;
- result replay: byte-identical; and
- private artifact mode: 0600.

## Frozen candidate B: OpenL3 representation

Candidate A shows that the hydrated production descriptors can identify an
easy high-precision subset but do not encode enough musical structure for the
required coverage. Candidate B asks one new representation question. Append a
training-partition PCA64 projection of the frozen OpenL3 embedding to candidate
A's unchanged 140 features. Retain the exact folds, class-balanced ridge head,
penalty 10, nested and global threshold calibration, v0.33 paired comparison,
and every gate. No feature is removed and OpenL3 is not combined with CLAP.

Use the unchanged Plan 066 label-blind extractor and model contract:

- `openl3-music-mel128-emb512-3.onnx`;
- model SHA-256
  `81c24c8a723054717fdea5c7448acb6023baaf70a0fc526deb030c2032db0ed3`;
- 48 kHz mono decode;
- twelve deterministic one-second excerpts across the full track;
- official 128-band Slaney-mel preprocessing;
- patch and final-track L2 normalization; and
- extractor source SHA-256
  `5d09431f18e77320a8f0c77b0af393cce4ea8979275cfb6adc2b7b5f44fd7e5c`.

OpenL3 is evaluated before CLAP because its 18.7 MB CC BY model and CPU runtime
are materially easier to integrate into a user-facing release. Extraction is
label-blind and reads audio only. Freeze the representation manifest, artifact,
summary, extended evaluator source, and candidate config before scoring. A
failure retires this exact candidate and leaves one candidate-family slot.

## Candidate B pre-score freeze

The OpenL3 harness was committed before representation extraction. The pinned
model was retrieved from the recorded official Essentia model URL and matched
the historical SHA-256 exactly. The managed runtime used NumPy 2.5.1,
Essentia 2.1-beta6-dev, and ONNX Runtime 1.28.0, matching the Plan 066 runtime.

All 575 rows completed without a skip or retry. The persistent per-row work
matrix and decoded-source hashes live under the private research directory, so
the extraction is resumable rather than dependent on temporary storage. A
second finalization from that frozen matrix reproduced the feature and summary
artifacts byte-for-byte. The candidate config now pins those outputs and every
base input before the first genre-conditioned score.

Pre-score record:

- harness implementation commit: `603067d`;
- representation-manifest preparer SHA-256:
  `1755ccf10e047f6bc66bca0141800e5091d6287d5c5e24e5fb82c88591f68dde`;
- OpenL3 evaluator source SHA-256:
  `d404c19941aa4822a05bd72a7c2a14d640fb156b127b67869b2e9228f401862a`;
- private representation-manifest SHA-256:
  `676de6f150a811494f255a667d61fb449c149a9cfbf5fa968a74327e2afd0e67`;
- OpenL3 feature-artifact SHA-256:
  `e9bad78b70f9d0b3a59d7ace1fb1a36ed4521c180d5eb34c1336627eb9e10755`;
- OpenL3 extraction-summary SHA-256:
  `74ead41c6569bc9205ab7adb002940126f683c1c84694a216cc568da2d504413`;
- ordered decoded-source SHA-256:
  `8c3988201f77a1c229e3aa6f3c14289e9b3e6c2761768cd0343559f4b97c70f6`;
- private candidate-config SHA-256:
  `d46d63fa7e33c2af9a8fd7a06d2fb689be3685db5bee35ae6c92776686eaffbf`;
- byte-identical representation-manifest preparation and feature finalization:
  yes; and
- every identity-bearing artifact mode: 0600.

## Done criteria

This plan is complete only when either:

1. a candidate passes development and independent release gates, the complete
   preview-first classifier and active-review loop are integrated and verified,
   and Genre Intelligence V1 is prepared for release; or
2. the bounded, sufficiently supported experiments fail, their negative result
   is documented, and the best-supported next direction is recorded without
   contaminating remaining evaluation data.
