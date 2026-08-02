# Plan 062: Evaluate release-grade broad genre classification

> **Status:** Complete on 2026-08-02; deterministic parent consensus is a
> bounded negative
> **Objective:** Determine whether Reklawdbox can offer materially more useful
> broad-genre suggestions than v0.33 without adding a machine-learning runtime
> or forcing unreliable fine labels.

## Product question

Can the existing evidence pipeline produce a broad genre suggestion with at
least 90% precision at useful coverage, while abstaining on weak or
cross-parent evidence?

This is a selective-classification problem. A correct abstention is preferable
to an unsupported label. Fine-genre accuracy is diagnostic only and does not
gate this plan.

## Separate taxonomy contract

`GenreFamily` remains an internal mixing and decision-tree concept. It is not a
public parent taxonomy: it currently groups Disco and Garage with House,
Electro and Trance with Techno, Breakbeat and Drum & Bass as Bass, and IDM and
Dub as Downtempo.

This plan instead freezes a conservative broad mapping. It collapses only
clear subgenre lineages. Cross-cutting, disputed, or already-broad canonical
genres retain themselves rather than being forced under a misleading parent.
`Experimental` remains an anti-genre and has no modeled broad target.

| Broad target | Canonical fine genres                                                                                               |
| ------------ | ------------------------------------------------------------------------------------------------------------------- |
| House        | Afro House; Deep House; Gospel House; House; Progressive House                                                      |
| Techno       | Ambient Techno; Deep Techno; Dub Techno; Hard Techno; Techno                                                        |
| Trance       | Hard Trance; Psytrance; Trance                                                                                      |
| Garage       | 2-Step Garage; Bassline; Future Garage; Garage; Speed Garage; UK Funky                                              |
| Breakbeat    | Breakbeat; Broken Beat                                                                                              |
| Drum & Bass  | Drum & Bass; Jungle                                                                                                 |
| Reggae       | Dancehall; Dub; Reggae                                                                                              |
| Disco        | Disco; Italo Disco                                                                                                  |
| Hardcore     | Gabber; Happy Hardcore; Hardcore; Hardstyle                                                                         |
| Downtempo    | Downtempo; Trip-Hop                                                                                                 |
| Pop          | Italodance; Pop; Synth-pop                                                                                          |
| Self-parent  | Acid; Ambient; Dubstep; EBM; Electro; Footwork; Grime; Highlife; Hip Hop; IDM; Jazz; Minimal; R&B; Rock; Tech House |
| Unmodeled    | Experimental                                                                                                        |

The mapping is an operational classification contract, not a claim that every
genre has only one historical parent. It may be revised only in a later plan
using independent evidence; it must not be changed after seeing this plan's
metrics.

## Frozen deterministic configurations

Evaluate the current classifier with stored profiles disabled and the current
Rekordbox genre removed from every input.

### Diagnostic A: unselective projection

Map every v0.33 fine recommendation to its frozen broad target. Preserve an
existing abstention. This measures how much broad accuracy arises from label
contraction alone; it is not a promotion candidate.

### Diagnostic B: current confident projection

Offer the mapped broad target only when v0.33 returns High or Medium confidence
in Full mode. Otherwise abstain. This is the current product's closest honest
broad baseline.

### Candidate: parent consensus

Start from the same unprofiled classification result:

1. Require Full mode and a mapped final recommendation.
2. Map every exposed fine candidate to its broad target. If any candidate is
   unmapped or belongs to a different broad target from the final
   recommendation, abstain.
3. If no fine candidates are exposed, offer only a High or Medium final
   recommendation. This preserves audio-veto results without promoting weak
   audio-only guesses.
4. If one fine candidate is exposed, offer only at High or Medium confidence.
5. If at least two distinct fine candidates are exposed and all map to the same
   broad target, offer that target at any existing confidence. Fine-label
   disagreement has then become broad-label agreement.

Do not add score thresholds, genre exceptions, BPM exceptions, new audio
rules, stored profiles, or current-genre tie-breaks after seeing results.

## Development corpus and metrics

Reuse the frozen 670-row usable `genre_verified` development corpus and its
existing five artist/release/related-version-isolated folds. Exclude any row
whose truth has no broad target. This corpus is exposed development truth and
can nominate a holdout candidate but cannot validate a release.

For each configuration report only aggregate data:

- eligible rows, offers, abstentions, coverage, correct offers, and offered
  precision;
- overall accuracy with abstentions counted as incorrect;
- macro recall and macro F1 across represented broad targets;
- fold-level coverage and offered precision; and
- per-target support, offers, precision, recall, abstentions, and leading
  confusions.

Private track identities, paths, fold assignments, predictions, and evidence
remain outside Git.

## Pre-registered development gate

The parent-consensus candidate advances only if all conditions pass:

1. offered precision is at least 0.90;
2. coverage is at least 0.50;
3. every fold's offered precision is at least 0.85;
4. every broad target with at least ten truth rows and at least five offers has
   offered precision of at least 0.75;
5. offered precision improves on the unselective projection by at least 0.10;
6. current genre, stored profiles, private row identity, and post-result tuning
   do not influence any decision.

If the candidate fails, record a bounded negative and stop deterministic rule
tuning on this exposed corpus. The next plan may evaluate the already-frozen
Discogs-EffNet representation directly against this unchanged broad mapping,
with a separately frozen selective-confidence rule.

## Sealed holdout boundary

If the development gate passes:

1. freeze the candidate implementation and semantic checksum;
2. select at least 60 previously unexposed collection tracks, isolated from all
   development rows by artist, release, remix, and related version;
3. seal predictions before listening and present only blind broad-genre review
   batches of four to six tracks;
4. accept `broad genre`, `ambiguous`, or `skip` as valid operator answers; and
5. evaluate once, without changing the candidate.

The holdout passes only if at least 30 tracks receive candidate offers, offered
precision is at least 0.90, and no supported broad target shows a material
failure hidden by the aggregate. A failed holdout retires the candidate.

## Release boundary

A passing holdout authorizes a separate production implementation. The initial
surface should:

- return broad and fine recommendations separately;
- state the selected granularity and confidence;
- abstain explicitly;
- default to read-only review;
- stage a broad genre only when the operator explicitly requests it; and
- continue routing every Rekordbox-visible change through `ChangeManager` and
  `write_xml`.

Do not advertise or release Plan 061's reference-comparison assistant as a
classification improvement. Its confirmation-first utility gate did not pass.

## Verification

- Unit-test complete mapping coverage and the permanent `Experimental`
  exclusion.
- Unit-test every parent-consensus branch, including cross-parent conflict,
  unmapped candidates, degraded mode, audio-veto output, and low-confidence
  same-parent agreement.
- Require deterministic aggregate output and a stable semantic checksum.
- Inspect committed output for private identities, paths, predictions, or
  evidence.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

This development plan is complete when the three frozen configurations have
run once and the aggregate result is recorded. It either nominates the frozen
parent-consensus candidate for a sealed holdout or records a bounded negative
and hands the unchanged broad taxonomy to a separately planned ML evaluation.

## Recorded result

The aggregate report is
[Broad Genre Deterministic Evaluation](../docs/genre-classification/broad-genre-deterministic-evaluation.md).
The frozen candidate failed four of its five performance checks:

- offered precision: 82.32%, required 90%;
- coverage: 24.55%, required 50%;
- worst fold offered precision: 51.28%, required 85%; and
- Techno offered precision: 64.15% on 53 offers, required 75% for a supported
  target.

It improved offered precision by 33.67 percentage points over the unselective
projection, so the contraction itself removed some bad fine-label guesses. It
did not make v0.33 release-grade: it abstained more often than the existing
High/Medium filter while remaining materially below the precision target.

The live corpus contained 668 usable rows rather than the frozen 670. A private
identity comparison found two absent audio files, one previously verified as
Electro and one as Jungle, with no additions or truth-label changes. Even if
both missing rows were perfect candidate offers, the maximum possible result
would be 82.53% precision at 24.78% coverage, so the failure is insensitive to
this drift.

No sealed holdout is warranted. Do not tune this rule on the exposed rows. The
unchanged broad mapping advances to a separately frozen Discogs-EffNet
selective-classification evaluation.

Reproducibility record:

- broad semantic SHA-256:
  `efe20460e7cc4b70af275ada2002be0dafa5cfbec0513a3cdd656b665773c255`
- private aggregate result SHA-256:
  `db69a9bee309f1c08e109b643c30bc6103f5bd6c9040f2906578f0c7d33d6f90`
- live corpus fingerprint:
  `sha256:b88911c7b24bbeecd1d59607ceb5e873ca29ff6f15052e77913635e5832471f1`
