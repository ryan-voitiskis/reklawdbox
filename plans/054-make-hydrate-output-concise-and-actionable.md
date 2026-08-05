# Plan 054: Make hydrate output concise and actionable

> **Executor instructions:** Read this plan in full before changing code. Run
> the drift check first and preserve all unrelated worktree changes. Implement
> the terminal contract below as one coherent behavior change; do not solve the
> symptom by hiding failures, weakening the non-zero exit, or discarding the
> details needed for diagnosis.
>
> Use an isolated worktree from the reviewed integration base. Do not push,
> merge, release, deploy a host binary, change cache schemas, or run a private
> whole-library hydrate unless the operator separately authorizes it.

## Status

- **Priority:** P1
- **Effort:** M
- **Risk:** MED
- **Depends on:** none
- **Category:** CLI usability / failure presentation / batch diagnostics
- **Planned at:** `main` commit `d5603cb`, Reklawdbox v0.33.0, 2026-08-01
- **Implementation:** preserved on unmerged branch
  `codex/hydrate-concise-output` at `edca3c8`; not integrated into `main`

## Decision

Change `hydrate` from a stream of per-track error logs into a quiet progress
display followed by one authoritative completion report.

The default report will retain the existing stage counts, show a bounded set of
concise and actionable details, explain that a rerun is resumable, and still
exit non-zero when the current `BatchOutcome` says the run failed. A new
`--verbose-errors` flag will list every affected track after progress has been
cleared. It will not stream failures during the run.

Do not create an automatic log file. Hydrate diagnostics can contain private
artists, titles, and absolute music paths; silently persisting them creates a
new privacy and lifecycle problem. Structured details should live only for the
duration of the process unless the operator explicitly redirects terminal
output.

## Evidence from v0.33

A real overnight v0.33 run selected 2,782 pending operations across 3,108
tracks and completed in 390 minutes. It produced:

- 182 Discogs enrichments, 106 terminal no-matches, and 31 Discogs failures;
- 2,460 successful analyses and 3 analysis failures;
- 5,239 successful cache writes and no cache-write failures;
- no worker-join failures, no incomplete work, and no cancellation; and
- a correct non-zero result for the 34 per-track/provider failures.

The accounting was good. The presentation was not:

1. each Discogs failure emitted a timestamped `ERROR` while the progress bars
   were drawing;
2. analysis failures did the same from the application workflow;
3. terminal redraws interleaved with those messages and truncated the useful
   track/reason text;
4. the same run then printed aggregate error counts at the end; and
5. Rust's top-level `Result` termination added a redundant implementation
   dump such as `Error: BatchFailure { ... }` after the human summary.

The user saw dozens of lines but still could not reliably read the error
reasons. The plan treats that as a presentation and information-retention bug,
not as a reason to relax failure accounting.

## Goals

1. Keep the progress area stable during ordinary per-track failures.
2. Make the final block sufficient to answer: did it finish, what succeeded,
   what failed, is the cache safe, and what should I do next?
3. Preserve a sanitized reason and useful track identity for every failure
   until final rendering.
4. Keep default output bounded even if hundreds or thousands of tracks fail.
5. Make complete per-track diagnostics available explicitly with
   `--verbose-errors`.
6. Preserve cache behavior, retries, cancellation, task ownership, and exact
   process success/failure semantics.
7. Print one human-facing failure report, not a human summary followed by a
   Rust debug representation of the same failure.

## Non-goals

- Do not change which tracks are selected or considered cached.
- Do not change the four-attempt Discogs retry policy, backoff, timeouts,
  terminal `error` cache rows, or `--no-retry-errors` semantics.
- Do not turn Discogs no-match into an error.
- Do not change Stratum, Essentia, analyzer versions, cache identities, or
  audio output schemas.
- Do not change `BatchOutcome` failure categories or make partial failure exit
  successfully.
- Do not change MCP schemas, MCP enrichment results, or MCP logging behavior
  unless a shared internal type requires a compatibility-only adjustment.
- Do not add a global logging framework, telemetry service, event bus, JSON
  output mode, or persistent run-history database.
- Do not broadly redesign `analyze` or other CLI commands in this plan.
- Do not expose bounded remote diagnostic bodies in normal or verbose human
  output. Continue to use the sanitized `Display` surface of provider errors.

## Locked terminal contract

### Live progress

While work is running, the terminal owns two changing lines only:

```text
Hydrating [####################--------------------] 1391/2782  50%  ETA 3h
  Discogs: 182 enriched, 31 errors | Analysis: 1072 done, 1 error
```

Expected per-track provider and analysis failures update the counters but do
not print timestamped lines. Singular/plural wording is optional; count meaning
is not.

Retry diagnostics explicitly requested through `RUST_LOG` may remain noisy.
The normal default must not require `RUST_LOG` to be useful and must not emit a
per-track `ERROR` for failures that are already captured by the batch report.

Unexpected structural failures such as a writer panic, coordinator panic, or
failed cleanup may still be logged immediately. Those are invariant/lifecycle
failures, not normal failed items. Where they can be safely deferred, prefer
the final report; do not suppress a structural failure merely to protect the
progress display.

### Successful completion

The no-failure case stays short:

```text
Completed (390m 44s)
  Discogs:  182 enriched, 106 no match, 0 errors
  Analysis: 2463 done, 0 errors
  Cache writes: 5245 succeeded, 0 failed
```

There is no failure-details section and the process exits zero.

### Partial completion

The observed run should render approximately as follows. Exact colors and
indentation may follow the existing presentation module, but the information
order and boundedness are part of the contract.

```text
Completed with errors (390m 44s)
  Discogs:  182 enriched, 106 no match, 31 errors
  Analysis: 2460 done, 3 errors
  Cache writes: 5239 succeeded, 0 failed

Failure details:
  Discogs lookup (31): <sanitized reason or grouped reasons>
  Analysis:
    - <artist> - <title>: <component>: <concise reason>
    - <artist> - <title>: <component>: <concise reason>
    - <artist> - <title>: <component>: <concise reason>

Re-run hydrate to retry missing analysis and cached Discogs errors.
Use --verbose-errors to list every affected track.
```

The process still exits non-zero. The terminal must not append
`Error: BatchFailure { ... }` or another duplicate generic error line after
this report.

### Default detail budget

Default output is deliberately bounded:

- show at most five distinct Discogs failure groups;
- show the count for every displayed group;
- show at most five analysis tracks, including their failed component(s);
- show all task/writer/incomplete/cancellation categories in the existing
  summary because they are batch-level and normally few; and
- when anything is omitted, print one exact remainder line such as
  `... 26 more Discogs failures; use --verbose-errors for all details`.

Group Discogs failures by stable internal kind plus sanitized user-facing
reason. Do not parse arbitrary prose to guess HTTP categories. If structured
HTTP status is already available at the adapter boundary, preserving it in a
typed internal category is allowed; parsing a rendered string is not.

Analysis details are track-oriented rather than message-oriented. If both
Stratum and Essentia fail for one track, that is one failed track with two
component reasons. It must not become two failed tracks in the headline.

Default analysis identity should prefer `artist - title`, with a basename when
needed for disambiguation. Full absolute paths belong only in
`--verbose-errors`, since they are useful but visually dominant and private.

### Verbose failure details

Add this hydrate-only flag:

```text
--verbose-errors  Print every per-track failure after the final summary
```

With the flag set:

- retain the same progress display and final headline;
- print every Discogs and analysis failure only after progress is cleared;
- include track ID, artist, title, and the sanitized reason for Discogs;
- include track ID, artist, title, full file path, failed component, and
  sanitized reason for analysis;
- use stable ordering independent of task completion timing; and
- never include raw provider response bodies, secrets, broker session tokens,
  or retry diagnostic prose excluded from `LookupError::Display`.

Verbose mode lists final failures, not every retry attempt. Existing opt-in
`RUST_LOG` behavior remains the low-level retry/debug surface.

### Cancellation and infrastructure failures

The first line must distinguish terminal state:

- `Completed (...)` when every selected operation succeeded;
- `Completed with errors (...)` when work reached terminal accounting but any
  operation, join, or cache write failed; and
- `Cancelled (...)` when the user requested cancellation, followed by the
  completed/incomplete counts already owned by the report.

Do not say `Done` for a non-zero result. Do not say `failed` in a way that
implies the 5,239 successful cache writes were rolled back or lost.

## Architecture

### Preserve failure evidence in the application report

The application report remains canonical for final truth. Extend its internal
failure detail without moving terminal strings or colors into `application/`.

Discogs already returns `HydrationFailure` values containing identity,
provider, and `HydrationFailureKind`. `discogs_stage_report` currently counts
and then discards them. Preserve those failures in the stage/application
report so the CLI can aggregate and render them after all workers finish.

Analysis currently returns only two booleans:

```text
operation_failed
cache_write_failed
```

That is insufficient for final diagnostics. Add a structured analysis failure
record that preserves:

- track ID;
- artist and title;
- file path;
- component: job dispatch, Stratum, Essentia, or cache write; and
- sanitized error text.

Keep track-level success/failure accounting explicit. A track with multiple
component failures still increments `HydrationStageReport.failed` and
`operation_failures` according to the current semantics, while retaining each
component reason for presentation.

Only failure records need to survive aggregation. This does not worsen current
peak asymptotic memory: `run_bounded_workers` already retains every completed
outcome until stage aggregation. After aggregation, discard successful item
details and keep failures only, bounded by selected work.

Do not put `console::Style`, bullet formatting, truncation prose, or the
five-item display policy in application types.

### Make live counters observation-only

`ProviderCounters` should update atomics only. In particular,
`observe_discogs` must stop rendering/logging the first failure as a side
effect. The final application report, not the progress counter, owns failure
details.

Keep the existing debug/test invariant that compares terminal live counters
with application report totals on a normally joined run.

### Return analysis errors instead of logging and forgetting them

`run_analysis_stage` should convert expected per-track job, Stratum, Essentia,
and acknowledgement errors into the structured outcome. It should not emit a
default `tracing::error!` for the same expected failure that the CLI will later
render.

This is not error swallowing: the failure remains typed, counted, rendered,
and responsible for the non-zero exit. Unexpected panics and lifecycle errors
retain their separate task/join paths.

### Keep presentation at the CLI edge

Extend `src/cli/hydrate/presentation.rs` with pure rendering inputs and helpers
for:

- terminal-state headline;
- existing stage/cache counts;
- deterministic grouping;
- default caps and remainder counts;
- concise identity/path display;
- verbose full-detail rendering; and
- the retry/resume hint.

Prefer returning `Vec<String>` from pure helpers, as `final_lines` does today,
so formatting can be tested without a terminal. Clear `MultiProgress` before
printing any final detail lines.

Use Unicode-safe truncation for long reasons and display labels. Do not slice
UTF-8 strings by byte offsets. The default concise reason should have a fixed
human-scale maximum (roughly 200-300 displayed characters); verbose mode may
show the complete sanitized local error string.

### Render the process failure exactly once

`BatchOutcome` and `BatchFailure` continue to decide success and exit status.
The CLI must consume that result without allowing Rust's `main -> Result`
termination to print the derived struct after `print_final` has already
reported it.

Implement a typed top-level outcome that distinguishes:

1. success;
2. a non-zero CLI failure already presented to the user; and
3. an unpresented startup/configuration error that still needs one concise
   `Display` line.

Return `std::process::ExitCode` from the outer boundary or use an equivalent
non-destructive mechanism. Do not call `std::process::exit` from inside
hydrate: all workers, acknowledgements, SQLite handles, and tracing buffers
must finish/drop normally before the process returns.

If the cleanest implementation touches `src/main.rs`, characterize the current
CLI/MCP launch behavior first. Never print error prose to MCP stdout; MCP stdio
must remain protocol-clean. An MCP startup failure may be rendered once to
stderr with its `Display` text.

## Delivery sequence

### Phase 0 - Drift and ownership check

1. Confirm the execution base and inspect all current changes in scope.
2. Preserve the current untracked Plan 053 and unrelated audio-integrity work.
3. Re-read `src/README.md`, `AGENTS.md`, and the current hydrate tests.
4. Run the focused hydrate tests before editing to establish the baseline.
5. Confirm that no newer change has already introduced a general CLI exit or
   terminal-event abstraction. Reuse a small existing abstraction if present;
   do not create a second one.

### Phase 1 - Characterize the human contract

Before restructuring outcomes, add pure/synthetic tests that lock:

1. current stage, writer, incomplete, cancellation, and non-zero accounting;
2. the successful completion shape;
3. the 31 Discogs plus 3 analysis partial-failure shape;
4. default caps and exact omitted counts;
5. multiple analyzer component errors on one failed track;
6. stable ordering independent of completion order;
7. sanitized provider display that excludes remote diagnostic bodies; and
8. no duplicate top-level `BatchFailure` rendering.

Do not snapshot ANSI escape codes or timestamps. Test semantic lines before
color is applied, or strip style in the existing supported way.

### Phase 2 - Carry structured failures to completion

1. Preserve `EnrichmentTrackOutcome.failures` in `discogs_stage_report`.
2. Add typed analysis identity/component failure details.
3. Change `run_analysis_stage` to return those details rather than logging and
   dropping them.
4. Preserve all current operation, writer, join, incomplete, and cancellation
   totals.
5. Add invariants proving detail records and headline counts cannot drift.

Do not copy failures into `BatchFailure.error_summaries`; that field is for
bounded batch/task summaries, not an unbounded list of tracks.

### Phase 3 - Stop per-track terminal interruption

1. Remove per-track `tracing::error!` from
   `ProviderCounters::observe_discogs`.
2. Remove equivalent expected per-track analysis emissions now represented in
   the outcome.
3. Keep retry/backoff warnings under the existing explicit logging surface.
4. Keep rare structural/invariant failures visible and separately accounted.
5. Ensure progress counters continue to update immediately when an item fails.

### Phase 4 - Render one bounded final report

1. Clear progress before final rendering.
2. Choose the terminal headline from final accounting and cancellation state.
3. Print the existing stage and cache counts.
4. Render bounded, grouped default failure details.
5. Render all deterministic details when `--verbose-errors` is set.
6. Print the resumable retry hint only when failed or incomplete work exists.
7. Preserve readable output with one provider disabled and with no analysis
   runtime available.

### Phase 5 - Remove the duplicate process trailer

1. Introduce the narrow presented/unpresented CLI failure boundary.
2. Preserve process exit code 1 for the same hydrate conditions as v0.33.
3. Preserve process exit code 0 for success, all-cached, declined confirmation,
   and other currently successful hydrate exits.
4. Verify startup errors still print once with useful `Display` text.
5. Verify MCP stdio output remains uncontaminated.

### Phase 6 - Update the public CLI contract

Update the hydrate options table and behavior text in
`site/src/content/docs/cli/index.mdx`:

- document `--verbose-errors`;
- state that per-track failures are collected and summarized after progress;
- state that the default detail list is bounded;
- state that verbose details may include local file paths;
- retain the non-zero partial-failure contract; and
- explain that successful cache writes remain reusable and a rerun resumes
  failed/missing work.

Run the documentation contract checker and the semantic doc-drift workflow.
No MCP SOP or embedded help update is required unless implementation changes a
surface referenced there.

## Expected production files

- `src/application/enrichment/hydrate.rs` - structured provider/analysis
  failure evidence and report aggregation.
- `src/cli/hydrate/mod.rs` - `--verbose-errors` argument.
- `src/cli/hydrate/command.rs` - pass presentation policy, clear progress, and
  return the typed final outcome.
- `src/cli/hydrate/presentation.rs` - grouping, caps, detailed rendering, and
  quiet live-counter observation.
- `src/cli/hydrate/tests.rs` - contract, aggregation, rendering, and exit tests.
- `src/cli/command.rs`, `src/cli/mod.rs`, and/or `src/main.rs` - only the narrow
  render-once process boundary chosen in Phase 5.
- `site/src/content/docs/cli/index.mdx` - public flag/output contract.
- `scripts/check-doc-contract.test.mjs` only if the existing options contract
  needs a regression fixture for the new flag; do not weaken the checker.

No cache migration, `stratum-dsp/`, broker, Rekordbox adapter, MCP transport,
or audio schema file should change.

## Compatibility invariants

The executor must prove all of the following remain true:

1. `master.db` remains SQLCipher read-only.
2. Every selected worker reaches the same terminal/incomplete accounting as
   before.
3. Discogs matched, no-match, and terminal-error cache rows are unchanged.
4. Default hydrate reruns retry cached Discogs errors; `--no-retry-errors`
   still skips them.
5. Analysis cache freshness and missing-backend selection are unchanged.
6. Cache writes remain acknowledged before success is counted.
7. Three consecutive cache-write failures retain the current cancellation and
   drain policy.
8. Ctrl+C retains current scheduling-stop and started-work-drain behavior.
9. Worker, status, and writer join failures remain distinguishable.
10. Partial failure still exits non-zero after printing completed work.
11. The progress counters are observational; final truth comes from application
    reports.
12. MCP schemas, output text, auth remediation, and cache-persistence policy do
    not change.

## Focused tests

At minimum, add or update tests for:

1. `hydrate --help` includes `--verbose-errors`, defaults it to false, and the
   documentation options contract matches.
2. Zero failures produce no details or retry hint.
3. A mixed 31/3 result produces `Completed with errors`, correct counts, at
   most five groups/tracks per section, and exact remainder text.
4. `--verbose-errors` renders every failure after the summary in stable order.
5. Two component failures for one analysis track render twice under one track
   but count as one failed track.
6. Two identical Discogs reasons group into one count; different kinds do not.
7. Long Unicode artist/title/reason values truncate safely in default mode.
8. A bounded remote HTTP diagnostic body never appears in either default or
   verbose human output; the sanitized status/reason does.
9. Full paths are absent from default detail and present in verbose detail.
10. No expected per-track failure path calls the default error logger as its
    only means of retaining the reason.
11. Cache-write, worker-join, incomplete, and cancellation summaries remain
    visible and retain their exact `BatchOutcome` categories.
12. The presented-failure process path returns non-zero without emitting a
    second debug trailer.
13. An unpresented startup error prints once and returns non-zero.
14. MCP launch mode writes no human error text to stdout.

Use synthetic tracks, injected futures, temporary Reklawdbox-owned stores, and
captured/in-memory terminal writers. Mandatory tests must not require private
Rekordbox or audio data. Bound every async scenario, signal, channel receive,
and task join with a short timeout.

## Drift check

Run before implementation:

```bash
git diff --stat d5603cb..HEAD -- \
  src/main.rs \
  src/cli/command.rs \
  src/cli/hydrate \
  src/application/batch.rs \
  src/application/cache_writer.rs \
  src/application/enrichment/hydrate.rs \
  src/application/analysis \
  src/mcp/enrichment \
  site/src/content/docs/cli/index.mdx \
  scripts/check-doc-contract.mjs \
  scripts/check-doc-contract.test.mjs
```

Re-read every changed in-scope file. Updating names around a compatible
refactor is expected. Stop and reconcile the plan if selection, cache identity,
failure categories, writer acknowledgement, cancellation, retry policy, or
MCP stdio ownership has changed.

## Verification

### Focused gate

```bash
cargo fmt --check
cargo test -p reklawdbox hydrate_ -- --nocapture
cargo test -p reklawdbox batch -- --nocapture
cargo test -p reklawdbox cache_writer -- --nocapture
node --test scripts/check-doc-contract.test.mjs
dprint check
git diff --check
```

If test filters drift, discover the current exact names and record the
replacement commands rather than silently skipping an area.

### Public-contract gate

```bash
node scripts/check-doc-contract.mjs
cd site && npm ci && npm run build
```

Run the semantic review in `docs/workflows/doc-drift/README.md` because the CLI
flag and user-visible failure behavior are public contract changes.

### Standard workspace gate

Return to the repository root and run:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
```

Also run the current-checkout MCP smoke because Phase 5 may touch the outer
process boundary:

```bash
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
```

### Optional local terminal smoke

Only with operator approval and available private data, run a small bounded
hydrate cohort that is known to include one synthetic/injected or safely
reproducible failure. Capture combined output and verify:

1. progress remains readable;
2. no per-track default `ERROR` interrupts it;
3. the final counts match cache/accounting evidence;
4. default detail is bounded;
5. verbose detail identifies every failure; and
6. both modes return the same non-zero status.

A private smoke is supporting evidence, never a mandatory test or permission
to rehydrate the whole collection.

## STOP conditions

Stop and ask for review if any of the following becomes necessary:

- changing cache keys, cache schemas, analyzer versions, or match-quality
  values;
- changing retry count/backoff, selection, cancellation, or writer drain
  behavior;
- weakening non-zero partial-failure exit semantics;
- parsing arbitrary provider prose to manufacture failure categories;
- exposing raw remote diagnostic bodies or credentials;
- persisting private track/path diagnostics automatically;
- changing MCP schemas or stdout protocol behavior;
- introducing `process::exit` before normal async/resource cleanup;
- broad global tracing changes affecting unrelated commands; or
- modifying the active Plan 053 or unrelated audio-integrity files.

## Done criteria

Plan 054 is complete only when:

1. normal hydrate progress is not interrupted by expected per-track error
   lines;
2. every failed item still has structured, sanitized evidence at final
   rendering time;
3. the default final report is bounded and actionable;
4. `--verbose-errors` prints all failures after progress in stable order;
5. successful cache work is clearly distinguished from failed operations;
6. partial failure retains the exact current non-zero accounting;
7. the redundant `BatchFailure { ... }` trailer is gone;
8. startup and structural errors are still visible once;
9. CLI docs and doc-contract checks describe the new behavior accurately;
10. focused, full workspace, release-build, and MCP smoke gates pass; and
11. the diff contains no unrelated source, DSP, cache, provider, or MCP surface
    change.
