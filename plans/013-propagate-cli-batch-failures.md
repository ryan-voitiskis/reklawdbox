# Plan 013: Propagate CLI batch failures to summaries and exit status

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat e6eb382..HEAD -- src/cli/mod.rs src/cli/analyze.rs src/cli/hydrate.rs site/src/content/docs/cli/index.mdx
> ```
>
> If any file changed, compare the writer return types, discarded joins, and
> final `Ok(())` excerpts below with live code. Existing acknowledged CLI cache
> writes or terminal batch outcome handling is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

The `analyze` and `hydrate` CLI commands log per-track, worker, and cache-writer
failures but end with `Ok(())`. Worker `JoinError`s and writer results are
discarded, and counters advance when a cache message is merely queued rather
than persisted. Shell scripts, unattended runs, and release workflows can
therefore receive exit status zero after incomplete work. Every essential task
and write needs an observable terminal result, the printed summary must reflect
durable outcomes, and any failed/incomplete/cancelled batch must return `Err`.

## Current state

- `src/cli/mod.rs` owns shared CLI cache messages/helpers and signal handling.
- `src/cli/analyze.rs` runs concurrent Stratum/Essentia analysis with one
  blocking cache writer.
- `src/cli/hydrate.rs` runs Discogs, Beatport, and analysis provider trees with
  one blocking writer for enrichment and analysis cache rows.
- `src/main.rs` already propagates `cli::main().await` errors from the Tokio
  main function, so returning `Err` is sufficient for a non-zero process exit.
- All writes in scope are to the internal cache. Rekordbox remains read-only.

Current `src/cli/analyze.rs:206-243` makes the writer return `()` even on open
or repeated-write failure:

```rust
let writer_handle = tokio::task::spawn_blocking(move || {
    let conn = match store::open(&writer_store_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Cache writer: failed to open store: {e} — aborting pipeline");
            writer_cancel.cancel();
            return;
        }
    };
    // ...
    if consecutive_failures >= MAX_CONSECUTIVE_CACHE_WRITE_FAILURES {
        writer_cancel.cancel();
        return;
    }
});
```

Current `src/cli/analyze.rs:362-391` discards joins and returns success after
printing failures:

```rust
for handle in handles {
    let _ = handle.await;
}
// ...
let _ = writer_handle.await;
// prints "Done: ... failed"
Ok(())
```

Current `src/cli/hydrate.rs:438-504` has the same unit-returning writer for both
cache payload kinds. Inner worker joins are discarded at
`src/cli/hydrate.rs:685-686`, `813-814`, and `888-889`.

Current `src/cli/hydrate.rs:894-900` also discards all three outer provider
results, the status task, and writer result:

```rust
drop(cache_tx);
let _ = tokio::join!(discogs_task, beatport_task, analysis_task);
cancel.cancel();
let _ = status_task.await;
let _ = writer_handle.await;
```

Despite printing non-zero provider error counters, current
`src/cli/hydrate.rs:955` always returns `Ok(())`.

Current `src/cli/mod.rs:315-338` defines unacknowledged payloads and a generic
send helper that proves only queue acceptance:

```rust
pub(crate) struct CliCacheWriteMsg { /* file identity + JSON */ }

pub(crate) async fn send_cache_message<T>(
    tx: &tokio::sync::mpsc::Sender<T>,
    message: T,
    context: &str,
) -> Result<(), String> {
    tx.send(message).await
        .map_err(|e| format!("{context} cache queue send failed: {e}"))
}
```

Applicable conventions:

- Commands return `Result<(), Box<dyn Error>>`; preserve this public shape.
- Continue processing independent tracks after ordinary per-track failure, but
  return a non-zero command result after the summary.
- Keep the single-writer design and bounded channels.
- `MAX_CONSECUTIVE_CACHE_WRITE_FAILURES` remains the threshold for cancelling
  new work, not a license to report earlier failed writes as successful.
- User Ctrl-C is graceful in cleanup but still an incomplete command and must
  exit non-zero.

## Commands you will need

- Shared CLI tests: `cargo test -p reklawdbox cli::tests` — all shared CLI tests pass.
- Analyze tests: `cargo test -p reklawdbox cli::analyze` — all matching tests pass.
- Hydrate tests: `cargo test -p reklawdbox cli::hydrate` — all matching tests pass.
- Format: `cargo fmt --check` — no diff.
- Docs/config format: `dprint check` — exits 0.
- Lint: `cargo clippy -p reklawdbox --all-targets -- -D warnings` — no warnings.
- Full crate tests: `cargo test -p reklawdbox --no-fail-fast` — all tests pass.
- Docs build: `(cd site && npm ci && npm run build)` — locked install/build passes.
- Release build: `cargo build --release` — exits 0.
- Version smoke: `./target/release/reklawdbox --version` — version prints.
- Help smoke: `./target/release/reklawdbox --help` — help prints.

## Scope

**In scope** (the only source/docs files you may modify):

- `src/cli/mod.rs`
- `src/cli/analyze.rs`
- `src/cli/hydrate.rs`
- `site/src/content/docs/cli/index.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Provider lookup/matching algorithms, retry intervals, and auth behavior.
- MCP `enrich_tracks` persistence (Plan 012 handles that separate surface).
- Changing CLI flags, output format beyond truthful additive cache-write/error
  totals, or interactive confirmation behavior.
- Store schema and cache freshness definitions.
- Treating a legitimate provider no-match as an error.
- Direct Rekordbox writes or user-visible metadata changes.

## Git workflow

- Branch: `codex/013-propagate-cli-batch-failures`
- Use Conventional Commits; preferred final message:
  `fix(cli): propagate batch failures`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Introduce testable terminal-result types

In `src/cli/mod.rs`, define private/shared types for:

- `CacheWriteRequest<T>`: payload plus
  `tokio::sync::oneshot::Sender<Result<(), String>>`;
- `CacheWriterReport`: attempted, succeeded, failed, whether the failure
  threshold caused cancellation, and stable error summaries;
- `CliBatchFailure`: command name plus track/provider failures, worker join
  failures, writer failures, and cancellation/incomplete counts. Implement
  `Display` and `Error` without dumping feature JSON or credentials.

Change `send_cache_message` to send a `CacheWriteRequest<T>` and await its
acknowledgement. It returns success only after SQLite persistence, not queue
acceptance. Add tests for acknowledged success, acknowledged write error,
closed queue, and canceled acknowledgement.

Add a pure final-outcome helper that returns `Ok(())` only when failure count,
join failure count, writer failure count, and incomplete count are all zero and
no user cancellation occurred. Test every input independently and in
combination.

All async ack, cancellation, writer-drain, and worker-join tests must have a
five-second outer `tokio::time::timeout` plus bounded channel/oneshot waits and
joins. Track sender and task ownership in a cleanup guard: drop every retained
MPSC sender before waiting for drain, cancel fixtures explicitly, and
abort/await unfinished tasks on early assertion failure. The timeout is a test
watchdog, never a production shortcut that converts incomplete work to success.

**Verify**: `cargo test -p reklawdbox cli::tests` → all shared ack and terminal
outcome tests pass.

### Step 2: Make signal cancellation distinguishable from normal shutdown

Extend `spawn_signal_handlers` with an `Arc<AtomicBool>` (or a small shared
cancellation-state type) set only by Ctrl-C. Writer-triggered cancellation must
not set the user flag. Before normal end-of-batch code calls `cancel.cancel()`
to stop the status/SIGWINCH tasks, capture whether cancellation was already
requested and its source.

Expected terminal meanings:

- operator declines the pre-run prompt: intentional `Ok(())`, as today;
- no matching/all cached: `Ok(())`;
- Ctrl-C after work starts: clean up/drain, print `Cancelled`, return `Err`;
- writer threshold/internal cancellation: clean up/drain, return `Err` naming
  the internal failure;
- normal post-work status-task shutdown: not cancellation failure.

**Verify**: unit-test the state transitions without sending a real OS signal;
`cargo test -p reklawdbox cli_cancellation_state` → all pass.

### Step 3: Make the analyze writer acknowledge and report every write

Change the analyze channel to `CacheWriteRequest<CliCacheWriteMsg>`. Extract a
blocking writer function that:

1. returns a `CacheWriterReport`;
2. acknowledges every attempted `set_audio_analysis` result;
3. records each failure, not just the third consecutive failure;
4. after the consecutive-failure threshold, cancels new analysis but continues
   draining queued messages and acknowledges them with a stable fatal-writer
   error so no producer hangs;
5. on store-open failure, drains all messages with an error acknowledgement;
6. never panics if an acknowledgement receiver was dropped.

Because `cli_analyze_single_track` already awaits `send_cache_message`, its
success counters will now mean durable cache writes. Preserve Stratum and
Essentia outcome distinctions.

Add writer tests using a temporary store and a SQLite trigger that selectively
fails `audio_analysis_cache` inserts. Cover success, one recoverable failure,
threshold cancellation/drain, open failure, and dropped ack receiver.

**Verify**: `cargo test -p reklawdbox cli::analyze -- --nocapture` → all writer
and existing analysis-result tests pass.

### Step 4: Stop discarding analyze worker/writer terminal results

For every spawned track handle, convert `JoinError` into one failed/incomplete
track and retain a stable error summary. Do not merely increment the progress
bar or log it. Await the writer and separately handle:

- normal report with zero failures;
- normal report containing failures;
- writer `JoinError`/panic.

Compute incomplete work as selected pending tracks minus terminal track
outcomes; this catches cancellation before spawn and early writer shutdown.
Print the summary first, then pass all counts to the final-outcome helper. If
any analysis, queue/write, join, cancellation, or incomplete failure occurred,
return `Err(CliBatchFailure)` so the process is non-zero.

**Verify**: add unit tests around an extracted analyze finalizer showing zero
failures returns `Ok`, while a per-track error, worker panic, writer failure,
and cancellation each return `Err`. Run
`cargo test -p reklawdbox analyze_batch_outcome` → all pass.

### Step 5: Apply the same acknowledged writer contract to hydrate

Change the hydrate channel to `CacheWriteRequest<HydrateCacheMsg>` and make its
writer follow Step 3 for both `Enrichment` and `AudioAnalysis`. Provider
`enriched` and `no_match` counters must advance only after an acknowledged
write. A lookup error may still attempt to persist `match_quality="error"`, but
the provider error counter remains non-zero regardless of whether that
diagnostic cache row writes successfully.

Return one `CacheWriterReport` covering both payload variants and label errors
without embedding response JSON or session credentials.

**Verify**: `cargo test -p reklawdbox cli::hydrate` → synthetic enrichment and
analysis writes prove counters advance only after persistence; selective
failure and threshold draining are covered.

### Step 6: Propagate every hydrate worker tree result

Refactor the Discogs, Beatport, and analysis outer tasks to return a small
provider-task report. Inside each, handle every inner `JoinHandle.await`:

- `Ok(())` records one terminal worker;
- `JoinError` increments that provider's error and incomplete counts and stores
  a stable message.

Await the three outer tasks individually (a `join!` is fine only if every tuple
member is matched and recorded). Treat an outer `JoinError` as failure for all
work not already terminal in that provider. Await the status task and record an
unexpected panic; normal cancellation-driven exit is success. Await and merge
the writer report as in analyze.

After printing all provider summaries, return `Err(CliBatchFailure)` when any
new provider error, join failure, writer failure, user cancellation, or
incomplete work occurred. Previously cached `match_quality="error"` rows that
the operator explicitly skipped with `--no-retry-errors` are startup context,
not new failures for this invocation; do not make that no-work case fail.

**Verify**: `cargo test -p reklawdbox hydrate_batch_outcome` → zero-work and
all-success cases return `Ok`; each provider error, inner/outer panic, writer
failure, cancellation, and incomplete-count case returns `Err`.

### Step 7: Document truthful CLI exit semantics

Update the `hydrate` and `analyze` sections in
`site/src/content/docs/cli/index.mdx`: commands still print/drain their final
summary, but any new provider/analysis/write/join failure, user Ctrl-C, or
incomplete selected work exits nonzero. All-cached/no-work and declined
confirmation remain successful. Do not add or claim new flags.

Run `docs/workflows/doc-drift/README.md`; if it identifies another CLI claim
outside scope, STOP and report before expanding the diff.

**Verify**:

```bash
dprint check
(cd site && npm ci && npm run build)
rg -n 'nonzero|non-zero|Ctrl.C|incomplete' site/src/content/docs/cli/index.mdx
```

Expected: docs build and both command sections describe failure/cancellation
exit status.

### Step 8: Remove silent join discards and run the full gate

Inspect only the in-scope production files. No essential worker/writer await may
remain assigned to `_`; explicitly match every terminal result. It is fine to
ignore display-only method return values such as `mp.clear().ok()`.

**Verify**:

```bash
rg -n 'let _ = (handle|writer_handle|status_task)\.await|let _ = tokio::join!' src/cli/analyze.rs src/cli/hydrate.rs
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
```

Expected: `rg` returns no matches; every remaining command exits 0; help
prints; only in-scope files changed.

## Test plan

- `src/cli/mod.rs`:
  - cache request acknowledgement success/write error/queue close/ack cancel;
  - terminal outcome truth table;
  - user versus internal versus normal-shutdown cancellation state.
- `src/cli/analyze.rs`:
  - writer success, selective error, threshold cancellation+drain, open error,
    dropped ack;
  - worker `JoinError` counted;
  - durable write required before analyzed success;
  - finalizer returns error for every failure/incomplete path.
- `src/cli/hydrate.rs`:
  - both payload variants acknowledged;
  - provider counters require persistence;
  - inner and outer task panics propagate;
  - no-match remains success after persistence;
  - network/provider error, writer error, cancellation, and incomplete work
    yield final `Err`;
  - all-cached/no-work and declined confirmation remain `Ok`.
- Tests use temporary SQLite stores, synthetic messages, injected task aborts,
  and local triggers only—no provider network, private audio, or real Rekordbox.
- Docs: clean install/build and doc-drift verify the nonzero failure/Ctrl-C
  contract for both commands.

## Done criteria

- [ ] Queue acceptance alone never increments a CLI success/no-match counter.
- [ ] Both CLI writers acknowledge every received message and return a report.
- [ ] Store-open and threshold failures drain/reject queued work without
      hanging producers.
- [ ] Async CLI tests bound every wait/join and deterministically drop senders,
      cancel fixtures, and clean up worker/writer tasks.
- [ ] Every essential inner worker, outer worker, status task, and writer join
      is explicitly handled.
- [ ] `analyze` returns `Err` after any track, join, write, cancellation, or
      incomplete failure.
- [ ] `hydrate` returns `Err` after any new provider, join, write,
      cancellation, or incomplete failure.
- [ ] Ctrl-C cleans up but exits non-zero; normal end-of-batch cancellation does
      not create a false failure.
- [ ] CLI docs state the new nonzero failure, incomplete-work, and user-Ctrl-C
      semantics without changing flags.
- [ ] No-match, all-cached/no-work, and declined confirmation retain intended
      success semantics.
- [ ] The silent-await `rg` command returns no matches.
- [ ] Format, dprint, clippy, full tests, release build, and CLI help exit 0.
- [ ] `git diff --name-only` lists only the three in-scope CLI files, the CLI
      docs page, and optionally `plans/README.md`.
- [ ] Rekordbox `master.db` remains read-only.

## STOP conditions

Stop and report back if:

- A concurrent plan has changed `CliCacheWriteMsg`, `HydrateCacheMsg`, or
  `send_cache_message`; reconcile one acknowledgement envelope rather than
  nesting/duplicating protocols.
- A provider can enqueue multiple required writes for one counter increment;
  define an explicit multi-write transaction/result before proceeding.
- Returning `Err` from `cli::main` does not produce a non-zero binary exit in a
  local test; fix the top-level propagation rather than calling
  `std::process::exit` inside workers.
- Draining a failed writer can wait forever due to a retained sender; fix
  ownership instead of adding a lossy timeout.
- Tests would require live providers, real Rekordbox, or private audio.
- The solution changes CLI flags or store schema.
- The doc-drift workflow finds another hydrate/analyze exit-status claim
  outside scope; report it before broadening the diff.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- New CLI batch worker trees must return reports; never detach an essential
  task or write `let _ = handle.await`.
- New cache payload variants must use the acknowledged envelope and define the
  counter that advances only after `Ok(())`.
- Reviewers should compare selected, terminal, and incomplete counts for each
  provider and ensure a failure is neither lost nor double-counted.
- Keep cleanup graceful, but do not conflate graceful cleanup with successful
  completion. Automation depends on the process exit status.
