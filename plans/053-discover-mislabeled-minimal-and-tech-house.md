# Plan 053: Discover mislabeled Minimal and Tech House with sparse-seed retrieval

> **Executor instructions:** Read this plan in full before changing code. Run
> the drift and live-readiness checks first. Use an isolated worktree and do
> not overlap expensive audio analysis with the in-progress lossy-source work.
> Stop on any condition in **STOP conditions**; do not silently turn provisional
> listening material into classifier truth.
>
> This plan creates a read-only discovery and review loop. It is not permission
> to retag tracks automatically, write Rekordbox's `master.db`, treat candidate
> labels as verified, tune against a sealed holdout, or deploy a profile.

## Status

- **Outcome:** COMPLETE — bounded negative result; do not promote the
  experimental MCP surface
- **Priority:** P1
- **Effort:** M for the discovery MVP; L if the MCP surface is promoted
- **Risk:** MED
- **Depends on:** Plan 038 listening checkpoint; current audio-integrity work
  must be idle or isolated before local analysis runs
- **Category:** classification evidence / active listening / sparse-label
  retrieval
- **Planned at:** `main` commit `d5603cb`, Reklawdbox v0.33.0, 2026-08-01
- **Plan 038 checkpoint:** branch
  `codex/038-curate-genre-reference-corpus` at `7374386`

### Execution result (2026-08-02)

The read-only MVP was implemented and evaluated privately, but the retrieval
method did not pass this plan's listening gate.

- The development corpus grew to 16 ear-verified Tech House anchors spanning
  early/stripped, deeper/minimal-adjacent, and modern strains.
- The final frozen experiment excluded 113 previously exposed review rows,
  scored 760 of 764 eligible candidates and 189 of 196 controls, and made zero
  Rekordbox or staged metadata changes.
- In the final ten-track blind page, the ranked cohort produced zero confident
  positives, two boundaries, and three negatives. The matched control cohort
  produced zero confident positives, one boundary, and four negatives.
- Strict positive yield was therefore 0% in both cohorts. Even counting
  boundaries permissively produced 40% ranked versus 20% control, while the
  required five new confident positives remained unmet.
- An earlier frozen pass was also retired unchanged after producing zero
  confident ranked positives while its control cohort produced one.

The failure mode is structural rather than a threshold miss. Best-match
aggregation against any single seed admitted ordinary House, Techno, Minimal
Techno, and Deep Techno tracks that shared tempo, brightness, energy, rhythm,
or broad timbre with one heterogeneous Tech House strain. The reported
nearest-control margin did not prevent these adjacent-genre matches because
positive similarity remained the primary ordering signal.

This branch is retained as experiment evidence only. Do not merge its MCP,
schema, SOP, or public-documentation changes into `main`. A successor
experiment must start from a clean `main` base, compare density-based
within-strain positive evidence against explicit House/Techno hard negatives,
and use a fresh unopened review batch.

## Decision

Proceed, but solve the bootstrap problem as **retrieval**, not classification.

The first result is a ranked listening queue of tracks currently tagged House
or Techno that resemble one or more user-approved examples. A result means
“worth listening to,” not “is Minimal” or “is Tech House.” Only the operator's
later verdict can promote a track to verified truth.

This distinction is necessary because the positive sets are currently too
small for a trustworthy genre prototype and because Tech House contains
materially different strains. The ranker must preserve per-seed matches rather
than average all seeds into one supposedly representative centroid.

## Why this is the right next step

The collection likely contains useful examples hidden under broad House and
Techno tags. Those tracks are more valuable than purchasing further references
if a reviewable audio-similarity pass can find them.

The existing system already has most of the useful primitives:

- fresh Stratum and Essentia cache identities;
- scalar audio features used by classification;
- globally normalized MFCC and spectral-contrast timbral vectors;
- pure pairwise pool-scoring axes; and
- read-only track selection plus XML-only metadata handoff.

The existing `expand_pool` workflow is not the right final algorithm. It is
optimized for DJ-pool cohesion, requires each addition to remain compatible
with every pool member, and lets early additions affect later ranking. Genre
discovery instead needs a one-shot score against fixed seeds, an `any_seed`
aggregation for multi-strain genres, and explicit contrast against known House
and Techno examples.

## Live baseline recorded during planning

These are read-only observations from the installed v0.33.0 binary on
2026-08-01. Re-check them at execution time.

### `genre_verified`

- 625 tracks total; 624 have complete, scorable Stratum + Essentia evidence.
- House: 145 complete/scorable tracks.
- Techno: 51 complete/scorable tracks.
- Deep House: 140 complete/scorable tracks.
- Deep Techno: 19 complete/scorable tracks.
- Minimal: one track, currently missing both analyzers.
- Tech House: no track in the playlist.
- No compatible stored profile registry currently exists; coverage reports
  `profile metadata is missing`.

### `genre_reference_candidates`

- 70 tracks: Dub 10, Electro 10, Garage 10, Hardcore 11, IDM 10, Minimal 10,
  and Tech House 9.
- Only two tracks currently have complete/scorable audio; both are IDM.
- All ten Minimal and all nine Tech House candidates currently need fresh
  Stratum and Essentia analysis before audio retrieval can use them.

This baseline means that recalibrating today would not produce a Minimal or
Tech House prototype. It also means the new imports must be analyzed before
they can act as discovery seeds.

## Locked listening state

Private IDs, paths, ownership data, and verdicts stay outside the repository.
The following names document the planning decision only.

### User-approved discovery seeds

- Minimal: Recondite — `Dwell`.
- Tech House: Housey Doingz — `Gob Stopper`.
- Tech House: Green Velvet — `Bigger Than Prince (The Martinez Brothers
  Remix)`.
- Tech House: East End Dubs — `Dis`.

All four become **development/discovery seeds**. Any Plan 038 recommendation
that placed one of them in `holdout_candidate` is superseded for this
experiment. A track used to retrieve or tune candidates cannot later be called
a sealed holdout.

### Reviewed boundaries, not positive seeds

- Minimal/Dub Techno boundary: Monolake — `Cyan`.
- IDM/Microhouse/Deep House boundary: Jan Jelinek — `Tendency`.

Maurizio — `M-4.5A` remains useful Minimal boundary material even though the
operator has not made the same explicit listening call on it.

### Unresolved tracks

The remaining eight imported Minimal candidates and six imported Tech House
candidates are unresolved. They are neither positives nor negatives. Do not
train from their provisional Genre fields.

For Minimal only, source-curated training anchors may be used as a separately
reported **weak probe tier** because the operator is unfamiliar with the
genre, not because those tracks were ear-approved. A result supported only by
a weak probe must rank below or be displayed separately from results supported
by `Dwell`.

## Defaults that avoid blocking on questions

1. **First-wave candidate universe:** exact current Genre `House` or `Techno`,
   excluding samplers, the seed tracks, `genre_verified`, and
   `genre_reference_candidates`.
2. **Second wave only if the first wave is low-yield:** exact current Genre
   `Deep House` or `Deep Techno`.
3. **No current-tag boost:** the current genre selects the search universe but
   contributes zero weight to similarity.
4. **No key compatibility:** musical-key compatibility is useful for DJ
   sequencing but irrelevant to genre identity; it receives zero weight.
5. **Multi-strain aggregation:** use the best fixed-seed match and report which
   seed matched. Do not require a candidate to resemble all Tech House seeds.
6. **No automatic writes:** the discovery tool returns a roster. Creating a
   review playlist or staging a Genre change remains a separate, explicit XML
   action.

## Success definition

The MVP succeeds if it makes listening materially more efficient, not if it
claims model accuracy.

For each target genre, compare a ranked cohort with a stratified-random control
cohort drawn from the same current-genre and BPM bands. Continue to profile
training only when:

1. the ranked cohort's positive hit rate is at least twice the control hit
   rate; and
2. listening produces at least five new confident positives for that target
   genre; and
3. no result was promoted from an unresolved or boundary label without an
   explicit user verdict.

If Minimal produces fewer than five positives, keep it as an exploratory queue
and use the listening results to choose better seeds. Do not compensate by
lowering the verification standard.

## Non-goals

- Do not finish the remaining 45 Plan 038 genre dossiers here.
- Do not purchase more music.
- Do not infer Minimal or Tech House from retailer tags alone.
- Do not deploy new classification rules, thresholds, DSP features, or
  profiles as part of the first retrieval pass.
- Do not consume the lossy-source detector as a genre feature.
- Do not auto-stage Genre changes.
- Do not put private track IDs, file paths, listening verdicts, or review
  rosters in Git.

## Architecture

### Pure retrieval kernel

Add a pure, deterministic ranking operation over already-built track profiles.
It receives fixed positive seeds, optional weak probes, optional boundaries,
and a candidate set. It must not read current genre except before this layer,
where genre is used only to construct the candidate universe.

For each candidate, calculate pairwise axes against each fixed seed using the
existing normalized feature machinery:

- timbral similarity: MFCC mean/std, spectral contrast, centroid variation,
  and dissonance through the existing timbral vector;
- BPM proximity;
- energy proximity;
- rhythm-regularity proximity; and
- brightness proximity.

Start with this explicitly provisional weight hypothesis:

| Axis       | Weight |
| ---------- | -----: |
| Timbral    |   0.45 |
| Rhythm     |   0.15 |
| BPM        |   0.15 |
| Energy     |   0.15 |
| Brightness |   0.10 |
| Genre      |   0.00 |
| Key        |   0.00 |

Weights are an experiment configuration, not a classifier policy. Freeze the
configuration before opening blind-review verdicts. Any change creates a new
experiment version and must be evaluated on unopened review material.

### Multi-seed score

For approved seeds:

```text
positive_similarity(candidate) = max(pair_similarity(candidate, seed_i))
```

Return the matched seed and every axis contribution. Do not hide a weak axis
inside one composite number.

Weak Minimal probes use the same kernel but are reported in a separate tier.
They must not increase an approved-seed score or be described as verified
evidence.

### Contrastive context

The strong negative/control pool is the complete, ear-verified House and
Techno material in `genre_verified`; the second-wave run may add verified Deep
House and Deep Techno controls.

Report the mean of the five nearest control similarities and a margin:

```text
contrast_margin = positive_similarity - mean(top_5_control_similarities)
```

The margin is an explanation and ranking tiebreaker, not proof of genre. A
candidate can be close to both a Tech House seed and ordinary House; that is
exactly the boundary the user needs to hear.

Boundary-track similarity is also reported but does not apply an automatic
penalty. Boundary examples are informative and should not be converted into
negative truth.

### Stable, diverse result ordering

Sort by:

1. approved-seed tier before weak-probe-only tier;
2. positive similarity descending;
3. contrast margin descending;
4. stable track ID ascending.

Apply a presentation-only cap of two tracks per release and three per artist
in the first review page. Preserve the uncapped rank in the output so diversity
does not alter the underlying experiment.

### Coverage contract

A row is eligible only when the same fresh Stratum and Essentia identities
required for Full classification are present and the timbral vector is valid.
Return counts and reasons for:

- candidates selected;
- candidates scored;
- stale/missing Stratum;
- stale/missing Essentia;
- invalid or dimension-mismatched timbral vectors; and
- excluded verified/candidate/seed tracks.

Never silently rank incomplete rows using a lower-information fallback in the
same result set.

## Delivery sequence

### Phase 0 — Isolate and re-check

1. Wait until the audio-integrity agent has finished using the main worktree,
   or create an isolated worktree from the reviewed integration base.
2. Confirm the main worktree's unrelated changes are untouched.
3. Re-run `calibration_coverage` for `genre_verified` and
   `genre_reference_candidates` with the binary being tested.
4. Resolve the four seed recordings to exact Rekordbox IDs and exact versions.
   Write the private mapping to
   `/tmp/reklawdbox-genre-discovery-seeds.json`, never to the repo.
5. Freeze experiment ID `minimal-tech-house-discovery-v1`, seed roles, weight
   configuration, source genres, exclusions, and review sample size before
   opening any verdicts.

### Phase 1 — Build only the required fresh evidence

1. Analyze the four approved seeds with current Stratum and Essentia using
   exact IDs and `skip_cached: true` semantics.
2. Analyze reviewed Minimal boundaries and any explicitly selected weak probes
   in a separate batch.
3. Resolve the first-wave House/Techno universe and inspect current cache
   coverage before starting more work.
4. Analyze only missing members of that fixed universe. Do not rescan the
   entire Music directory and do not run concurrently with the lossy-source
   benchmark workload.
5. Re-run coverage and require complete, fresh evidence for every seed and for
   each row included in the experiment.

Cache writes from analysis are allowed Reklawdbox-owned state. Rekordbox itself
remains read-only.

### Phase 2 — Implement the read-only MVP

Prefer a narrow application workflow and MCP transport over a private script
that duplicates cache identity, track-profile construction, or timbral
normalization.

Suggested public name after the experiment gate:
`suggest_genre_review_candidates`.

Minimum parameters:

```text
target_genre: canonical genre name used as a review label only
seed_track_ids: non-empty approved development seeds
probe_track_ids: optional weak, non-verified query examples
boundary_track_ids: optional explanatory boundary examples
source_genres: exact current genres; default ["House", "Techno"]
negative_playlist: default "genre_verified"
negative_genres: default source_genres
limit: default 40, maximum 100
max_tracks: explicit bounded candidate scan
```

Minimum result fields:

```text
method_status: "exploratory_retrieval"
experiment inputs and normalized weights
coverage and exclusion counts
track ID, artist, title, current genre, BPM
rank, uncapped rank, evidence tier
positive similarity and matched seed ID
top-five negative-control mean and contrast margin
boundary similarity, when available
per-axis scores and missingness
```

The handler must reject:

- an empty seed list;
- a non-canonical target genre;
- any seed also supplied as a boundary or negative control;
- a candidate scan without an explicit upper bound;
- non-finite or all-zero weights;
- a seed lacking complete/scorable evidence; and
- an attempt to request staging or mutation.

### Phase 3 — Characterize before trusting

Use synthetic fixtures for mandatory tests. Private-library checks are opt-in
and must never become mandatory CI.

1. Pairwise scoring is symmetric and deterministic.
2. `any_seed` chooses the strongest seed rather than averaging strains.
3. Current Genre and musical key have zero scoring influence.
4. Weak probes cannot outrank the approved-seed tier solely by being marked as
   probes.
5. Boundaries are explanatory only.
6. Contrast controls affect the reported margin but cannot turn a low positive
   similarity into a high one.
7. Missing or stale evidence is counted and excluded, never defaulted.
8. Artist/release diversity changes presentation order only; uncapped ranks
   remain stable.
9. The workflow performs no `ChangeManager`, XML, tag, or Rekordbox mutation.
10. Pagination and limits are applied after all semantic exclusions.

Run a Tech House leave-one-seed-out diagnostic. It is not a holdout accuracy
claim: all three tracks are already development seeds. Record where each held
out seed lands when the other two are used. Large differences are evidence of
multiple strains and should remain visible rather than being averaged away.

For Minimal, one approved seed cannot support leave-one-out validation. Report
that limitation directly.

### Phase 4 — Create a blind listening batch

1. Take up to 30 ranked candidates per target genre after diversity caps.
2. Draw 15 stratified-random controls per target genre from the same exact
   source genres and coarse BPM bands.
3. Shuffle ranked and control rows into one private roster with a fixed random
   seed. Keep the rank/control mapping only in `/tmp`.
4. Exclude all development seeds, weak probes, boundaries, existing verified
   tracks, reference-candidate tracks, and release/leakage duplicates.
5. Only after the roster is frozen, use the existing explicit XML workflow to
   create `genre_discovery_blind_v1` if the operator asks for the playlist.
   Do not stage Genre values.
6. Capture one of four private verdicts per track:
   `positive`, `boundary`, `negative`, or `unsure`.
7. Open the frozen mapping only after every verdict in that batch is recorded.

Report precision at 10, precision at 20, overall ranked hit rate, control hit
rate, yield, and per-seed yield. Do not call these collection-wide classifier
accuracy metrics.

### Phase 5 — Promote only listening truth

If the success gate passes:

1. Mark every track used as a seed or tuning verdict as development/training
   material, never holdout.
2. De-duplicate by recording/release leakage group.
3. Propose, but do not automatically perform, an XML export that adds approved
   positives to `genre_verified` and stages their user-approved canonical Genre.
4. Source a fresh, independent holdout after the retrieval configuration is
   frozen. No seed, weak probe, boundary used for tuning, ranked-review row, or
   release sibling may enter that holdout.
5. Require at least five complete/scorable training positives for software
   eligibility and prefer ten or more before treating a prototype as useful.
6. Seal the independent holdout before any profile calibration or weight
   change.
7. Calibrate into a development registry, score the sealed holdout once, and
   compare against the no-profile baseline before proposing deployment.

If the success gate fails, keep the verdicts as useful taxonomy evidence,
retire experiment v1 unchanged, and design v2 with new seeds or features. Do
not tune v1 after seeing its blind labels.

### Phase 6 — Promote the MCP surface only if useful

The first private experiment may keep the transport explicitly experimental.
Promote and document the MCP tool only after the blind batch passes the success
gate. If it fails, remove the experimental surface and retain only reusable
pure-kernel improvements that have independent value and tests.

Promotion requires:

- MCP schemas and descriptions that say “review candidate,” never “classified”;
- genre-classification SOP updates;
- help and public contract updates;
- semantic doc-drift review; and
- a happy-path plus incomplete-evidence MCP smoke test from the current
  checkout.

## Expected production files if the MCP is promoted

Exact paths may drift; the executor must follow current ownership boundaries.

- `src/domain/planning/` or `src/domain/classification/` — pure fixed-seed
  retrieval kernel; do not duplicate pairwise feature math.
- `src/application/classification/` — candidate selection, exclusions,
  negative-control resolution, coverage, and stable ranking.
- `src/mcp/classification/transport.rs` — request/response schema.
- `src/mcp/classification/handlers.rs` and `src/mcp/server.rs` — read-only MCP
  transport.
- `src/mcp/tests/classification.rs` plus focused domain/application tests.
- `site/src/partials/sops/genre-classification.mdx`, `src/mcp/help.rs`, and
  relevant public reference docs if the public tool ships.

Do not modify `stratum-dsp/` or audio-integrity policy files in this plan.

## Drift check

Before implementation, compare the planning base with the execution base:

```bash
git diff --stat d5603cb..HEAD -- \
  src/domain/classification \
  src/domain/planning \
  src/application/classification \
  src/application/planning \
  src/mcp/classification \
  src/mcp/planning \
  src/mcp/server.rs \
  src/mcp/help.rs \
  site/src/partials/sops/genre-classification.mdx \
  docs/tmp/genre-classification-features-todo.md
```

Re-read every changed in-scope file. Updating paths or test counts to match a
compatible refactor is expected. A changed cache identity, profile scoring
contract, read/write boundary, or active audio-integrity overlap is a STOP
condition until the plan is reconciled.

## Verification

### Focused gates

Use the exact current package/test filters discovered at execution time. At
minimum:

```bash
cargo fmt --check
cargo test -p reklawdbox classification -- --nocapture
cargo test -p reklawdbox planning -- --nocapture
cargo test -p reklawdbox mcp -- --nocapture
dprint check
git diff --check
```

If an MCP/public surface ships, run:

```bash
node scripts/check-doc-contract.mjs
./scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db
```

Then run the full workspace gate from `AGENTS.md`:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
```

Review `docs/workflows/doc-drift/README.md` whenever a tool, schema, SOP, help
entry, or public contract changes.

### Local private gate

After mandatory tests pass, run one opt-in local experiment against the frozen
private manifest. Verify:

- exact seed identities;
- zero Rekordbox writes and zero staged changes;
- deterministic repeat output for the same experiment configuration;
- complete coverage accounting;
- stable uncapped rankings; and
- no private IDs, paths, or verdicts appear in the Git diff.

## STOP conditions

Stop and report if any of the following occurs:

1. The lossy-source agent is still changing overlapping source/cache contracts
   or running a competing heavy analysis workload and an isolated execution is
   not possible.
2. Any seed resolves to an uncertain recording or mix/version.
3. A proposed sealed holdout has already been used as a seed, weak probe,
   boundary for tuning, review candidate, or weight-selection example.
4. The implementation needs the current Genre tag as a positive scoring
   feature rather than only a candidate-universe filter.
5. Fresh Full-classification evidence cannot be established for all seeds.
6. The scorer silently mixes complete and degraded rows.
7. The workflow stages metadata, writes tags, or mutates Rekordbox without a
   separate explicit operator request.
8. Blind labels are opened before the experiment inputs are frozen.
9. A result supported only by source curation is represented as ear-verified.
10. Private library identities or listening decisions enter a tracked file.

## Git workflow

- Execute in an isolated branch such as
  `codex/053-discover-mislabeled-genres` after selecting the reviewed base.
- Preserve and do not stage the other agent's audio-integrity files.
- Use reviewable Conventional Commits, for example:
  - `feat(classification): rank sparse-seed review candidates`
  - `test(classification): characterize genre retrieval ranking`
  - `docs(classification): document discovery review workflow`
- Do not push, open a PR, deploy, or release without separate authorization.

## Done criteria

This plan is complete only when either:

### Successful path

- the fixed-seed ranker is deterministic, read-only, coverage-complete, and
  tested;
- a frozen blind batch has been reviewed;
- ranked-versus-control yield and per-seed results are reported honestly;
- at least five new confident positives per promoted target genre exist;
- development and holdout roles are leakage-safe;
- no automatic genre changes occurred; and
- any promoted MCP/docs surface passes focused, doc-drift, and full gates.

### Bounded negative result

- v1 is frozen and shown not to beat its stratified control usefully;
- no profile or auto-tagging behavior is deployed;
- the failure mode is recorded without private library data; and
- the next experiment is framed around better seeds or independently validated
  features rather than post-hoc threshold tuning.
