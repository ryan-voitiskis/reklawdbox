# Plan 034: Make post-Beatport classification confidence source-aware and measurable

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` only if the reviewer has not told you that they own the
> index.
>
> **Required base and drift check (run first)**:
>
> This plan was written against commit `4e031ca` plus the uncommitted
> post-Beatport-removal working-tree snapshot present on 2026-07-14. Commit
> `4e031ca` itself still contains Beatport. Do not execute this plan until that
> removal is reviewed and integrated into a clean base.
>
> ```bash
> test -z "$(git status --porcelain)"
> ! rg -n -i "beatport" src --glob '*.rs'
> git diff --check
> git rev-parse HEAD
> ```
>
> Expected: the worktree is clean; the Beatport search exits 1 with no active
> Rust references; `git diff --check` exits 0. Record the resulting commit as
> `REMOVAL_BASE`. Compare the live code with every excerpt below. If the vote,
> confidence, handler-filter, or benchmark semantics have changed, STOP and
> re-audit rather than adapting this plan silently.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: reviewed post-Beatport removal base; extends completed Plans
  009 and 023
- **Category**: classifier correctness / tests / public review contract
- **Planned at**: commit `4e031ca` plus post-Beatport working-tree snapshot,
  2026-07-14

## Why this matters

Removing Beatport deletes a strong classifier input rather than merely a
network integration. A read-only audit of the local cache found 1,425 cached
Beatport genre results; 359 had no Discogs styles, and only 3 of those 359 had
Bandcamp tags. These are planning aggregates observed on 2026-07-14, not
portable test fixtures or release thresholds, but they establish that real
tracks will lose evidence.

The remaining confidence calculation can also mistake correlated data for
independent consensus. When the Rekordbox label is empty,
`build_track_evidence` copies the label from the same Discogs response that
provided the styles:

```rust
let effective_label = if !track.label.is_empty() {
    Some(track.label.clone())
} else {
    discogs_val
        .as_ref()
        .and_then(|v| v.get("label"))
        .and_then(|v| v.as_str())
        .filter(|l| !l.is_empty())
        .map(std::string::ToString::to_string)
};
```

The engine then treats Discogs style and label as separate weights. A focused
style can contribute `0.9`, its label contributes `0.4`, and the single-winner
branch declares `High` at score `>= 1.0`. That is one provider payload counted
twice. The stored enrichment row already exposes `match_quality`, but evidence
construction discards it, so fuzzy and exact matches receive the same weight.

The current genre field is also added to the vote pool with up to `0.5` weight:

```rust
let tokens = genre::extract_genre_tokens(&evidence.current_genre);
if !tokens.is_empty() {
    let n = tokens.len();
    let weight_per = (0.5 / n as f32).min(0.5);
    for g in tokens {
        let plausible = bpm_plausible(g, effective_bpm);
        votes.push(GenreVote {
            genre: g,
            weight: if plausible {
                weight_per
            } else {
                weight_per * 0.5
            },
            source: "current-genre",
            bpm_plausible: plausible,
        });
    }
}
```

That input is the value being audited, not independent evidence. It may be a
normalization hint or conservative tie-breaker, but must not manufacture
confidence in its own correctness.

Finally, all `Confirm` results are hidden from normal result, audit, and review
dispatch paths even if their confidence is Low or Insufficient:

```rust
.filter(|r| !matches!(r.action, ClassificationAction::Confirm))
```

This plan makes source independence explicit, preserves weak confirmations for
human review, and creates a real benchmark before any further taxonomy or
weight tuning. It extends Plan 009's missing-evidence contract and Plan 023's
workflow-specific readiness vocabulary; it does not replace either one.

## Current state

### Confidence is based on summed weight, not independent provenance

`src/domain/classification/engine.rs::gather_votes` currently emits flat
`GenreVote`s. The relevant weights are:

```rust
let base_weight = proportion * 0.9 * diversity_decay;
// ...
let weight = if confirms { 0.4 } else { 0.6 };
```

`src/domain/classification/engine.rs` then assigns High without checking unique
source groups:

```rust
let mut confidence = if ranked.len() == 1 && top_score >= 1.0 {
    if votes
        .iter()
        .filter(|v| v.genre == top_genre)
        .all(|v| v.bpm_plausible)
    {
        ClassificationConfidence::High
    } else {
        flags.push("bpm-implausible".into());
        ClassificationConfidence::Medium
    }
```

Audio rules and calibrated profiles are both derived from Stratum/Essentia
measurements. They may contribute separately to ranking, but they are one
independence group for confidence. BPM plausibility is a constraint on a vote,
not another source.

### Match quality exists before the classifier boundary

`src/adapters/state/enrichment.rs::EnrichmentCacheEntry` retains:

```rust
pub match_quality: Option<String>,
pub response_json: Option<String>,
```

`src/application/enrichment/lookup.rs::persist_discogs_result` writes `exact`
or `fuzzy`, while `build_track_evidence` currently parses only
`response_json`. The classification model has `has_discogs: bool`, but no
Discogs match-quality or label-provenance field.

### Reviewability is action-gated instead of confidence-gated

`ClassificationAction::Confirm` means only that current and recommended
canonical genres are equal. It says nothing about confidence. Nevertheless:

- full and compact `classify_tracks` omit every Confirm;
- `needs_review` first excludes every Confirm;
- `audit_genres(include_confirmed=false)` omits every Confirm; and
- `build_dispatch_groups` skips every Confirm before inspecting confidence.

Auto-staging is still the canonical `ChangeManager` path and must remain so.
This plan does not write to Rekordbox or stage a no-op confirmation.

### The existing "golden" test does not exercise classification

`src/mcp/tests/classification.rs::golden_dataset_genre_accuracy` is ignored and
loads nine fixture rows across Techno, House, and Deep House. It selects each
track from the real database and compares the existing Rekordbox genre to the
fixture, but never calls the classifier. It therefore measures fixture drift,
not classifier accuracy.

`stratum-dsp/benchmarks/real-audio-v1/README.md` explicitly limits its frozen
24-track baseline to DSP drift. All tracks are Dub Techno positives and there
are no negative controls, so it is not a classifier benchmark. Preserve that
v1 benchmark unchanged.

The Beatport-removal diff also deleted provider-shaped tests that happened to
cover provider-independent branches, including same-family and cross-family
BPM overrides. Those behaviors still exist in `engine.rs` and need direct
tests that do not name or construct Beatport.

## Commands you will need

| Purpose                         | Command                                                                                     | Expected on success                                                                                       |
| ------------------------------- | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Source-independent engine tests | `cargo test -p reklawdbox domain::classification -- --nocapture`                            | exit 0; provenance, hint, BPM-override, and confidence tests pass                                         |
| Handler regressions             | `cargo test -p reklawdbox classification -- --nocapture`                                    | exit 0; weak Confirm results remain reviewable                                                            |
| Benchmark metric unit tests     | `cargo test -p reklawdbox classification::evaluate -- --nocapture`                          | exit 0; aggregation and leakage guards pass                                                               |
| Real benchmark                  | `cargo test -p reklawdbox golden_dataset_genre_accuracy -- --ignored --nocapture`           | exit 0 when the private DB/store and verified playlist are available; prints structured aggregate metrics |
| Rust format                     | `cargo fmt --check`                                                                         | exit 0                                                                                                    |
| Docs/config format              | `dprint check`                                                                              | exit 0                                                                                                    |
| Lint                            | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                   | exit 0                                                                                                    |
| Crate tests                     | `cargo test -p reklawdbox --no-fail-fast`                                                   | exit 0                                                                                                    |
| Workspace DSP tests             | `cargo test -p stratum-dsp --no-fail-fast`                                                  | exit 0                                                                                                    |
| Release build                   | `cargo build --release`                                                                     | exit 0                                                                                                    |
| MCP smoke                       | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000` | exit 0; no protocol violations                                                                            |
| Documentation contract          | commands in `docs/workflows/doc-drift/README.md`                                            | tests and site build exit 0; live schemas and embedded SOPs agree                                         |

## Scope

**In scope — source and test files**:

- `src/domain/classification/model.rs`
- `src/domain/classification/engine.rs`
- `src/application/classification/evidence.rs`
- `src/application/classification/classify.rs`
- `src/application/classification/evaluate.rs` (new, only if a pure benchmark
  aggregation module is needed)
- `src/application/classification/mod.rs`
- `src/mcp/classification/handlers.rs`
- `src/mcp/tests/classification.rs`
- `src/mcp/classification/fixtures/golden_genres.json` only to add audited,
  canonical ground-truth rows; never copy private paths, IDs, or provider
  payloads into it
- `src/mcp/server.rs` only for descriptions affected by the review contract

**In scope — public documentation if handler output/review semantics change**:

- `site/src/content/docs/mcp-tools/classification-staging.mdx`
- `site/src/content/docs/workflows/genre-classification.mdx`
- `site/src/content/docs/workflows/genre-audit.mdx`
- `site/src/partials/sops/genre-classification.mdx`
- `site/src/partials/sops/genre-audit.mdx`
- `site/src/data/tool-reference.mjs` only if required by the existing
  documentation contract

**Reviewer-owned plan artifact**:

- `plans/README.md`

**Out of scope**:

- Restoring Beatport, replacing it with a new provider, or making a network
  call from classification or benchmark execution.
- Bandcamp genre scoring, MusicBrainz genre scoring, taxonomy expansion, label
  mapping changes, BPM-range changes, and broad numeric weight retuning.
- Plan 035's cache-readiness states, profile-version migration, calibration
  readiness, or MusicBrainz label hydration.
- Deleting or migrating legacy Beatport cache rows. They are inert local state
  and useful for rollback/A/B evaluation.
- Changing album/year backfill precedence or release automation.
- Direct writes to Rekordbox `master.db`. User-visible metadata changes remain
  staged through `ChangeManager` and exported through XML.

## Git workflow

- Branch: `codex/034-make-classification-confidence-source-aware`
- Preferred commit: `fix(classify): make confidence source-aware`
- A second docs-only commit is acceptable if needed:
  `docs(classify): document reviewable confirmations`
- Do not push, open a pull request, deploy locally, release, or purge caches
  without separate operator authorization.
- The reviewer owns `plans/README.md`; do not edit the tracker in an executor
  worktree.

## Steps

### Step 1: Replace the fixture check with an actual classifier baseline

Before changing classification behavior, add pure metric aggregation in
`src/application/classification/evaluate.rs` (or an equivalently narrow module)
and rewrite the ignored `golden_dataset_genre_accuracy` test so every matched
track is passed through the same `build_track_evidence` and classification
entry points used at runtime.

The benchmark must separate truth from input:

1. Resolve canonical ground truth from the verified playlist/fixture genre.
2. For predictive classification, clone the track and blank its current genre
   before building evidence. The target genre must never re-enter through the
   current-genre hint.
3. Report a `rules_only` mode with no persisted `ProfileRegistry`; this is the
   primary non-leaky metadata/audio-rule baseline.
4. Report a `deployed_registry_diagnostic` mode separately if a stored registry
   exists, clearly marking it non-acceptance evidence because its training
   overlap may be unknown.
5. If calibrated-profile accuracy is reported as acceptance evidence, use
   deterministic stratified folds: build a registry only from training folds
   and score the held-out fold. Never train and score a track in the same fold.
6. Do not mutate the Rekordbox DB, internal store, fixture, or audio cache.

Emit one structured JSON summary containing at least:

- evaluated, skipped, and canonical-label counts;
- exact accuracy and same-family accuracy among recommendations;
- abstention rate (`genre: null`);
- manual-review rate (`Low` or `Insufficient`);
- precision per confidence tier;
- confusion pairs sorted by count;
- results stratified by usable source groups and Discogs match quality; and
- the classifier/profile/analyzer versions available at runtime.

Add deterministic unit tests for metric denominators, abstentions, family
accuracy, confidence precision, and zero-row handling. The live benchmark may
remain `#[ignore]` because it depends on the user's private library; the metric
tests may not.

Run the benchmark against the removal base and save only its aggregate output
in executor/review notes. Do not commit track identities or paths. Do not set a
new accuracy threshold from the nine-row fixture. If the private benchmark
cannot run, STOP before changing weights or confidence semantics and report the
missing prerequisite.

**Verify**:

```bash
cargo test -p reklawdbox classification::evaluate -- --nocapture
cargo test -p reklawdbox golden_dataset_genre_accuracy -- --ignored --nocapture
```

Expected: pure tests exit 0; the real benchmark executes the classifier and
prints non-empty aggregate metrics. The predictive mode cannot read the target
genre through `current_genre` or a registry trained on the same held-out row.

### Step 2: Carry provider provenance and match quality into votes

In `model.rs` and `evidence.rs`, replace the ambiguous effective-label and
`has_discogs` representation with typed internal provenance sufficient to
answer:

- whether a cache row is an exact match, fuzzy match, no-match, or unusable;
- whether the mapped label came from Rekordbox metadata or the same Discogs
  response as the style list; and
- which independent source group produced each vote.

Use these independence groups:

1. **Discogs** — styles and any label copied from the same Discogs response;
2. **Rekordbox label** — a non-empty library label, provided it is not merely a
   normalized duplicate of the cached Discogs label;
3. **Audio** — calibrated affinities and rule-based audio evidence together.

`current_genre` is not a source group. BPM plausibility, effective BPM, era,
and family rules are modifiers/constraints, not independent sources.

The internal `GenreVote` must carry a typed group rather than relying on its
human-readable source string. Preserve readable evidence strings. Ranking may
still sum multiple contributions, but confidence must use the unique groups
supporting the final winner.

Carry `EnrichmentCacheEntry.match_quality` into `TrackEvidence`. Unknown or
malformed qualities must fail closed as unusable and add a diagnostic flag;
they must not be silently treated as exact. A fuzzy Discogs match may support a
recommendation but cannot, by itself or through its own response label, produce
High confidence.

Add red tests first for:

- Discogs style plus Discogs fallback label counts as one group and caps at
  Medium;
- a distinct Rekordbox label plus Discogs can satisfy two-group consensus;
- the same normalized Rekordbox/Discogs label is conservatively correlated;
- fuzzy Discogs-only evidence cannot be High;
- Discogs plus independent audio may be High when all existing plausibility
  rules agree; and
- malformed match quality does not create provider evidence.

Do not change the current numeric ranking weights in this step unless the
baseline exposes a concrete regression and the reviewer approves the exact
threshold change. The required invariant is: **High confidence needs at least
two independent source groups supporting the final genre**.

**Verify**:

```bash
cargo test -p reklawdbox domain::classification -- --nocapture
```

Expected: every provenance test passes; no single provider payload or duplicate
label can produce High confidence.

### Step 3: Demote current genre from evidence to a bounded hint

Remove `current-genre` votes from `gather_votes`. Resolve current-genre tokens
once and use them only after evidence-backed ranking:

- zero tokens: no effect;
- one unambiguous token with no independent recommendation: return it only as a
  Low-confidence normalization hint, with a stable `current-genre-only` (or
  equivalently documented) flag and review hint;
- multiple tokens with no independent winner: return no genre with
  Insufficient confidence;
- one token matching exactly one candidate in a genuine evidence tie: it may
  select that candidate, but must add a tie-break flag and must not raise the
  pre-hint confidence tier; and
- a canonical current genre matching an evidence-backed recommendation still
  yields `ClassificationAction::Confirm` through the existing action resolver.

Keep an evidence string that explains the hint without describing it as an
independent vote. Add tests proving current-only, multi-token, tie-break, and
no-confidence-increase behavior.

**Verify**:

```bash
cargo test -p reklawdbox current_genre -- --nocapture
cargo test -p reklawdbox classification -- --nocapture
```

Expected: current metadata cannot manufacture Medium/High confidence; bounded
normalization and tie-break behavior remains visible and reviewable.

### Step 4: Make review eligibility confidence-aware across every handler

Add one canonical helper on `ClassificationResult`, such as
`review_required()`, that returns true for Low or Insufficient confidence
regardless of `ClassificationAction`. Use it consistently in:

- full `classify_tracks` results and `needs_review`;
- compact `classify_tracks` results;
- `audit_genres(include_confirmed=false)` visibility;
- `dispatch_genre_review` artist groups; and
- summary counts.

Preserve the default omission of High/Medium confirmations. Preserve
`include_confirmed=true` as the way to include all confirmations. Add an
additive `review_required` summary count if a count is needed; do not redefine
the existing `manual_review` action count.

Auto-staging must remain confidence-tier-controlled through `ChangeManager`.
It must not stage a Confirm no-op, an Insufficient result, or any result without
a recommended genre. Add handler regressions for Low Confirm and Insufficient
Confirm visibility in full, compact, audit, and dispatch outputs, plus a
regression proving auto-stage still stages only real changes at requested
tiers.

Update live schema descriptions, workflow pages, and both embedded SOPs to say
that a weak confirmation still needs review. Do not duplicate filtering logic
in prose; name the confidence rule.

**Verify**:

```bash
cargo test -p reklawdbox classification -- --nocapture
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
```

Expected: weak confirmations appear everywhere reviewable tracks are promised;
strong confirmations remain hidden by default; no direct DB write is added.

### Step 5: Restore provider-independent branch coverage

Recreate direct engine tests for behavior that survived the Beatport removal
but lost test coverage when provider-shaped fixtures were deleted:

- conflicting enrichment resolved by audio;
- same-family BPM override floors at Medium;
- cross-family BPM override downgrades to Low;
- confirmation of an existing canonical genre; and
- representative multi-way Discogs ambiguity remains Insufficient.

Construct `TrackEvidence` directly from Discogs, label provenance, audio, and
current-genre fields. Do not mention Beatport in test names, fixtures, comments,
or evidence strings. If the new source-independence rule changes an obsolete
expected tier, assert the new invariant explicitly rather than weakening the
test to "not High."

**Verify**:

```bash
cargo test -p reklawdbox domain::classification -- --nocapture
```

Expected: the BPM override and ambiguity branches are directly covered without
any removed-provider fixture.

### Step 6: Re-run the benchmark, document the delta, and run all gates

Run the same real benchmark command used in Step 1. Compare aggregate before
and after metrics by evidence stratum. The expected product change is reduced
false certainty: single-source and fuzzy-only High counts fall to zero, and
weak confirmations move into review. A higher manual-review rate is acceptable
if precision is preserved; do not tune weights merely to restore the previous
coverage percentage.

Run the full runtime and documentation gates:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo test -p stratum-dsp --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
git diff --check
git status --short
```

Expected: every command exits 0; the MCP smoke has no protocol violations; the
doc contract matches the live binary; the diff is limited to this plan's scope.

## Test plan

- Pure unit tests cover source grouping, duplicate labels, fuzzy/malformed
  matches, current-genre hints, confidence caps, and BPM modifiers.
- MCP handler tests cover full/compact/audit/dispatch visibility and staging
  safety for weak Confirm results.
- Benchmark metric tests use synthetic rows and have no private-library,
  network, or filesystem dependency.
- The ignored real benchmark calls the classifier, withholds truth from input,
  prevents training/test leakage, and emits only aggregate data.
- The existing Stratum real-audio v1 benchmark remains frozen and DSP-only.
- The full crate and DSP suites protect classification and audio behavior; the
  doc-drift workflow protects public tool/SOP claims.

## Implementation results (2026-07-14)

Implemented on the exact user-authorized uncommitted post-Beatport-removal
snapshot. This did not satisfy the plan's clean-integrated-base prerequisite,
but active Rust was Beatport-free and the composite passed `git diff --check`.

The pre-change withheld-truth rules-only benchmark evaluated 9 canonical rows:
44.4% exact, 66.7% same-family, 77.8% manual review. The pre-change deployed
registry diagnostic was 33.3% exact. After implementation, rules-only and the
safe deployed diagnostic both measured 44.4% exact, 66.7% same-family, and
77.8% manual review; no recommendation was High confidence. The legacy stored
registry was excluded from acceptance evidence because it predates compatible
profile metadata. These are aggregate local diagnostics, not release
thresholds.

All targeted tests, full crate and DSP suites, release build, CLI smoke, MCP
smoke, site build, and documentation contract passed. No taxonomy, broad vote
weight, analyzer schema, provider, direct Rekordbox write, deploy, or release
was added.

## Done criteria

- [ ] The implementation starts from a clean reviewed base with no active
      Beatport Rust surface.
- [x] A pre-change real classifier baseline was captured with ground truth
      withheld from classifier input.
- [x] High confidence requires at least two independent source groups.
- [x] Discogs styles plus its own fallback label count as one source group.
- [x] Exact/fuzzy/invalid Discogs match quality is represented and tested.
- [x] Audio rules and calibrated profiles count as one Audio source group.
- [x] Current genre is a hint/tie-breaker, never an independent confidence
      source.
- [x] Low/Insufficient Confirm results remain visible in all review surfaces.
- [x] High/Medium confirmations remain hidden by default and all confirmations
      remain available with `include_confirmed=true`.
- [x] Provider-independent BPM override and ambiguity regressions are restored.
- [x] No weight/taxonomy tuning was made without benchmark evidence and
      explicit review.
- [x] No new provider, cache purge, Rekordbox write path, deploy, or release was
      introduced.
- [x] Targeted tests, full gates, release build, MCP smoke, site build, and doc
      contract all pass.
- [x] Post-change aggregate metrics are recorded without private track data.

## STOP conditions

Stop and report rather than improvising if:

- Beatport is still present in active Rust code or the removal base is dirty;
- the live vote/confidence/filter semantics no longer match the excerpts;
- the real benchmark cannot run because the private verified corpus, Rekordbox
  DB, or internal store is unavailable;
- ground-truth genre would enter predictive input through current genre,
  persisted profiles, fixture construction, or another hidden path;
- source independence cannot be represented without a breaking response-schema
  replacement rather than additive/internal fields;
- a proposed fix requires network access during classification or benchmark
  execution;
- a proposed fix changes taxonomy, label mappings, BPM ranges, or broad numeric
  weights without reviewed benchmark evidence;
- handler changes would bypass `ChangeManager`, stage no-op confirmations, or
  write directly to `master.db`;
- files outside Scope require semantic edits; or
- two reasonable attempts cannot make a required verification command pass.

## Maintenance notes

Treat evidence independence as a semantic invariant, not a numeric threshold.
Future providers must declare their source group and match-quality behavior
before contributing confidence. Multiple fields from one provider response do
not become consensus merely because they enter different rules. Derived audio
rules and calibrated profiles remain correlated unless a future benchmark
proves a genuinely independent signal.

The real benchmark is the prerequisite for later classifier work. Keep its
truth-withholding and no-leakage checks explicit, keep private rows out of the
repository, and add new stratified metrics before changing taxonomy or weights.
Do not delete inert legacy Beatport cache rows as part of classifier cleanup;
their small local footprint is outweighed by rollback and A/B value.
