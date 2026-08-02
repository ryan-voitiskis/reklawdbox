# Plan 038: Maintain gap-driven genre reference evidence

> **Status:** Rescoped by operator on 2026-08-02. This replaces the original
> requirement to research 12 purchasable candidates for every canonical genre.
> The seven completed dossiers remain useful; the unfinished catalog-wide quota
> is retired.

## Decision

Genre-reference work is now driven by measured classifier gaps, not by a fixed
catalog-completion target. Reklawdbox should first evaluate broad genre
classification against the live ear-verified corpus. Additional references are
researched only when that evaluation identifies an under-covered genre or a
specific unresolved confusion boundary.

The earlier 52-genre/624-candidate target was reasonable before the project had
substantial listening evidence. It is no longer a useful definition of
completion: it would require extensive sourcing and purchasing without first
showing that those genres limit classifier quality.

## Durable outcomes retained from the original plan

- `Dub` is canonical. `Dub Reggae` and `Reggae Dub` are compatibility aliases.
- `Drone Techno` is not a canonical classifier target. The phrase may still be
  used descriptively in historical or musicological discussion.
- `Experimental` remains an anti-genre/umbrella and is not modeled as one
  coherent audio prototype.
- Candidate status is provisional. A researched recording becomes truth only
  after the operator listens to the exact version and approves it.
- Development anchors never become sealed holdouts merely because they were
  used successfully in an experiment.
- Release/remaster/remix relationships and artist relationships must remain in
  one leakage group for evaluation.

The public corpus currently contains 84 source-verified, purchasable candidates
across seven completed dossiers: Dub, Electro, Garage, Hardcore, IDM, Minimal,
and Tech House. Empty genre records are retained as an explicit coverage map,
not as unfinished mandatory work.

## Current objective

Improve broad genre classification in this order:

1. Evaluate the current classifier without stored profiles against profiles
   trained only inside each group-stratified fold of `genre_verified`.
2. If the current Fisher-profile representation fails a pre-registered gate,
   evaluate a pretrained music representation in an isolated offline harness.
3. Inspect aggregate per-genre and same-family errors.
4. Ask for small listening batches only where the evidence exposes a concrete
   gap that additional human truth can resolve.
5. Add or refresh a public dossier only when the source research itself will be
   reused. Do not create a purchasing quota merely to fill a taxonomy row.

Plans 056–058 record the completed profile, pretrained-representation,
hierarchical-router, and supervised-adapter evaluations. None passed its frozen
development gate. Plan 055 is the retained bounded-negative Tech House
retrieval audit.

The 2026-08-02 truth audit found 51 unambiguous, operator-approved development
references absent from `genre_verified`. They were approved before the
representation outputs were inspected and are not eligible for a future sealed
holdout. A playlist-preserving XML export now reconciles them into a 696-track
`genre_verified` playlist and stages seven approved Minimal genre corrections.
Explicit boundary or uncertain verdicts remain excluded. Rekordbox has not
changed until the operator manually imports the XML.

After import, confirm the exact playlist count, genre distribution, and fresh
analysis coverage before deciding whether the unchanged frozen
representations merit one new development rerun. Do not reinterpret that rerun
as sealed evidence merely because the truth pool grew.

## Reference-corpus maintenance rules

The existing structured corpus and validator remain useful for completed public
dossiers:

- every committed candidate has exact version metadata, source provenance, a
  legitimate digital-purchase route observed on the recorded date, a leakage
  group, and a proposed review role;
- retailer categorization alone is not canonicality evidence;
- no private Rekordbox ID, file path, ownership state, fingerprint, account
  data, price, listening verdict, or audio is committed;
- populated dossiers continue to meet the original diversity and source
  standards; and
- empty dossiers are valid in the maintained gap-driven corpus.

Run the maintained incomplete-corpus gate after changing the JSON:

```bash
python3 -m unittest scripts/test_validate_genre_reference_corpus.py
python3 -m json.tool \
  docs/genre-classification/genre-reference-candidates.json >/dev/null
python3 scripts/validate_genre_reference_corpus.py \
  --allow-incomplete \
  docs/genre-classification/genre-reference-candidates.json
```

The validator's complete mode preserves the original 52-dossier contract for
archival or voluntary completion. It is no longer a project or classifier gate.

## Listening and truth boundary

When evaluation identifies a gap:

1. Freeze the question, candidate universe, exclusion list, and acceptance
   metric before examining results.
2. Prefer already-owned tracks and the existing verified corpus.
3. If sourcing is warranted, research exact versions and current legal purchase
   routes. Purchasing still requires the operator's decision.
4. Present at most four to six new listening decisions at a time unless the
   operator asks for a larger batch.
5. Record approval as development truth or as a sealed holdout before any
   subsequent tuning. Never use one recording in both roles, including related
   versions.
6. Route any Rekordbox-visible metadata through XML; never mutate `master.db`.

## Done criteria for a gap-driven work item

A reference work item is complete when:

- the classifier gap and affected genre boundary are stated in advance;
- existing verified examples were audited before new music was proposed;
- the smallest useful candidate/listening batch was used;
- exact-version, artist, release, and related-version leakage is controlled;
- operator verdicts are distinguished from external-source claims;
- no classifier metadata is written automatically; and
- the resulting evaluation is either promoted under its pre-registered gate or
  recorded as a bounded negative without post-hoc tuning.

There is intentionally no global requirement to populate every genre dossier or
reach 624 candidates.

## Stop conditions

Stop and request an operator decision if:

- a proposed taxonomy change would add, remove, merge, or rename another
  canonical target;
- the exact purchased/imported version cannot be distinguished from a related
  version that has already entered development or holdout evidence;
- a listening candidate would expose a sealed holdout;
- a result would require treating a store tag, artist reputation, or current
  Rekordbox genre as ground truth;
- a runtime experiment would write profiles, cache rows, audio tags, or
  Rekordbox metadata; or
- a model or dependency cannot be isolated from the supported runtime.
