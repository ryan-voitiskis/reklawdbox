# Plan 012: Acknowledge enrichment cache writes before reporting success

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat e6eb382..HEAD -- src/tools/enrich_handlers.rs src/tools/tests.rs site/src/content/docs/mcp-tools/enrichment-analysis.mdx
> ```
>
> If any file changed, compare the channel message, provider result tuples,
> writer loop, and final summary below with live code. Existing per-write
> acknowledgements or a changed summary contract are STOP conditions.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

`enrich_tracks` counts a match as enriched—or a no-match as skipped—as soon as
it queues a cache message. Channel closure and SQLite write errors are only
logged, so a response can claim success and zero failures even though no
durable cache entry exists and the next batch repeats the work. Each provider
attempt must await a result from the sole cache writer, and only acknowledged
persistence may increment success/no-match counters.

## Current state

- `src/tools/enrich_handlers.rs` implements individual lookups and concurrent
  batch enrichment. Only the batch writer/producer path is in scope.
- `src/tools/tests.rs` has deterministic provider lookup overrides plus
  temporary Rekordbox/internal-store fixtures; use these rather than network.
- Cache writes go to the internal SQLite store and are permitted. Rekordbox
  `master.db` stays read-only.

Current `src/tools/enrich_handlers.rs:366-375` has a fire-and-forget message:

```rust
enum EnrichCacheWriteMsg {
    Enrichment {
        provider: String,
        norm_artist: String,
        norm_title: String,
        norm_album: Option<String>,
        match_quality: Option<String>,
        response_json: Option<String>,
    },
}
```

Current `src/tools/enrich_handlers.rs:438-467` logs queue failures but still
returns a successful provider outcome:

```rust
if let Err(e) = cache_tx.send(EnrichCacheWriteMsg::Enrichment { /* ... */ }).await {
    tracing::warn!(provider, "cache channel send failed: {e}");
}
(1, 0, Vec::new())

// No-match path does the same, then:
(0, 1, Vec::new())
```

The Discogs branch duplicates the same behavior at
`src/tools/enrich_handlers.rs:638-667`.

Current `src/tools/enrich_handlers.rs:817-850` discards writer-open and write
failures after logging:

```rust
let writer_handle = tokio::task::spawn_blocking(move || {
    let conn = match store::open(&writer_store_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("Enrich cache writer: failed to open store: {e}");
            return;
        }
    };
    while let Some(msg) = cache_rx.blocking_recv() {
        if let Err(e) = store::set_enrichment(/* ... */) {
            tracing::error!("Enrich cache writer: failed to write ...: {e}");
        }
    }
});
```

Current `src/tools/enrich_handlers.rs:912-948` aggregates lookup results before
the writer join and only sees writer task panics:

```rust
progress.processed += track_result.processed;
progress.cached += track_result.cached;
progress.skipped += track_result.skipped;
progress.failures.extend(track_result.failures);
// ...
if let Err(e) = writer_handle.await {
    progress.failures.push(json!({ "provider": "cache_writer", /* ... */ }));
}
```

Applicable conventions:

- Keep exactly one blocking SQLite writer connection; do not perform SQLite
  writes from async provider tasks.
- Failures are returned as structured JSON with track/provider context.
- Cached hits do not enqueue a write and remain counted as `cached`.
- A no-match is useful only after its negative cache row is durable; its
  existing `skipped` counter therefore also requires acknowledgement.

## Commands you will need

- Enrichment tests: `cargo test -p reklawdbox enrich_tracks` — all matching tests pass.
- Writer tests: `cargo test -p reklawdbox enrich_cache_writer` — all matching tests pass.
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

- `src/tools/enrich_handlers.rs`
- `src/tools/tests.rs`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Provider matching algorithms, rate limits, retries, or existing top-level response
  fields; the additive `summary.cache_writes` object is the only response change.
- Discogs device-auth serialization (handled by Plan 015).
- Individual `lookup_discogs`/`lookup_beatport`/`lookup_bandcamp` semantics;
  they already write synchronously and surface store errors.
- Store schema or `set_enrichment` SQL changes.
- Increasing enrichment concurrency or adding multiple SQLite writers.
- Direct Rekordbox writes or metadata staging.

## Git workflow

- Branch: `codex/012-acknowledge-enrichment-cache-writes`
- Use Conventional Commits; preferred final message:
  `fix(enrich): acknowledge cache persistence`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Add deterministic failing-writer regressions

In `src/tools/tests.rs`, use existing `set_test_beatport_lookup_override` (and
Discogs override where useful) so no network is contacted. Add tests for:

1. A successful no-match lookup with a normal store is counted as `skipped`
   only after `store::get_enrichment` confirms the negative cache row exists.
2. Install a SQLite `BEFORE INSERT` trigger on `enrichment_cache` that raises
   `FAIL` for the test row. The same no-match must report `skipped == 0`,
   `failed == 1`, and a structured provider/track/cache-write failure; no cache
   row exists.
3. Configure the server's initialized internal connection normally but point
   its separate writer `store_path` at a directory. Writer-open failure must be
   reported for the affected provider attempt rather than as success.
4. With two tracks and a selective trigger, one write succeeds and one fails;
   summary counts and persisted rows match exactly.

Do not assert exact rusqlite wording. Assert stable context (`track_id`,
provider, and a `cache write`/`cache writer` prefix).

**Verify**: `cargo test -p reklawdbox enrich_cache_writer -- --nocapture` → the
new failure tests fail against current fire-and-forget behavior; existing
cached-hit tests pass.

### Step 2: Add a per-message acknowledgement channel

Extend each `EnrichCacheWriteMsg::Enrichment` with a
`tokio::sync::oneshot::Sender<Result<(), String>>`. Create one shared async
helper that:

1. creates the oneshot pair;
2. sends the full write message to the bounded MPSC queue;
3. awaits the acknowledgement;
4. maps MPSC closure, oneshot cancellation, and SQLite `Err` into stable
   cache-write error strings.

The helper should accept/return only cache-write data and `Result`; provider
track context belongs in the caller's structured failure object. Do not hold a
SQLite connection or blocking operation in async code.

**Verify**: add helper-level tests for successful ack, explicit writer `Err`,
closed MPSC receiver, and dropped oneshot sender; then run
`cargo test -p reklawdbox enrich_cache_ack` → all pass.

Every acknowledgement/writer test must have a five-second outer
`tokio::time::timeout`, with separate bounded waits for each channel receive,
oneshot acknowledgement, and writer join. Use a scope/RAII cleanup guard that
drops every retained MPSC sender before awaiting the writer and aborts/awaits
the writer task on early assertion failure. These are test watchdogs only;
production code must still drain to a truthful terminal result rather than
timing out and discarding messages.

### Step 3: Make the blocking writer acknowledge every received message

Extract the writer loop into a private, testable function returning a small
report (`attempted`, `succeeded`, `failed`, and optionally dropped ack
receivers). For each message:

- call `store::set_enrichment` once;
- convert its result to `Result<(), String>` with no secret data;
- send that result through the message's oneshot sender;
- continue draining after an individual SQLite error so other tracks can
  succeed.

If `store::open` fails, do not simply return and strand/close producers. Drain
the receiver until all senders close and acknowledge every message with the
same stable writer-open error. This prevents bounded-channel deadlock and gives
each provider attempt a terminal result. The writer report is diagnostic; the
per-message acknowledgements are the source of per-attempt truth.

If an acknowledgement receiver was dropped because a producer task panicked,
record it in the writer report but continue. Never panic on `ack.send` failure.

**Verify**: `cargo test -p reklawdbox enrich_cache_writer` → open failure,
selective SQLite failure, continued draining, and dropped receiver tests pass.

### Step 4: Count provider outcomes only after durable acknowledgement

Replace every batch producer send in both paths:

- generic Beatport/Bandcamp `provider_enrich_fut`;
- the distinct Discogs match and no-match branches.

For a lookup match:

- acknowledged `Ok(())` → `(processed=1, skipped=0)`;
- any queue/ack/write error → `(0, 0)` plus one structured failure.

For a lookup no-match:

- acknowledged `Ok(())` → `(0, skipped=1)`;
- any queue/ack/write error → `(0, 0)` plus one structured failure.

Every write failure object must include `track_id`, `artist`, `title`,
`provider`, and a stable error prefix. Keep lookup/serialization/auth failures
unchanged. Avoid cloning response JSON solely for acknowledgement.

**Verify**: `cargo test -p reklawdbox enrich_tracks` → all existing summary
tests pass plus match/no-match write-failure regressions.

### Step 5: Reconcile the writer report with the final response

Await the writer handle after all track tasks and MPSC senders finish. A
`JoinError` remains an infrastructure failure and must make the response's
failure list non-empty. For a normal writer return, add an additive
`summary.cache_writes` object with `attempted`, `succeeded`, and `failed`.

Assert in code (debug assertion is sufficient) that writer attempted equals
succeeded + failed. The public counters mean:

- `enriched`: matched lookups durably written;
- `skipped`: no-match lookups durably written;
- `cached`: pre-existing cache hits;
- `failed`: provider attempts that did not reach their required terminal state.

Do not double-count one SQLite failure as both a provider failure and a second
generic writer failure. A writer task panic is the exception because it is an
independent infrastructure failure; identify it with `provider=cache_writer`.

**Verify**: targeted tests assert that the successful+failed write totals equal
attempted and that persisted row counts agree with the summary.

### Step 6: Document the durable summary contract

Update `site/src/content/docs/mcp-tools/enrichment-analysis.mdx` for
`enrich_tracks`: document additive `summary.cache_writes` fields and state that
`enriched`/`skipped` advance only after durable cache acknowledgement. A failed
write appears in `failed` and never as a successful/no-match terminal result.
Run `docs/workflows/doc-drift/README.md`; if it finds another public response
claim outside scope, STOP and report before editing it.

**Verify**:

```bash
dprint check
(cd site && npm ci && npm run build)
rg -n 'cache_writes|attempted|succeeded|failed' site/src/content/docs/mcp-tools/enrichment-analysis.mdx
```

Expected: docs build and the public summary contract is described.

### Step 7: Run the full crate gate

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
```

Expected: all commands exit 0; only in-scope files changed.

## Test plan

- Ack helper unit tests: success, explicit write error, closed queue, canceled
  acknowledgement.
- Writer unit/integration tests: successful write, selective trigger failure,
  continued draining, store-open failure drains and rejects all messages, and
  dropped ack receiver does not panic.
- MCP handler tests using lookup overrides:
  - persisted match increments `enriched`;
  - persisted no-match increments `skipped`;
  - failed match/no-match write increments only `failed`;
  - mixed success/failure has exact per-attempt totals and database rows;
  - pre-existing cache hits enqueue no writes.
- No test may contact a live provider or require a real Rekordbox database.
- Docs: doc-drift plus a clean-site install/build covers the additive summary.

## Done criteria

- [ ] Every batch enrichment cache message carries a one-shot result channel.
- [ ] Every producer awaits acknowledgement before incrementing `processed` or
      `skipped`.
- [ ] Writer-open failure is delivered to every queued/subsequent attempt
      without bounded-channel deadlock.
- [ ] An individual SQLite write error does not stop later writes.
- [ ] The final response exposes attempted/succeeded/failed cache-write totals.
- [ ] MCP docs define those totals and durable counter semantics.
- [ ] Persisted rows exactly match acknowledged success/no-match counts.
- [ ] No failure is double-counted as provider plus generic writer failure.
- [ ] Cached-hit semantics remain unchanged and enqueue no write.
- [ ] Ack/writer tests bound every wait/join, drop all senders before drain,
      and deterministically clean up spawned writer tasks.
- [ ] `cargo fmt --check`, `dprint check`, clippy, targeted tests, and full
      crate tests exit 0.
- [ ] `git diff --name-only` lists only `src/tools/enrich_handlers.rs`,
      `src/tools/tests.rs`, the enrichment-analysis docs page, and optionally
      `plans/README.md`.
- [ ] Rekordbox `master.db` remains read-only.

## STOP conditions

Stop and report back if:

- The batch writer already has an acknowledgement/result protocol not shown in
  Current state.
- A provider attempt can legitimately enqueue more than one cache write; the
  one-attempt/one-ack accounting must be redesigned explicitly before coding.
- Draining on writer-open failure can wait forever because an unrelated sender
  is retained after all tasks finish; locate/fix sender ownership rather than
  adding a timeout that drops results.
- Tests would need live network, private audio, or a real Rekordbox database.
- The solution requires multiple concurrent SQLite writers or a store-schema
  change.
- The doc-drift workflow finds another public `enrich_tracks` response claim
  outside scope; report it before broadening the diff.
- Any proposed path writes user-visible metadata or Rekordbox `master.db`.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- Any future `EnrichCacheWriteMsg` variant must carry an acknowledgement and
  define which public counter advances only after persistence.
- Reviewers should trace each success/no-match increment back to an awaited
  `Ok(())`, and ensure error objects retain track/provider context.
- Plan 015 will change Discogs auth concurrency. It must preserve this
  acknowledgement boundary and must not reintroduce fire-and-forget writes.
- If per-write latency becomes material, batch writes inside the sole writer
  transaction while retaining one acknowledgement per message; do not weaken
  durability reporting.
