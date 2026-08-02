# Plan 057: Evaluate hierarchical genre routing

> **Status:** Complete on 2026-08-02; bounded negative
> **Objective:** Preserve Discogs-EffNet's broad gains without collapsing useful
> fine genres into their larger families.

## Evidence Boundary

Plan 056 exposed the 619-row development corpus and showed that the flat
Discogs-EffNet projection is strong overall but regresses Breakbeat, Deep
Techno, and Electro recall. This plan is deliberately post-hoc development
work. Its result cannot establish generalization or authorize production use,
even if its internal gate passes.

The first candidate is frozen before its row-level output is inspected. There
will be no threshold, weight, alias, family, or exception search on this
development run.

## Candidate V1: Broad Router, Existing Fine Decision

For every row:

1. compute the unchanged Plan 056 direct Discogs-EffNet canonical prediction;
2. compute the unchanged current Reklawdbox baseline prediction with current
   genre removed and no stored audio profiles;
3. if the baseline prediction exists and belongs to the same classifier family
   as the Discogs-EffNet prediction, keep the baseline prediction as the fine
   label; otherwise use the Discogs-EffNet prediction.

This is intentionally minimal. The pretrained model decides the broad musical
region; the existing classifier may resolve only within that region. There are
no genre-specific overrides. The same frozen taxonomy-family function used by
the production classifier defines family equality.

## Development Gate

The one-shot development candidate is worth sealing for new evaluation only if
all conditions hold:

1. exact accuracy is at least 0.60;
2. macro recall is at least 0.45;
3. macro F1 is at least 0.40;
4. same-family accuracy is at least 0.78;
5. every fold improves macro F1 over the unchanged Plan 056 baseline;
6. no genre with at least ten rows loses more than 0.10 recall versus that
   baseline; and
7. Breakbeat, Deep Techno, and Electro each satisfy the same recall guard.

Passing means only `development_candidate_for_new_holdout`. Failing records a
bounded negative and stops this rule; it does not authorize exceptions.

## Fresh Holdout Boundary

No `genre_reference_holdout` playlist existed at the 2026-08-02 read-only
audit. The existing candidate playlist includes unverified tracks not scored in
Plans 056 or 057, but their current genre tags are provisional and their
artist/release/version relationships still need auditing.

If Candidate V1 passes its development gate:

1. freeze its code and checksums;
2. select prospective examples without classifier or model output, using
   external canonicality evidence and exact-version identity;
3. exclude every artist, remixer, release, and related version connected to the
   development corpus;
4. ask the operator to review no more than four to six tracks at a time;
5. route confident verdicts into a newly created sealed holdout and ambiguous
   verdicts into a separate boundary pool;
6. do not score the sealed tracks until the first boundary-focused cohort is
   complete; and
7. evaluate once, with current genre stripped and no configuration changes.

Begin with the measured failure/boundary region: Breakbeat, Deep Techno,
Electro, House, Tech House, and Techno. Accumulate four independent leakage
groups per genre (24 tracks) over small batches. This is a purposeful challenge
set, not a new all-taxonomy quota. If it supports the candidate, broaden the
holdout only in response to the next measured coverage gap.

## Safety And Privacy

- Rekordbox and the Reklawdbox cache remain read-only.
- The isolated model and private feature artifacts stay outside the supported
  runtime and outside Git.
- Row predictions, track identities, paths, labels, and fold assignments stay
  private. Only aggregate metrics and artifact checksums may be committed.
- No production dependency, cache schema, CLI, MCP, tag, XML, or classifier
  behavior changes in this plan.

## Recorded Result

The hard router repaired the three Plan 056 recall regressions. Relative to the
baseline, recall loss was 0.056 for Breakbeat and zero for both Deep Techno and
Electro; no genre with at least ten rows lost more than 0.10 recall. It also
improved macro F1 in every fold and retained 78.35% same-family accuracy.

It failed the aggregate gate:

- exact accuracy: 53.31% (required 60%);
- macro recall: 41.60% (required 45%); and
- macro F1: 36.06% (required 40%).

Result SHA-256:
`aa68e8da56ad07a6f89eae0ba4b2349c040fa2e44eaf8124352a4aef06a34a82`.
No candidate advanced to the holdout phase.
