# Plan 037: Require Essentia evidence for full classification

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 4aad526..HEAD -- \
>   src/domain/classification \
>   src/application/classification \
>   src/mcp/classification \
>   src/mcp/analysis/coverage.rs \
>   src/mcp/tests/classification.rs \
>   src/adapters/state \
>   README.md \
>   site/src/partials/sops/genre-classification.mdx \
>   site/src/partials/sops/genre-audit.mdx \
>   site/src/content/docs/mcp-tools/enrichment-analysis.mdx \
>   site/src/content/docs/reference/environment-variables.md \
>   site/src/content/docs/concepts/architecture.mdx \
>   site/src/content/docs/getting-started/index.mdx \
>   docs/workflows scripts/check-doc-contract.mjs
> ```
>
> Plan 036 must already be reviewed and integrated. Compare the live cache
> identity, profile schema, classifier result, auto-stage filter, calibration
> sample selection, coverage output, and public documentation with the
> excerpts below. If Plan 036 is absent, the runtime remains floating, another
> plan changed classification ranking, or a full/degraded contract already
> exists with different semantics, STOP and report the drift.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: 036 (and transitively 034–035)
- **Category**: correctness / classification / capability readiness
- **Planned at**: commit `4aad526`, 2026-07-17

### Execution history

- **2026-07-18 — COMPLETED on `main` at `06caf71`.** Classification now keeps
  typed Stratum/Essentia readiness through inference, calibration, coverage,
  staging, benchmark, and public output. Full requires two fresh valid rows;
  Degraded is reasoned, capped at Low, review-required, and ineligible for
  auto-staging. Profile schema v2 and the exact runtime/training identities are
  fail-closed and non-destructive. Three independent reviews approved the code
  and public contract, and every mandatory workspace/release/MCP/site/docs gate
  passed.
- **2026-07-18 — LOCAL MIGRATION COMPLETED WITH PROFILE ROLLBACK.** The managed
  runtime refreshed all 624 `genre_verified` tracks to Full with zero missing,
  invalid, analyzer-failure, or cache-write rows. Calibration built 13 v2
  profiles from all 624 scorable samples, but its explicitly non-acceptance
  nine-track diagnostic reduced same-family accuracy from 66.7% rules-only to
  55.6%. The profile tables were therefore restored transactionally from the
  pre-calibration snapshot while retaining all fresh audio rows: the 28 legacy
  rows remain preserved but incompatible and no profile registry is deployed.
  Final Full rules-only results were 33.3% exact and 66.7% same-family. The old
  Plan 034 cache-light baseline was 44.4% exact and 66.7% same-family; readiness
  alone is separately proven not to reorder candidates, so the exact delta is
  attributed to admitting newly refreshed real audio rather than the mode
  policy. Do not redeploy profiles until a larger held-out benchmark supports
  them; do not tune this readiness migration around the nine-row diagnostic.
- **2026-07-17 — NOT DISPATCHED.** Plan 036 did not pass execute review after
  two revision rounds and remains uncommitted in its isolated worktree. This
  plan's precondition therefore fails. Do not implement or test Plan 037 until
  Plan 036 is completed, reviewed, committed on its isolated branch, and its
  managed-runtime/cache-v3 contract is available as the base.

## Why this matters

The classifier currently collapses fresh Stratum evidence, fresh Essentia
evidence, partial audio evidence, invalid payloads, and no audio evidence into
one `Option<AudioFeatures>`. A track can therefore receive the same public
classification shape on two machines even though one result used both
analyzers and the other silently ran on partial evidence. The auto-stage path
filters on confidence and action but does not independently require complete
audio evidence; a caller that opts into Low confidence can stage a partial
classification.

Making Essentia globally mandatory would overcorrect. Reklawdbox's library,
export, metadata, transition, and pool workflows can still provide useful
behavior without it. The correct boundary is capability-specific:

- **full classification** requires fresh, valid Stratum and Essentia cache
  rows for the track;
- **degraded classification** still returns a recommendation when either row
  is missing or invalid, but caps confidence at Low, requires review, reports
  why it is degraded, and is never auto-staged; and
- non-classification features continue to start and run without Python.

This plan makes that distinction a typed domain contract, uses it consistently
in inference, calibration, coverage, staging, and documentation, and validates
the change against the existing real-audio benchmark. It does not retune the
taxonomy, weights, or genre ranking.

## Current state

### Evidence extraction erases backend readiness

`src/application/classification/evidence.rs` parses Stratum and Essentia
payloads independently, but `extract_audio_features` returns only
`Option<AudioFeatures>`. It returns `Some` when either payload contributes and
logs invalid JSON without preserving that failure as result data.
`build_track_evidence` then sets only `has_audio = audio.is_some()`.

`src/domain/classification/model.rs` likewise exposes no backend-specific
status or classification mode. `ClassificationResult::review_required()` is
currently derived only from Low or Insufficient confidence.

This means `None`, Stratum-only, Essentia-only, and invalid cache states cannot
be distinguished after evidence construction.

### The engine has an early result path

`src/domain/classification/engine.rs:111-115` returns early for its audio veto
before the common result construction used by the normal scoring path. Adding
mode fields only to the normal tail would leave this path able to bypass the
new confidence and review policy. Readiness finalization must be one domain
operation applied to every return path.

### Auto-stage trusts confidence alone

`src/mcp/classification/handlers.rs` currently selects staged changes from
genre, action, confidence, and the caller's requested auto-stage levels. A
caller may include Low confidence, so reducing partial evidence to Low is not
by itself a staging guard. The handler needs a positive `Full`-mode
eligibility check in addition to the existing filters.

Manual staging through `ChangeManager` remains available after a human reviews
the evidence. This plan changes only automatic classification staging and
never writes directly to Rekordbox.

### Calibration accepts generic audio rather than complete audio

`src/application/classification/calibrate.rs` currently accepts a sample when
`extract_audio_features` returns `Some` and
`profiles::has_scorable_optional_features` succeeds. Stratum-only optional
values can satisfy that test. Coverage reports Stratum and Essentia separately,
but prototype readiness is still computed from the generic sample set.

`src/domain/classification/profiles.rs` declares
`PROFILE_SCHEMA_VERSION = "1"`. Requiring both analyzers changes the training
sample contract, so profiles built under v1 must remain stored for rollback but
must not load as current v2 profiles.

### Public guidance still calls Essentia optional without the boundary

The README, Genre Classification and Genre Audit SOPs, environment reference,
architecture/getting-started material, and enrichment-analysis reference
currently describe Essentia as optional or non-gating. That remains true for
the process as a whole, but is incomplete for classification quality and
auto-staging. Documentation must distinguish core availability, explicit
Stratum-only analysis, and full classification readiness rather than replacing
all optional-language mechanically.

## Contract to implement

| Cache evidence for the track               | Mode       | Maximum confidence               | Review          | Auto-stage                 |
| ------------------------------------------ | ---------- | -------------------------------- | --------------- | -------------------------- |
| Fresh valid Stratum + fresh valid Essentia | `full`     | Existing engine result           | Existing policy | Existing filters may allow |
| Stratum missing or invalid                 | `degraded` | `low`                            | Required        | Never                      |
| Essentia missing or invalid                | `degraded` | `low`                            | Required        | Never                      |
| Both missing or invalid                    | `degraded` | `low` or existing `insufficient` | Required        | Never                      |

A **fresh valid row** means the current cache identity resolves and its JSON
payload parses into the backend's typed features. A valid Essentia payload may
still have individual detector fields set to `null`; preserve that missingness
and do not convert it to zero. Freshness must continue to use the exact track
cache identity established by Plan 035 and the analyzer versions left by Plan
036.

This is a readiness and disclosure change, not a new vote. The presence of an
Essentia row does not itself increase a genre score, and degraded mode must not
change candidate ordering before the confidence cap is applied.

## Commands you will need

| Purpose                   | Command                                                                                                                                                                                             | Expected on success                                               |
| ------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Domain mode policy        | `cargo test -p reklawdbox classification_mode -- --nocapture`                                                                                                                                       | exit 0; every engine exit applies the same mode policy            |
| Evidence combinations     | `cargo test -p reklawdbox classification_audio_readiness -- --nocapture`                                                                                                                            | exit 0; fresh/missing/invalid combinations are distinct           |
| Auto-stage safety         | `cargo test -p reklawdbox auto_stage_degraded -- --nocapture`                                                                                                                                       | exit 0; no degraded result enters `ChangeManager`                 |
| Calibration compatibility | `cargo test -p reklawdbox classification_calibration -- --nocapture`                                                                                                                                | exit 0; only complete samples build v2 profiles                   |
| Classification suite      | `cargo test -p reklawdbox classification -- --nocapture`                                                                                                                                            | exit 0; existing ranking and provenance regressions pass          |
| Format                    | `cargo fmt --check && dprint check`                                                                                                                                                                 | exit 0                                                            |
| Lint                      | `cargo clippy --workspace --all-targets -- -D warnings`                                                                                                                                             | exit 0                                                            |
| Tests                     | `cargo test --workspace --no-fail-fast`                                                                                                                                                             | exit 0                                                            |
| Release and smoke         | `cargo build --release && ./target/release/reklawdbox --version && ./target/release/reklawdbox --help && node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000` | exit 0; no MCP protocol violations                                |
| Public docs               | commands in `docs/workflows/doc-drift/README.md`                                                                                                                                                    | parser tests, site build, live contract, and semantic review pass |

## Scope

**In scope — domain and application**:

- `src/domain/classification/model.rs`
- `src/domain/classification/engine.rs`
- `src/domain/classification/profiles.rs`
- `src/domain/classification/tests.rs`
- `src/application/classification/evidence.rs`
- `src/application/classification/classify.rs`
- `src/application/classification/calibrate.rs`
- `src/application/classification/evaluate.rs` only if required to stratify the
  existing benchmark report by classification mode
- adjacent classification test modules only when required by current layout

**In scope — MCP presentation and safety**:

- `src/mcp/classification/handlers.rs`
- `src/mcp/classification/transport.rs`
- `src/mcp/analysis/coverage.rs`
- `src/mcp/tests/classification.rs`
- existing MCP analysis/coverage tests required by the additive result fields

**In scope — cache/profile compatibility**:

- `src/adapters/state/analysis.rs` and its tests only as required to preserve
  old profile rows and expose v2 incompatibility
- no Essentia or Stratum cache-version bump; Plan 036 owns Essentia v3

**In scope — public documentation and contracts**:

- `README.md`
- `site/src/partials/sops/genre-classification.mdx`
- `site/src/partials/sops/genre-audit.mdx`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `site/src/content/docs/reference/environment-variables.md`
- `site/src/content/docs/concepts/architecture.mdx`
- `site/src/content/docs/getting-started/index.mdx`
- genre classification/audit workflow pages that repeat the old readiness
  claim
- `scripts/check-doc-contract.mjs` and its test only for structural assertions
  covering the new public contract
- `plans/README.md` for the status row only

**In scope — opt-in local validation, never committed**:

- current read-only Rekordbox library data and private audio for the existing
  ignored benchmark only
- Reklawdbox-owned cache and calibration databases through existing supported
  tools

**Out of scope**:

- Refusing to start the MCP server, CLI, or unrelated tools when Essentia is
  absent.
- Changing genre taxonomy, scoring weights, thresholds, candidate ordering,
  label fallbacks, current-genre hint semantics, or independent-source rules.
- Changing transition or pool scoring to require Essentia. They continue to
  degrade according to their existing contracts.
- Automatically running audio analysis from classification or calibration.
  Readiness tools report the work; the operator invokes existing batch tools.
- Replacing Plan 036's managed environment, package pin, or cache version.
- Requiring every track in the library to have both analyzers before any genre
  can calibrate. Readiness is per sample and per genre.
- Deleting or rewriting old profile/cache rows.
- Direct SQL writes to Rekordbox `master.db`, automatic XML import, audio tag
  edits, or audio-file mutations.
- Deploying the Homebrew binary, pushing, opening a PR, or releasing.

## Git workflow

- Branch: `codex/037-require-essentia-classification`
- Use Conventional Commits; preferred final message:
  `feat(classification): require Essentia for full readiness`.
- Stage only in-scope source, tests, and documentation. Never stage cache
  databases, private audio, ignored MCP configuration, or benchmark artifacts.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Preserve backend-specific readiness in the domain model

Add serializable, snake-case domain types with names equivalent to:

```rust
enum AudioBackendStatus {
    Fresh,
    Missing,
    Invalid,
}

enum ClassificationMode {
    Full,
    Degraded,
}

enum ClassificationDegradedReason {
    MissingStratum,
    InvalidStratum,
    MissingEssentia,
    InvalidEssentia,
}
```

`TrackEvidence` must retain Stratum and Essentia status separately alongside
the merged optional feature values. `ClassificationResult` and its compact MCP
representation must add `mode` and `degraded_reasons`. Keep the change
additive: preserve existing fields and their meanings.

Put the derivation of `ClassificationMode` and reasons in one pure domain
function. Reasons must be stable and deterministically ordered (Stratum before
Essentia; missing/invalid according to the actual status). Full mode is
possible only when both statuses are `Fresh`.

Make `review_required()` return true for every degraded result in addition to
the existing confidence rule. Add a domain method such as
`is_auto_stage_eligible()` that is true only for Full mode; transports must not
reconstruct this policy from strings.

Add serialization and compact-shape tests for all statuses, Full, Degraded,
multiple ordered reasons, and backward-compatible existing fields.

**Verify**:

```bash
cargo test -p reklawdbox classification_mode -- --nocapture
```

Expected: domain tests pass and prove deterministic public values without any
I/O fixture.

### Step 2: Extract features and status without erasing missingness

Refactor `src/application/classification/evidence.rs` so each backend parser
returns both its typed optional features and `AudioBackendStatus`:

- no current cache row -> `Missing`;
- a current row whose payload cannot be parsed -> `Invalid`;
- a current row that parses, even when individual optional measurements are
  null -> `Fresh`.

Merge values with the existing precedence and missingness rules. Do not infer
`Fresh` merely because one merged `AudioFeatures` field is populated. Do not
convert absent detector fields into zero or copy a value from the other
backend just to satisfy readiness.

Update every caller, including classification, calibration, coverage, and
benchmark code, to consume one shared readiness interpretation rather than
parsing cache state independently.

Add a combination matrix covering: neither row, Stratum only, Essentia only,
both valid, invalid Stratum with valid Essentia, valid Stratum with invalid
Essentia, both invalid, and valid sparse/null payloads. Include freshness tests
using the exact track identity so a stale row is `Missing`, not `Fresh`.

**Verify**:

```bash
cargo test -p reklawdbox classification_audio_readiness -- --nocapture
```

Expected: every cache combination produces the intended status, merged
features, mode, and reason without private data.

### Step 3: Apply degraded policy to every classifier exit

Centralize result finalization in the domain engine and send both the normal
scoring path and the early audio-veto path through it. Finalization must:

1. derive Full/Degraded and reasons from the evidence statuses;
2. preserve existing candidate ordering, recommended genre, action, and
   evidence provenance;
3. preserve `Insufficient` when the engine already produced it;
4. cap High or Medium at Low for Degraded mode;
5. add a stable degraded flag and a concise review hint listing the missing or
   invalid backend(s); and
6. make review required for Degraded mode regardless of the pre-cap result.

Do not turn backend presence into an evidence vote. A Full result may still be
Low or Insufficient under the existing source-aware confidence rules.

Add paired tests with identical scoring evidence and different backend status
to prove: recommendation/candidate order do not change; only mode, reasons,
confidence cap, flags, hint, and review status change. Include the audio-veto
path explicitly so it cannot bypass finalization.

**Verify**:

```bash
cargo test -p reklawdbox classification_mode -- --nocapture
cargo test -p reklawdbox classification -- --nocapture
```

Expected: new policy tests and the existing genre/provenance regressions pass.

### Step 4: Make auto-staging require Full mode

In `src/mcp/classification/handlers.rs`, require the domain-owned Full-mode
eligibility predicate before a result may enter the existing auto-stage
filter. This check is mandatory even when the caller explicitly includes Low
confidence in its requested auto-stage levels.

Keep manual review and staging available through the existing
`ChangeManager`/XML path. Do not change confirmations, dry-run behavior,
pending-change storage, or the read-only database boundary.

Extend batch summaries additively with:

- counts for `full` and `degraded` results;
- degraded-reason counts; and
- a count such as `auto_stage_skipped_degraded`.

Add tests proving:

- Degraded High-before-cap and Degraded Low results never stage;
- Full results still obey the existing genre/action/confidence filters;
- requesting Low does not bypass the mode guard;
- weak confirmations and no-op results retain existing behavior; and
- no pending change is created for a skipped degraded result.

**Verify**:

```bash
cargo test -p reklawdbox auto_stage_degraded -- --nocapture
```

Expected: all degraded cases are reported but none reach `ChangeManager`.

### Step 5: Build profiles only from complete classification audio

Change calibration sample selection so a track contributes to a genre
prototype only when:

1. both backend statuses are `Fresh`;
2. the existing scorable-feature predicate passes; and
3. all existing verified-genre and candidate-quality rules pass.

Do not impose a global 100% coverage gate. Report readiness per track and per
genre, then apply the existing minimum-track and scorable/candidate checks to
the complete sample subset. This preserves useful calibration for ready genres
without letting partial evidence silently train the profile.

Add calibration/coverage fields for at least:

- `tracks_with_complete_classification_audio`;
- `missing_required_stratum` and `invalid_required_stratum`;
- `missing_required_essentia` and `invalid_required_essentia`; and
- per-genre complete/scorable counts and readiness reason.

Bump `PROFILE_SCHEMA_VERSION` from `"1"` to `"2"`. Preserve old v1 rows, but
load them as incompatible. Preserve the existing atomic safety rule: a failed
or empty calibration must not replace the current non-empty compatible
registry. If no complete samples exist, return a clear actionable error that
points to coverage and batch analysis; do not run analysis implicitly.

Add tests for partial samples excluded, complete sparse samples interpreted by
the existing scorable predicate, mixed genre readiness, v1 incompatibility,
old-row preservation, and failed recalibration retaining the previous valid
registry.

**Verify**:

```bash
cargo test -p reklawdbox classification_calibration -- --nocapture
cargo test -p reklawdbox profile -- --nocapture
```

Expected: only complete samples train v2 profiles and failure is non-
destructive.

### Step 6: Report classification readiness from the same contract

Extend `cache_coverage` additively with a classification readiness block that
uses the same evidence/status helper as classification. Report Full and
Degraded track counts, degraded reasons, and managed Essentia runtime
availability. Keep raw backend cache counts so operators can distinguish
runtime absence from pending or invalid rows.

Align `calibration_coverage` with Step 5's complete-sample rule. Its readiness
and actionable next step must agree with what `calibrate_audio_profiles` will
actually accept. Do not claim ready merely because generic optional audio
features exist.

For a mixed library, report the ready subset and next work rather than failing
the coverage call. Keep selectors, pagination, and current cache identity
behavior unchanged.

Add tests proving coverage, calibration, and classification assign the same
mode/reasons to a shared fixture.

**Verify**:

```bash
cargo test -p reklawdbox coverage -- --nocapture
cargo test -p reklawdbox classification_calibration -- --nocapture
```

Expected: all three surfaces agree on complete, missing, invalid, and stale
fixtures.

### Step 7: Publish the capability-specific contract

Update in-scope public docs and embedded SOPs to say exactly:

- Essentia is required for **Full classification**, not for Reklawdbox process
  startup or unrelated workflows;
- missing, stale, or invalid Stratum/Essentia evidence produces Degraded mode,
  maximum Low confidence, required review, and no auto-staging;
- a valid sparse payload counts as present while individual missing detector
  fields remain unknown;
- `cache_coverage` and `calibration_coverage` show the work required before
  calibration or Full classification;
- explicit `--stratum-only` analysis remains supported but cannot by itself
  produce Full classification readiness; and
- transition and pool scoring keep their existing graceful-degradation
  behavior.

Use the managed environment and pinned analyzer language established by Plan
036. Do not reintroduce repository-local venv instructions or describe the
expert override as standard setup.

Update doc-contract assertions for the Full/Degraded distinction, no-auto-stage
rule, and capability-specific optionality. Because SOPs are embedded with
`include_str!`, rebuild the binary and site before accepting the change.

**Verify**:

```bash
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
cargo build --release
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
```

Expected: structural checks, embedded help, and public site all expose the same
capability boundary.

### Step 8: Refresh local evidence and run the real benchmark

Perform this opt-in step only after Plan 036's managed interpreter is healthy
and while read-only access to the local Rekordbox library and private audio is
available. Do not add this data dependency to mandatory tests.

1. Build the release binary from the completed source.
2. Run `cache_coverage` for the `genre_verified` selector/playlist and record
   Full/Degraded counts and reasons.
3. Refill stale Stratum and Essentia rows through the existing paginated
   `analyze_audio_batch` workflow at low/background priority. Use
   `skip_cached: true`, follow `page.next_offset`, and stop only when the
   selected set converges. Do not invent a direct cache write or use
   `skip_cached: false` for already-current rows.
4. Re-run `cache_coverage`; investigate any remaining invalid rows rather than
   treating them as missing.
5. Run `calibration_coverage`, then `calibrate_audio_profiles` only when its
   reported prerequisites are satisfied.
6. Run the existing ignored real classifier benchmark and save only its
   aggregate output outside the repository unless the existing fixture policy
   explicitly allows a checked-in sanitized artifact.

Compare exact-genre and same-family recommendation metrics with the Plan 034
baseline. This plan may change mode, confidence, review, and auto-stage
statistics, but must not silently change recommendation/candidate ordering. If
recommendation metrics move, STOP and diagnose before tuning anything.

**Verify**:

```bash
cargo test -p reklawdbox golden_dataset_genre_accuracy -- --ignored --nocapture
git status --short
```

Expected: the benchmark runs against fresh Full evidence where available;
recommendation metrics do not regress from readiness-only changes; degraded
rows are clearly separated; private/cache artifacts are not staged.

If the private library or audio is unavailable, report this opt-in validation
as not run. Do not convert private inputs into a mandatory test or manufacture
passing numbers.

### Step 9: Run full workspace and documentation gates

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
git diff --check
git status --short
```

Then run the semantic review prompt in
`docs/workflows/doc-drift/prompt.md` against classification, calibration,
coverage, environment, and SOP changes.

Expected: every command exits 0; existing recommendation/provenance tests
remain stable; MCP and docs expose the additive mode/reason fields; only
in-scope files and the status-row change are modified.

## Test plan

- Pure domain tests cover status-to-mode mapping, deterministic reasons,
  confidence capping, review policy, serialization, and auto-stage eligibility.
- Application fixtures cover every fresh/missing/invalid backend combination,
  including valid sparse payloads and stale exact-identity rows.
- Engine regressions prove normal and audio-veto exits share finalization and
  that readiness does not reorder candidates.
- Handler tests prove Degraded never auto-stages even when Low is explicitly
  requested, while Full continues through existing filters.
- Calibration tests require both backends per sample, preserve per-genre
  readiness, version profiles to v2, and retain valid state after failure.
- Coverage tests prove classification, calibration, and reporting interpret
  the same fixture identically.
- Mandatory tests require no private database, private audio, Essentia install,
  or network access.
- The existing ignored real benchmark validates the completed managed runtime,
  refreshed caches, mode distribution, and ranking non-regression locally.
- Documentation contract and semantic-review gates cover the capability-
  specific wording and embedded SOPs.

## Done criteria

- [x] `TrackEvidence` preserves separate Stratum and Essentia readiness.
- [x] Public results add stable `full`/`degraded` mode and deterministic
      degraded reasons without removing existing fields.
- [x] Full classification requires fresh, valid rows from both analyzers.
- [x] Degraded classification caps High/Medium at Low, requires review, and
      applies the same policy to every engine exit.
- [x] No degraded result can auto-stage, even when the caller requests Low;
      manual reviewed staging still uses `ChangeManager` and XML export.
- [x] Candidate ordering, recommended genre, taxonomy, and weights are
      unchanged by readiness alone.
- [x] Calibration samples require both analyzers, with readiness evaluated per
      track and per genre rather than as a global library gate.
- [x] `PROFILE_SCHEMA_VERSION` is `2`; v1 rows are preserved but incompatible;
      failed or empty calibration is non-destructive.
- [x] Coverage, calibration, classification, and benchmark surfaces use the
      same readiness interpretation.
- [x] Docs distinguish core availability from Full classification and preserve
      truthful Stratum-only, transition, and pool degradation behavior.
- [x] Existing MCP result fields remain backward-compatible and new summary
      fields make skipped degraded staging observable.
- [x] Mandatory tests are offline/private-data-free; the opt-in real benchmark
      is run when local data is available and recommendation metrics do not
      regress.
- [x] Full workspace, release, MCP smoke, site, documentation-contract, and
      semantic doc-drift gates pass.
- [x] No Rekordbox data, audio files, cache rows, profile rows, or unrelated
      worktree files are destructively modified.
- [x] `plans/README.md` status row is updated.

## STOP conditions

Stop and report rather than improvising if:

- Plan 036 is not reviewed and integrated, the managed Essentia runtime is
  ambiguous, or the expected Essentia v3 cache identity is absent;
- the requested behavior is actually to make the whole process fail at startup
  without Essentia rather than to make Full classification capability-specific;
- a Full/Degraded contract already exists with incompatible public values or a
  migration would need to remove existing fields;
- enforcing mode would require changing genre taxonomy, weights, thresholds,
  candidate ordering, current-genre hinting, or independent-source policy;
- any engine return path cannot be routed through one tested finalization
  policy;
- degraded auto-staging can still be reached through another handler or alias;
- profile v1 state would need deletion/rewrite rather than preservation as
  incompatible;
- calibration can only be made safe by requiring 100% global library coverage;
- mandatory verification requires private audio, a private Rekordbox database,
  network access, or a live Essentia install;
- the opt-in benchmark shows a recommendation/candidate-order regression from
  this readiness-only change;
- public docs would need to claim Essentia is mandatory for unrelated
  transition, pool, metadata, export, or startup workflows;
- files outside Scope require semantic edits; or
- two reasonable attempts cannot make a required verification command pass.

## Maintenance notes

Full classification readiness is the conjunction of two versioned analyzer
results, not merely proof that Python imports. Future Stratum or Essentia schema
bumps will naturally move affected tracks to Degraded until their cache rows
are refreshed. Keep this fail-closed and visible; do not add implicit stale-row
fallbacks.

Individual null detector fields remain missing data even in Full mode. Full
means both current payloads parsed, not that every optional feature exists or
that confidence must be High.

Keep the Full-mode auto-stage predicate in the domain contract so new
transports cannot accidentally reconstruct a weaker rule. Any future analyzer,
profile, or classification change should extend the shared readiness helper and
the combination matrix before changing public prose.

Plan 036 and this plan form one migration lane: environment reproducibility and
cache identity first; capability semantics, profile compatibility, evidence
refresh, and calibration second. Do not combine them into one unreviewable
commit or delete old rows to make the migration appear clean.
