# Plan 074: Prospectively validate House and Techno suggestions

> **Status:** Handover; proposed, not started
> **Objective:** Build and independently validate a new precision-first,
> House-and-Techno-only suggestion candidate without reusing consumed review
> rows as a release test.

## Read first

1. Plan 073 records the completed independent O3 evaluation and its failed
   release gate.
2. Plans 071 and 072 record the model lineage, feature contract, isolation
   rules, and earlier consumed holdouts.
3. Plan 053 records a structurally unsuccessful sparse-seed retrieval
   experiment. Its experimental MCP and public surfaces remain unmerged.
4. Plan 038 remains an incomplete corpus checkpoint on
   `codex/038-curate-genre-reference-corpus`; do not describe it as complete.

No broad-genre model is approved for release. This plan begins a new candidate,
not a reinterpretation or threshold adjustment of O3.

## Current evidence and decision

O3 offered 35 of 150 independently selected rows (23.33% coverage) and scored
30/35 (85.71% exact parent precision), below the preregistered 90% release
gate. Its useful signals were:

- House: 10/12 (83.33%);
- Techno: 20/21 (95.24%);
- Electro: 0/2;
- paired rows: O3 26/29 (89.66%) versus v0.33 17/29 (58.62%), a 31.03
  percentage-point improvement; and
- high/medium-confidence sensitivity only: 28/31 (90.32%).

The sensitivity result and a retrospective House-and-Techno-only score must
not be promoted to independent evidence. O3 is retired unchanged. The next
candidate, O4, should narrow its product claim to House and Techno, improve
hard-negative separation, and abstain everywhere else.

The final Plan 073 configuration/replay record has SHA-256
`77100b0d13eb10b899208e74a25f3684f2681ae442fbaf0af4eaf422fef7370e`.
Its evaluation/replay record has SHA-256
`6f669f2523534ee8dbe23fdb8bd1b8e25c9020f98bc89350418bb28bc950dc6b`.
Private artifacts remain under
`~/.local/share/reklawdbox/research/genre-intelligence-v1/plan073` with their
existing mode-0600 controls.

## Locked evidence rules

- All Plan 071, 072, and 073 holdouts and reviewed suggestions are consumed.
  They may be used for error analysis or development only, never as fresh
  validation.
- High- and medium-confidence reviewed parent verdicts may become development
  truth with their provenance retained. Low-confidence verdicts remain
  boundary/error evidence, not hard training labels.
- Preserve artist, release-group, path, and decoded-audio isolation across
  development and validation.
- Freeze candidate source, features, thresholds, and prediction artifacts
  before exposing any new review row.
- Never tune after seeing a prospective verdict. A changed candidate starts a
  new validation sequence.
- Rekordbox access remains read-only. Suggestions are previews; any eventual
  metadata handoff must use `ChangeManager` and XML.

## Available fresh evidence

A read-only collection audit on 2026-08-05, after excluding all consumed
research identities, found:

- 3,137 library rows;
- 124 eligible rows across 115 artists and 118 release groups;
- at most 115 rows after one-per-artist and one-per-release isolation; and
- hidden current-tag strata of 31 House and 93 Techno rows.

These are aggregate planning counts, not truth. At O3-like coverage the pool
would yield roughly 25 offers, so it cannot reliably power another immediate
30-offer test. Refresh the audit before selection, and accumulate prospective
evidence from future imports rather than weakening isolation or precision
requirements.

## Proposed sequence

### 1. Freeze a consumed-evidence development manifest

Build a reproducible manifest of accepted development truth and boundary
evidence. Record source verdict, confidence, provenance, normalization, and all
identity groups. Audit label balance and conflicting versions before fitting
anything. Do not include private identities in Git.

### 2. Develop O4 under group isolation

Train and compare House-versus-rest and Techno-versus-rest candidates using
artist/release/decoded-audio-grouped evaluation. Prioritize hard negatives from
House, Techno, Electro, Garage, Tech House, Minimal, and adjacent electronic
parents. Keep class-specific thresholds and require exactly one qualifying
parent; otherwise abstain. Do not enable Electro merely to increase coverage.

Use Plan 073 only for diagnosis and regression analysis. It is not a model
selection fold or a validation set.

### 3. Build a read-only shadow-review loop

Generate compact Markdown review batches with opaque codes, artist, title,
album, and audio location only. Hide model output, current genre, scores,
thresholds, and sampling strata until verdicts are frozen. Keep batches at six
or fewer by default, even if several batches are prepared together.

The reviewer should be able to record parent, confidence, alternatives, and
free-form listening notes. Preserve the user's words and optionally translate
them into groove, swing, timbre, arrangement, density, modulation,
progression, and scene-context vocabulary.

### 4. Validate prospectively

After O4 is frozen, score untouched current rows and then eligible future
imports in shadow mode. Accumulate frozen-prediction verdicts until the
preregistered gate is powered. Do not retune between batches or stop early on
a favorable result.

Before the first prediction, specify the exact gate. The recommended minimum
is:

- at least 30 offered rows;
- at least 90% exact parent precision overall;
- at least 85% precision for each of House and Techno when that parent has at
  least ten offers;
- at least a five-percentage-point paired improvement over v0.33 where both
  systems offer; and
- byte-replay, leakage, and identity-isolation checks passing.

Coverage is a disclosed product characteristic, not a pass condition to game.
If fresh evidence is too sparse, keep accumulating it.

### 5. Define the product contract only after a pass

The first releasable surface should be opt-in and explicitly experimental: a
read-only parent suggestion with a clear abstention state and user confirmation
before XML staging. It must not silently retag a collection or imply reliable
subgenre classification. Validate additional parents as separate expansions.

## STOP conditions

Stop and preserve evidence if any of the following occurs:

- a reviewed or otherwise exposed row enters prospective validation;
- development and validation share an artist, release group, path, or decoded
  audio;
- thresholds, labels, or features are changed after a prospective verdict;
- the fresh pool cannot power the preregistered test;
- an O3 retrospective slice is presented as independent evidence;
- private identities or audio-derived artifacts are about to enter Git; or
- a user-facing classifier is proposed before the exact release gate passes.

## Repository handover

At this handover, `main` contains the completed Plans 056 through 073 research
lineage and the O3 failure record. It is intentionally not pushed by this
cleanup. Unique unmerged work is preserved at:

- `codex/038-curate-genre-reference-corpus` (`7374386`): incomplete Plan 038
  corpus and validator checkpoint;
- `codex/053-discover-mislabeled-genres` (`73c6f1f`): failed retrieval
  experiment and unpromoted MCP/public surfaces;
- `codex/055-audit-contrastive-genre-retrieval` (`23ec500`): contrastive
  retrieval audit evidence;
- `codex/hydrate-concise-output` (`edca3c8`): concise hydrate implementation
  awaiting an explicit integration decision; and
- `codex/planning-workflow-integrity` (`241a518`): separate planning-workflow
  implementation awaiting review.

Clean worktrees whose commits were already merged into `main` may be removed;
the five branches above must be retained until their documented disposition is
decided.

## First actions in the next session

1. Confirm clean `main`, its ahead/behind state, and the retained branches.
2. Read Plans 071 through 074 and the Plan 053 outcome before designing O4.
3. Refresh the eligible-pool aggregate without exposing identities.
4. Build and audit the consumed-evidence development manifest.
5. Preregister O4's candidate and prospective gate before inference.

Do not begin by purchasing more canonical references, releasing a broad model,
or merging Plan 053's experimental surface. The immediate deliverable is a
reproducible O4 development/evaluation protocol and a read-only shadow loop.
