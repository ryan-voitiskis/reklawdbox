# Plan 073: Validate a precision-first parent-genre preview

> **Status:** Awaiting prediction-blind human verdicts; all machine outputs and
> scoring rules are sealed
> **Objective:** Determine whether the already-frozen O3 model can ship as a
> useful precision-first preview at approximately 20% collection coverage,
> using enough independent offers to evaluate its 90% precision claim.

## Why this is a separate release question

Plan 072's O3 candidate passed every development gate at 27.51% nested coverage
and 96.45% exact precision. On its independently sealed 60-row holdout, it
offered twelve rows, or 20% coverage. That valid result failed the original
requirement of thirty offers from sixty rows before human truth was needed.

The original gate combined two different requirements: 50% product coverage
and at least thirty labels for a useful precision estimate. O3 plainly does not
meet the former. Twelve unknown labels also cannot establish the latter. The
model may still be useful as a selective preview over roughly one-fifth of a
collection, especially against v0.33's 56.95% development precision, but that
is a narrower product claim and needs a newly powered independent test.

This plan does not retune O3 or reinterpret the consumed 60-row result. It
freezes the lower-coverage product claim before selecting a new holdout and
uses 150 rows so 20% coverage yields thirty blind offers. The exact-primary-
parent precision bar stays at 90% and gains an explicit paired v0.33
improvement gate.

## Frozen candidate

Use O3 without any change:

- Plan 072 development result SHA-256:
  `9d1683960d2ace05aa553965e4bf486c46df509eb9147ed0d202eff02ec6eb5d`;
- full-fit model SHA-256:
  `2633db33edc2e2af4e3e42adae9cea945f4453f7524522c4f4fecca8530f30df`;
- inference implementation source SHA-256:
  `9d1675acdb0cf5bb532bcf7763a628f4a14bfff7caeb9d1320ca9d087672746f`;
- seven independent class-balanced binary ridge models, penalty 10;
- unchanged 140 cache-native/v0.33 features plus training-only CLAP PCA64;
- unchanged 95%-precision deployment thresholds;
- House, Ambient, Techno, Reggae, and Electro enabled;
- Breakbeat and Trance disabled; and
- zero- and multi-qualified rows abstain.

The first successful Plan 072 prediction artifact at SHA-256
`4e5c4d6402638c7e5f5331559576d1347210caeb005920020c7947ed6c4a2527`
is consumed. Do not inspect its suggestions or identities, review its twelve
offers, add them to development, or compare a new candidate against it.

## New 150-row holdout

Seal a new identity-only roster before extracting a feature. Use the complete
live Rekordbox collection rather than the earlier audit-manifest subset. Current
genre may be used only as a hidden sampling stratum and is never truth.

Exclude every path, normalized artist, and release group present in:

- the 716-row development corpus;
- the consumed Plan 066/070 holdout;
- the consumed Plan 071/072 holdout; and
- `genre_verified` or any earlier genre research/review playlist.

Also exclude missing files, blank artists, duplicate paths, unmapped genres,
and `Experimental`. Select one row per normalized artist and one per normalized
artist-release group using a new fixed seed. Attempt these hidden-stratum
quotas in scarcity-first order, then fill any shortfall from all remaining
eligible rows in fixed-seed order:

| Hidden sampling stratum | Desired rows |
| ----------------------- | -----------: |
| Breakbeat               |           15 |
| Disco                   |            1 |
| House                   |           60 |
| Minimal                 |            1 |
| Tech House              |            1 |
| Techno                  |           72 |

The quotas reflect the aggregate untouched collection available before this
plan: 335 rows across 265 artists and 291 releases, heavily concentrated in
House and Techno. They are a sampling record, not expected truth. Stop if the
deterministic selector cannot produce 150 unique artists and releases. Keep
identities and sampling fields private at mode 0600 and log only aggregate
counts.

### Holdout seal record

The selector and tests were committed as `a0b43ad` before the first live
selection. Its source SHA-256 was
`fda0b24f5ad9f46b11dd698db437cc94ee6cd987689b23d7cdc82aeae1f8aa52`.
Selection and an immediate replay produced byte-identical mode-0600 artifacts:

- holdout artifact SHA-256:
  `cecaa886b20ce5262353e8465c505be1d5680f735558a9f96f1226852afa17f0`;
- private roster fingerprint:
  `14b4d0420900849f6258b7345417a4003bcb7f900ebb69b448b36ed38fa4c464`;
- universe: 3,137 live rows, 335 eligible rows, 265 eligible artists, and 291
  eligible release groups; and
- hidden sampling counts: Breakbeat 15, Disco 1, House 60, Minimal 1, Tech
  House 1, and Techno 72.

All 150 selected rows have unique normalized artists and release groups. No
feature, embedding, prediction, or identity value was exposed during
selection.

## Isolation, inference, and review

Before inference, verify and replay:

1. zero path, artist, release, accepted-truth, prior-holdout, and research-
   playlist overlap;
2. zero decoded-audio overlap with all 716 development rows and both consumed
   holdouts;
3. complete Stratum 21 and Essentia 3 cache evidence for every selected file;
   and
4. byte-identical cache-native and CLAP artifacts from label-blind manifests.

Bind the new artifacts to the unchanged O3 source, model formulation,
thresholds, and development result. Fit the full model again from the frozen
716 rows and require its serialized artifact to match the Plan 072 model
byte-for-byte. Freeze and replay all 150 predictions before exposing a track.

If fewer than thirty rows receive exactly one qualifying parent, stop with a
coverage failure and do not request listening. Otherwise partition every offer
in fixed-seed, prediction-blind batches of at most six. Review materials may
contain only opaque code, artist, title, album, and audio location. They must
not contain the model suggestion, scores, margins, thresholds, current genre,
sampling stratum, or v0.33 result.

Freeze all human verdicts before the first prediction join. Use exact canonical
parent as primary truth; preserve uncertainty, alternatives, and listening
notes separately.

### Isolation and feature seal

The input preparation, decoded-audio audit, cache-native extraction, and CLAP
extraction all replayed byte-for-byte. No identity value or model output was
exposed. The sealed evidence is:

- holdout feature-manifest SHA-256:
  `a324393657531919df7b143b400c329e5b74288aebdb0eaae4871457169f9380`;
- holdout representation-manifest SHA-256:
  `aa717ac333772395902cf4b376fca3d2f3a1c8c138357092f2f0306feec2858a`;
- holdout input-summary SHA-256:
  `9bdc70094721f827ae4b461ced24f96227ce3a5377bc414e6a916bf981e8d2a3`;
- decoded-audio audit and replay SHA-256:
  `3b11b896511c0571aec64f23f4ed7711fb11b5738d0cbb95b12609374168b3ac`;
- cache-native feature artifact SHA-256:
  `369050fb519340a888794965f8ff41aa2e69bb14af2490043f43921df1cd351a`;
- CLAP feature artifact SHA-256:
  `c137bce90faef12683644d03c05adb5dc40387fccba5ad24da4b10e82f962f3f`;
  and
- ordered decoded-audio/source fingerprint:
  `5630dd0df566ae152531c7eeacaf84b589e9e179678c6610f9acdbad3d5b1990`.

Every one of the 150 holdout rows had complete cache evidence. The holdout had
zero path, normalized-artist, release, accepted-truth, research-playlist, prior-
holdout, and decoded-audio overlap. Its decoded audio was also unique within
the holdout. The comparison covered all 716 development rows and both earlier
60-row consumed holdouts.

### Frozen inference result

The inference implementation was committed as `55ba210` before its first run.
The sealed configuration at SHA-256
`486659cd239055f2f948d75cff2ebcf32bf59215c3ebcb3b9a0f37a88e8bbe55`
re-fitted a model byte-identical to the Plan 072 artifact at SHA-256
`2633db33edc2e2af4e3e42adae9cea945f4453f7524522c4f4fecca8530f30df`.
Its predictions and a clean replay were byte-identical at SHA-256
`0fae375e636631cac88a6762f3c64ed69caf50b602a737fde61d5b122034e1ff`.

Exactly 35 of 150 rows received one qualifying parent: 23.33% coverage, 115
abstentions, and no multi-qualified collision. This clears the frozen pre-
listening requirements of at least thirty offers and at least 20% coverage.
Suggestions, scores, current tags, sampling strata, and v0.33 labels remain
sealed.

### Blind review seal

The review partitioner was committed as `fdeccd7` before reading the private
identity roster. It partitioned every offer exactly once into batches of
6/6/6/6/6/5. The mode-0600 review manifest has SHA-256
`7f8c84c706f50638533b62e5478cdab0ccd88caf3fd96cdc6c9391afe37e5993`.
Each mapping contains only the fields required to identify and export a track;
the generated TSVs and guides contain only opaque code, artist, title, and
blank human-answer fields.

A single Rekordbox XML contains the six independently named playlists. Its
collection records were deliberately reduced to TrackID, name, artist, album,
and audio location so existing genre, comments, ratings, and other metadata
cannot bias the review. The export verified all 35 identities against the live
read-only Rekordbox library, left zero staged changes, and produced XML
SHA-256 `99622dc153c5a378223a8c3664f9fc83c9fb1f1169afb5b862da26d93e970219`.

The exact-primary-parent evaluator and its boundary tests were committed as
`842f690` before any listening. Its source SHA-256 is
`4266651f12280f9d7282963730e46362fe0e31cac810bdcac25f5399615c6a6a`.
It freezes the aggregate, per-emitted-parent, paired-v0.33, uncertainty, and
isolation rules below. It cannot run until all six human verdict files have
been sealed.

## Release gate

The precision-first preview passes only if all of these hold:

- at least 30 of 150 rows receive offers;
- offer coverage is at least 20%;
- aggregate exact-primary-parent offered precision is at least 90%;
- every emitted parent with at least five offers reaches at least 80%
  precision;
- on rows where both O3 and v0.33 offer, O3 precision exceeds v0.33 by at least
  five percentage points;
- identity and decoded-audio isolation pass; and
- feature extraction, model fitting, inference, and evaluation replay byte-
  for-byte.

If the candidate passes, it authorizes a read-only, explicitly experimental
precision-first preview whose product contract states that most rows abstain.
It does not authorize automatic metadata writes, a claim of full-collection
classification, support for disabled parents, or machine-learning expansion
beyond the frozen model. If it fails, retire O3 and keep the product unchanged.
