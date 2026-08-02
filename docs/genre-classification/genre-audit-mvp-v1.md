# Genre Audit MVP V1

**Run date:** 2026-08-02

**Status:** Blind six-track pilot ready for operator review

## Purpose

This experiment tests whether consensus among three imperfect signals can
prioritize likely Rekordbox genre errors for human review. It is a retrieval
tool, not an autonomous classifier.

The signals are the existing classifier with current genre removed, direct
Discogs-EffNet style projection, and the frozen 70/20/10 EffNet fusion fitted on
the complete Plan 059 development corpus. A row is ranked only when all three
recommend the same sufficiently supported, sufficiently precise target and that
target differs from the current canonical genre.

## Read-only boundary

- Rekordbox, audio, tags, and caches were read only.
- Stored profiles were not loaded or written.
- The export contains a playlist only and preserves live track metadata.
- No model recommendation was written to a genre field.
- Private identities, paths, scores, predictions, targets, cohorts, and review
  mappings remain outside Git.
- All eight prior truth or listening-candidate playlists were excluded.

## Universe and selection

| Stage | Rows |
|---|---:|
| Non-sample library | 3,129 |
| Unique prior truth/review exclusions | 827 |
| Existing-file candidates before genre/audio checks | 2,299 |
| Canonical current genre | 1,641 |
| Fresh, valid, scorable analysis | 1,612 |
| Three-way mismatch candidates | 14 |
| Three-way confirmation controls | 17 |
| Blind ranked rows selected | 4 |
| Blind controls selected | 2 |

Ten target genres met the frozen development-support and held-out-precision
floor. The selected ranked rows have distinct targets and artist/release groups,
and no more than two targets share one classifier family. Controls match two
distinct ranked targets by closest BPM while remaining artist/release distinct.

## Verification

- all 1,612 audio files decoded for the frozen 12-patch frontend;
- an exact inference replay produced the same feature and roster hashes;
- six selected live IDs, paths, artists, and titles matched at export time;
- XML is well formed with six collection rows and six playlist references;
- the blind sheet has six rows and contains no target, current genre, cohort, or
  model score; and
- no changes were staged before or after `write_xml`.

## Review protocol

Import the `genre_audit_blind_v1` playlist and hide Rekordbox's Genre column.
Review all six rows before asking to reveal the mapping. For each row record:

- the genre heard, or `ambiguous`;
- confidence;
- plausible alternatives; and
- a short sound description if useful.

The pilot advances only if at least two of four ranked rows confidently match
their hidden targets and no more than one of two controls confidently
contradicts its current genre. Ambiguous rows remain boundary evidence and do
not become truth.

## Reproducibility

- candidate feature SHA-256:
  `49c5e57aea256cb9a721d9a4215410511e725aaa2f3b8abcfbcb2b10308ca9a1`
- roster SHA-256:
  `4dacf6e1204db9cb11544cd850450da000525f3c08a91bc128ab47ac10c43060`
- playlist XML SHA-256:
  `3ad9e7e69ffeee2411a8941e95fba56aa5006a60763f785a40612582c1524213`
- review sheet SHA-256:
  `7c4d6344106d458a93cc9bfb7d04ffa3891b94912d9bc75629084b540ee42ac7`

The exact private mapping stays sealed until the six verdicts are complete.
