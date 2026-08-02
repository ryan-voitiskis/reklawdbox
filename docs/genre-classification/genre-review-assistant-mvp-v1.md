# Genre Review Assistant MVP V1

**Run date:** 2026-08-02

**Status:** Pilot complete; genre decisions passed and utility was not
demonstrated

## Purpose

This research MVP tests whether verified reference comparisons and neutral
audio vocabulary make manual genre decisions easier. It does not test a hidden
classifier target and does not authorize automatic genre changes.

The assistant always presents the current genre as context, then up to two
alternative listening hints derived from proximity to ear-verified references.
Every hint includes concrete references. Cached analysis contributes only
relative descriptions of tempo, event density, spectral motion, and dynamics.

## Safety boundary

- The selector reused frozen Plan 059 and Plan 060 artifacts; it did not decode
  or analyze audio.
- Rekordbox, audio files, tags, analysis caches, and stored profiles were read
  only.
- Embedding proximity is labelled as a listening hint, not probability,
  confidence, or truth.
- The playlist export staged no metadata and applied no changes.
- A later metadata XML may contain only operator-approved verdicts.
- Private identities, paths, similarities, hints, and verdicts remain outside
  Git.

## Frozen selection result

| Stage | Rows |
|---|---:|
| Frozen Plan 060 candidates | 1,612 |
| Newly exposed Plan 060 rows excluded | 6 |
| Eligible confirmation-first candidates | 1,529 |
| Usable ear-verified references | 670 |
| Distinct current genres selected | 6 |
| Private review rows exported | 6 |

The six rows have distinct genres and artist/release groups. No more than two
selected genres belong to one taxonomy family. Related collaborators are
excluded from reference comparisons, and displayed references are artist- and
release-diverse.

## Review protocol

Import the `genre_review_assistant_v1` playlist and use the private guide while
listening. For each row record:

- `verified`, a canonical replacement genre, `ambiguous`, or `skip`;
- confidence;
- plausible alternatives and notes;
- whether the verified references helped; and
- whether the vocabulary helped describe the sound.

The interaction advances only if at least four rows receive confident genre
decisions and at least four report material help from references or vocabulary.
Ambiguous and skipped rows are valid. There is no hidden target to match.

## Verification

- nine deterministic unit tests cover artifact parsing, alias resolution,
  embedding validation, collaborator exclusion, hint bounds, vocabulary,
  diversity, and failure without quota relaxation;
- all 670 usable development rows matched the imported reference XML by path
  and canonical genre;
- a pre-export replay was byte-identical;
- all six live identities and current canonical genres matched at export time;
- XML is well formed with six collection rows and six playlist references; and
- `preview_changes` was empty before and after `write_xml`, which reported
  `changes_applied: 0`.

## Reproducibility

- roster SHA-256:
  `eb7d5dba363ab2283c04f0bfcc0a74ae2906044dd62ffc46d5c616a74960e70d`
- byte-identical pre-export private result SHA-256:
  `8d84db22a894aa818ec26e3a13e5902e237999ad30b835bb81eb3c1bc7e73a00`
- playlist XML SHA-256:
  `c3732070e61a85faa91aea006d8fe59400fc73e4c739c159feeddafc68258d11`
- private review sheet SHA-256 after genre verdicts:
  `52f9ce90b6755933d55d48ee2a7a4a92345fdd09ab51b2762805121e9c165aba`
- private review guide SHA-256:
  `e9d3998a281c866228a286e7c964d54db6decfad99f8f50c8397177c18ff4afc`

## Final pilot result

All six current canonical genres received confident operator verification.
There were no replacement genres, ambiguous verdicts, or skipped rows. The
genre-decision criterion passed 6/6; the required threshold was 4/6.

The operator did not need additional information for certainty on four rows.
On the two rows requiring more listening, the operator was unsure whether the
references or vocabulary materially helped. No row received a positive utility
report, so the 4/6 utility criterion was not demonstrated and the overall pilot
gate did not pass.

This does not weaken the genre verdicts. It shows that a confirmation-first
batch does not establish the value of this assistance design. A future utility
experiment, if pursued, should use a small assisted-versus-unassisted
comparison on genuinely ambiguous decisions rather than more blind accuracy
review.

No hint was treated as automated truth, and no Rekordbox metadata, audio, tags,
or caches were changed.
