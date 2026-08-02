# Plan 061: Build a human genre-review assistant

> **Status:** Pilot complete on 2026-08-02; genre decisions passed and assistant
> utility was not demonstrated
> **Objective:** Turn the useful explanatory and similarity signals from Plans
> 056–060 into a small, read-only review workflow without treating any model as
> genre truth.

## Why this plan exists

The expanded-corpus classifiers retained useful broad musical structure but
failed their exact-label promotion gates. The frozen Plan 060 audit then found
only one target-aligned verdict among four proposed corrections. That rule is
retired and must not be tuned against its exposed rows.

The operator nevertheless found comparison vocabulary and concrete genre
references useful while listening. The next product question is therefore not
whether Reklawdbox can classify the collection automatically. It is whether it
can make a human-selected genre decision easier, clearer, and more consistent.

## Frozen MVP boundary

Build an isolated research assistant that consumes the already-frozen private
Plan 059 development embeddings and Plan 060 candidate embeddings. It must not
run new audio analysis, load or write stored profiles, mutate Rekordbox, stage
metadata changes, or add the pretrained model to the supported runtime.

For each selected track, produce a review packet containing:

- its current Rekordbox genre as context, never as truth;
- the nearest ear-verified references in the frozen embedding space;
- the current genre plus at most two alternative reference-neighbour genres,
  all explicitly labelled as listening hints;
- neutral, relative vocabulary for tempo, event density, spectral motion, and
  dynamics derived from existing analysis values; and
- blank `verdict`, `confidence`, `alternatives`, and `notes` fields.

Similarity values describe proximity in one representation. They are not
probabilities, confidence scores, or evidence that a tag should change.

## Frozen first batch

The first batch is confirmation-first rather than an error hunt:

1. Start from the 1,612 complete, previously unexposed Plan 060 candidates.
2. Exclude all six now-exposed Plan 060 pilot rows.
3. Require a canonical current genre with at least three usable verified
   references.
4. Exclude `Experimental`, which remains an anti-genre rather than a coherent
   modeled target.
5. Rank current genres by lowest verified support, then canonical order.
6. Within a genre, prefer the candidate with the strongest mean similarity to
   its three nearest same-genre verified references.
7. Select six distinct current genres and artist/release groups, with no more
   than two genres from one taxonomy family. Use a frozen hash as the final
   tie-break and fail rather than relax these constraints.

This selection is intentionally biased toward likely confirmations. Its
purpose is to test the review experience, not estimate classifier accuracy or
discover metadata errors.

## Reference and vocabulary rules

- Resolve verified reference identities from the already-imported 696-track
  `genre_verified` XML and require exact path agreement with all 670 usable
  development rows.
- Exclude same-artist and same artist/release references for the candidate.
- Normalize embeddings before cosine similarity.
- For each displayed genre, show at most two distinct verified references.
- Always show the current genre. Choose at most two alternatives by the mean
  similarity of their three nearest eligible references.
- Derive vocabulary from development-corpus quantiles. Describe only the
  measured dimensions; do not infer instrumentation, kick patterns, swing,
  mood, or arrangement events that the cached values do not encode.

## Export and review boundary

Export a six-track `genre_review_assistant_v1` playlist and a private review
sheet. Before and after `write_xml`, require `preview_changes` to report no
staged changes. Re-read every selected live record and fail on identity or path
drift.

The review sheet is not blind: the assistant is being evaluated as a decision
aid. The operator may answer `verified`, a canonical replacement genre,
`ambiguous`, or `skip`. No answer becomes metadata until a later, separately
reviewed XML reconciliation.

## Pilot gate

Ask for no more than six listening decisions. Continue this interaction design
only if:

1. at least four tracks receive a confident `verified` or replacement verdict;
2. the operator reports that the references or vocabulary were materially
   useful on at least four tracks;
3. no hint is presented or interpreted as automated truth; and
4. Rekordbox, audio, tags, and staged metadata remain unchanged.

Ambiguous and skipped rows are valid outcomes. The gate evaluates review
utility, not agreement with a hidden target. Do not tune the frozen first batch
after inspecting its identities.

## Verification

- Unit-test XML identity resolution, artifact alignment, related-reference
  exclusion, deterministic ranking, family and diversity constraints, hint
  bounds, vocabulary quantiles, and failure without quota relaxation.
- Re-run the selector and require byte-identical semantic output and roster
  hashes.
- Confirm playlist XML identities against the live read-only database.
- Confirm `preview_changes` is empty before and after export.
- Inspect the committed diff for private IDs, paths, artists, titles, albums,
  predictions, similarities, or verdicts.
- Run the standard workspace gate and maintained Plan 038 corpus gate.

## Done criteria

The implementation phase is complete when the deterministic six-track packet
and playlist are exported privately with no state writes, all gates pass, and
only the operator's utility and listening verdicts remain. The plan is fully
complete when the aggregate pilot gate is recorded without private identities.

## Pre-review result

The frozen selector consumed the existing 1,612-row candidate artifact and
670-row verified-reference artifact without audio decoding or inference. It
excluded the six exposed Plan 060 rows and found 1,529 candidates meeting the
reference and identity requirements. The fixed diversity rule selected six
unique genres and artist/release groups without relaxing the two-genres-per-
family cap.

An exact pre-export replay produced byte-identical private results. The export
then matched all six selected identities against the live read-only database,
resolved one supported compatibility alias to its canonical genre, and wrote a
playlist-only XML with six collection rows and six playlist references.
`preview_changes` was empty before and after; `changes_applied` was zero.

Reproducibility record:

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

The operator confidently verified the current canonical genre on all six rows.
There were no replacement genres, ambiguous verdicts, or skipped rows. The
genre-decision criterion therefore passed 6/6, above its required 4/6.

The operator did not need additional information for certainty on four rows.
On the two rows requiring more listening, the operator was unsure whether the
references or vocabulary materially helped. No row received a positive utility
report, so the 4/6 utility criterion was not demonstrated and the overall pilot
gate did not pass.

This is not a classification failure: all six decisions remained confident
after listening. It means this confirmation-first batch does not justify
continuing the interaction design as-is. Any later utility experiment should
be separately planned around genuinely ambiguous decisions, preferably with a
small assisted-versus-unassisted comparison rather than more blind accuracy
review.

No hint was treated as automated truth, and no Rekordbox metadata, audio, tags,
or caches were changed. Track identities, paths, hints, similarities, and
listening notes remain outside Git.
