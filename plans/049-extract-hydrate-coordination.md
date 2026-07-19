# Plan 049: Extract hydrate coordination from the CLI

> **Executor instructions**: Preserve CLI flags, prompts, progress wording,
> retry timing, cache compatibility, cancellation, exit status, and partial
> failure accounting. Move only transport-independent decisions and lifecycle
> ownership into `application/`. STOP if the CLI and MCP need different product
> behavior that cannot be represented by a small explicit policy. Update the
> tracker only after independent review and full verification.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/application/batch.rs \
>   src/application/cache_writer.rs \
>   src/application/analysis \
>   src/application/enrichment \
>   src/cli/hydrate.rs \
>   src/cli/runtime \
>   src/mcp/enrichment \
>   src/mcp/tests/enrichment
> ```
>
> The planning base already centralizes bounded workers and cache-write
> acknowledgements. Reconcile that code rather than recreating a second helper.
> STOP if cancellation, writer acknowledgement, retryability, or cached-error
> semantics changed after the planning commit.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: none
- **Category**: architecture / async lifecycle / CLI decomplexification
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

`src/cli/hydrate.rs` is 1,382 lines. More importantly,
`run_hydrate` spans lines 229-856 and owns ten different phases:

1. bootstrap and resource policy;
2. Rekordbox track selection;
3. per-provider cache selection;
4. estimates and startup rendering;
5. confirmation;
6. Discogs authentication;
7. cancellation and progress;
8. the combined cache writer;
9. provider/analysis task execution and joining; and
10. result accounting, rendering, and exit status.

Some of those are correctly CLI-owned. Cache selection, per-track provider
outcomes, combined cache persistence, worker completion, and final accounting
are not. The application layer already has `run_bounded_workers`,
`run_analysis_stage`, `run_enrichment_workers`, and acknowledged cache-write
types, but the CLI still implements a second Discogs outcome machine and uses
atomics as both live presentation state and final truth.

This plan makes the application report canonical and leaves prompts, spinners,
terminal style, CPU niceness, browser opening, and retry logging at the CLI
edge.

## Target ownership

### Application owns work selection

Add a concrete hydration selection model in
`application/enrichment/hydrate.rs`:

- `HydrationSelectionPolicy` — requested stages, retry-cached-errors flag, and
  whether a compatible Essentia runtime is available;
- `HydrationPlan` — selected Discogs identities, selected analysis jobs with
  exact `needs_stratum`/`needs_essentia` flags, cached/error counts, and total
  matched tracks; and
- `select_hydration_work` — one canonical cache interpretation using batched
  state reads where available.

Do not put CLI limits, colors, prompt state, `MultiProgress`, or strings in
these types. Do not make a generic selector framework.

### Application owns one per-track provider outcome

Expose one concrete application function for a Discogs hydration track. Both
MCP's bounded enrichment worker and CLI's worker wrapper must call it. Preserve
the current difference in lookup-failure persistence with an explicit
two-value policy:

- `LookupFailurePersistence::DoNotCache` for MCP; and
- `LookupFailurePersistence::CacheTerminalError` for CLI hydrate.

The application function owns normalization, serialization, cache
acknowledgement, auth-failure classification, match/no-match/error accounting,
and the optional terminal error row. The CLI supplies its existing retrying
lookup future and updates progress after the typed outcome returns.

This policy is intentionally narrow. Do not introduce hooks for arbitrary
providers or a trait hierarchy.

### Application owns the combined writer lifecycle

Move `HydrateCacheMsg`, its conversions, and
`run_hydrate_cache_writer` out of `cli/`. They persist Reklawdbox-owned state
and are transport-independent. Reuse `application/cache_writer.rs` request and
report types. Start/finish ownership must be represented by a session type that:

- closes its sender before awaiting the writer;
- reports writer spawn/join/open/write/ack failures separately;
- cannot be finished twice; and
- keeps cleanup alive if the CLI future is cancelled.

Do not merge this SQLite writer with the synchronous Essentia or backup process
runners.

### CLI owns interaction and observation

Convert `src/cli/hydrate.rs` to `src/cli/hydrate/` only after the application
extractions are green:

- `mod.rs` — CLI argument type, declarations, and `run_hydrate` entry only;
- `command.rs` — the short phase coordinator;
- `discogs.rs` — CLI auth, browser opening, and retry/backoff;
- `presentation.rs` — estimates, prompt, progress, and final summary; and
- `tests.rs` — CLI rendering/retry characterization.

The CLI may keep live atomic counters for presentation, but final exit status
must derive from the application reports plus explicit CLI task-join failures.

## Scope

**In scope**:

- `src/application/cache_writer.rs`
- `src/application/enrichment/hydrate.rs`
- `src/application/enrichment/model.rs`
- `src/application/enrichment/mod.rs`
- `src/application/analysis/job.rs` only if a narrow typed analysis outcome is
  needed; no analyzer semantics
- `src/cli/hydrate.rs` (replaced by `src/cli/hydrate/**`)
- `src/cli/hydrate/mod.rs`
- `src/cli/hydrate/command.rs`
- `src/cli/hydrate/discogs.rs`
- `src/cli/hydrate/presentation.rs`
- `src/cli/hydrate/tests.rs`
- `src/cli/mod.rs`
- `src/cli/runtime/cache_writer.rs` only to remove/move hydrate-specific code
- `src/mcp/enrichment/handlers.rs` and
  `src/mcp/enrichment/core.rs` only to call the canonical per-track outcome
- focused tests in `src/mcp/tests/enrichment/**`
- `tests/source_boundaries.rs` only for a narrow regression proving hydration
  coordination did not return to `cli/`
- `plans/README.md` status row only during execution

**Out of scope**:

- CLI flags, defaults, help, provider names, prompts, progress strings, retry
  count/backoff, estimates, output ordering, or exit categories.
- MCP schemas/results, provider network contracts, Discogs auth protocol,
  Bandcamp behavior, MusicBrainz behavior, or cache serialization.
- Changing cache keys, schemas, match-quality strings (`exact`, `fuzzy`,
  `none`, `error`), analyzer versions, or retry eligibility.
- Adding provider-wide traits, a generic workflow engine, or a new event bus.
- Moving terminal UI, browser launch, CPU niceness, signals, or CLI-only retry
  logging into `application/`.
- Live provider calls or mandatory private Rekordbox/audio fixtures.

## Steps

### Step 1: Characterize the current command contract

Add/retain bounded tests for:

1. provider parsing and all `hydrate --help` flags/defaults;
2. selection counts for cached success, cached `error` with retries enabled,
   cached `error` with retries disabled, stale/missing analysis, and missing
   Essentia;
3. Discogs match/no-match/error cache rows and acknowledgements;
4. four-attempt retry timing metadata for 429, 5xx, and transport failures
   using paused Tokio time (no sleeps);
5. cancellation before scheduling, during a provider lookup, during analysis,
   and while the writer has acknowledged/unacknowledged requests;
6. worker panic, status-task panic, writer panic/open/write failure, and
   incomplete-count exit behavior; and
7. final human summary ordering and `BatchOutcome` error categories.

Bound every scenario, barrier, channel receive, and join with a short timeout.
Mandatory tests use synthetic tracks, a temporary internal store, and injected
lookup/analysis futures.

### Step 2: Extract batched hydration selection

Move normalization and cache interpretation out of `run_hydrate`. Use existing
batch state functions or add focused batch queries in `adapters/state` only if
the selection otherwise remains an N+1 loop. The result must preserve exact
pending order, including longest-processing-time sorting of analysis jobs.

The CLI still resolves its `SearchParams` and prints the plan. It should no
longer know how enrichment/analysis cache rows determine `needs_*` flags.

### Step 3: Canonicalize the Discogs track outcome

Factor normalization, serialization, cache acknowledgement, auth failure, and
terminal result into the application function. Make existing MCP workers call
it with `DoNotCache`; make CLI workers call it with `CacheTerminalError` and
the existing retry future.

Add a two-surface regression matrix proving CLI and MCP retain their current
error-row difference while match/no-match behavior remains identical. Do not
change public failure text; typed internal failures are rendered at the
existing edges.

### Step 4: Move and own the combined cache writer

Move the transport-independent message enum and writer loop to application.
Wrap channel + join handle in a session with explicit `sender()` and `finish()`
behavior. Reuse the existing generic acknowledgement request rather than
adding another channel envelope.

Test dropped acknowledgements, selective trigger failure, store-open failure,
join panic, sender drop, caller cancellation, and all-success accounting. A
cancelled command must not leave a writer task or SQLite handle alive.

### Step 5: Make application reports the final truth

Derive terminal, incomplete, operation-failure, join-failure, and
writer-failure totals from typed reports. Live counters may update the spinner,
but add a debug/test invariant comparing them with the final report on a
non-cancelled run. Map the report to the existing CLI `BatchOutcome` without
changing its exit behavior or user-visible summaries.

### Step 6: Extract CLI interaction modules

After Steps 2-5 are green, move auth/retry and presentation into their named CLI
modules. `run_hydrate` should read as a bounded sequence of prepare, present,
confirm/auth, execute, present-result. It must not contain per-track cache SQL,
provider response serialization, cache-writer loops, or duplicated final
accounting.

Keep `mod.rs` as a navigation surface. Reject wrappers whose only purpose is
forwarding the original long parameter list.

### Step 7: Focused and full verification

Run:

```bash
cargo test -p reklawdbox hydrate_ -- --nocapture
cargo test -p reklawdbox enrichment_cache -- --nocapture
cargo test -p reklawdbox metadata_backfill -- --nocapture
cargo test -p reklawdbox --test source_boundaries
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
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
git diff --check
```

Inspect all task/channel ownership paths manually; green tests alone are not
sufficient for cancellation or writer cleanup.

Require independent architecture/API and async-resource reviews. One reviewer
must prove no CLI/MCP transport type moved inward and compare edge contracts;
the other must trace sender closure, acknowledgements, worker/status/writer
joins, cancellation, and partial-failure accounting. Remediate and re-review
all actionable findings.

## Machine-checkable done criteria

- [ ] CLI hydrate no longer implements per-track cache interpretation,
      provider serialization/persistence, or the cache-writer loop.
- [ ] One application Discogs track outcome serves CLI and MCP with one
      explicit two-value lookup-failure persistence policy.
- [ ] Application reports, not live atomics, determine final failure,
      incomplete, and exit accounting.
- [ ] Prompts, progress, retry/backoff, browser launch, CPU policy, help, and
      output remain at the CLI edge and are unchanged.
- [ ] Combined writer ownership closes senders, awaits/cleans the writer, and
      preserves typed open/write/ack/join failures under cancellation.
- [ ] The two `too_many_arguments` suppressions in
      `application/enrichment/hydrate.rs` are removed through semantic request
      or policy types, not a grab-bag context.
- [ ] No MCP schema, cache row, match-quality string, provider behavior, or
      analyzer version changes.
- [ ] Mandatory tests require no network, private DB, or private audio.
- [ ] Full architecture, workspace, release, MCP, docs-contract, site, and
      diff gates pass.

## STOP conditions

Stop and report if:

- sharing the per-track outcome would force CLI and MCP to change their
  existing retry, auth, or error-cache contracts;
- preserving live progress requires putting terminal UI in `application/`;
- cancellation cannot prove writer/provider/analysis task cleanup with bounded
  tests;
- a cache schema/version or serialized row change becomes necessary;
- a proposed abstraction has only one useful implementation and merely hides
  the original arguments; or
- a test would require a live provider, private library, or source audio
  mutation.

## Complexity accounting

Success removes duplicate cache/outcome/writer policy and shortens the CLI to
interaction plus coordination. Moving the existing 630-line function into
`command.rs`, or wrapping its arguments in one broad context struct, is not
success.

## Git workflow

- Branch: `codex/049-extract-hydrate-coordination`
- Preferred commit: `refactor(hydrate): extract application coordination`
- Do not push, merge, release, deploy, or call live providers.
