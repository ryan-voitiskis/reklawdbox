# Plan 003: Serialize mutations by canonical audio-file identity

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` unless the reviewer who dispatched you maintains the index.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plan 002 is reviewed and marked `DONE` in `plans/README.md`.
> 2. Start from that reviewed result, then run:
>    `git diff --stat e6eb382..HEAD -- src/tools/mod.rs src/tools/file_tag_handlers.rs src/tools/tests.rs src/tags.rs`
>
> Plan 002 is expected to add XML-export state in `src/tools/mod.rs` and tests
> in `src/tools/tests.rs`; preserve that reviewed result. Other behavioral
> drift in an in-scope file is unexpected. Compare the excerpts below with
> live code and STOP on a mismatch rather than merging an unrelated change by
> guesswork.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/002-transactional-xml-export.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Each `write_file_tags` and `embed_cover_art` request creates its own
eight-permit semaphore. It limits work inside that request but does not
coordinate concurrent MCP requests, duplicate paths, or symlink/path aliases.
Both native tag writers use read-modify-save sequences, so overlapping
mutations of one physical file can lose fields or artwork, collide on a WAV
temporary path, or corrupt the file. Serialize only mutations that resolve to
the same canonical existing audio file while retaining the current eight-way
parallelism across different files.

This concerns explicitly requested native audio-file edits only. It must not add
any write path to Rekordbox `master.db`, and it does not replace the
`ChangeManager`/XML-only path for Rekordbox metadata.

## Current state

- `src/tools/file_tag_handlers.rs` schedules native tag and cover-art writes.
- `src/tools/mod.rs` owns shared server state and routes MCP tools.
- `src/tags.rs` contains the synchronous read-modify-save implementation.
- `src/tools/tests.rs` constructs explicit `ServerState` instances.

Per-request tag-write concurrency (`src/tools/file_tag_handlers.rs:197-213`):

```rust
let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
for entry in entries {
    let sem = semaphore.clone();
    handles.push(tokio::task::spawn(async move {
        let _permit = sem.acquire().await.expect("semaphore is never closed");
        tokio::task::spawn_blocking(move || tags::write_file_tags(&entry)).await
        // ...
    }));
}
```

Cover-art embedding independently creates another semaphore
(`src/tools/file_tag_handlers.rs:296-314`):

```rust
let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(8));
for target in params.target_audio_files {
    // ...
    handles.push(tokio::task::spawn(async move {
        let _permit = sem.acquire().await.expect("semaphore is never closed");
        tokio::task::spawn_blocking(move || tags::embed_cover_art(&img, &tgt, &pt)).await
        // ...
    }));
}
```

The underlying critical sections re-read then save the target. Tag fields are
written at `src/tags.rs:842-918`; artwork is read, changed, and saved at
`src/tags.rs:1155-1198`.

`ServerState` is the existing shared coordination boundary
(`src/tools/mod.rs:89-105`). Tool routing currently omits the server argument
for both mutators (`src/tools/mod.rs:624-648`). The lock registry belongs in
this shared state so cloned servers and separate tool calls converge on the
same per-file lock.

## Commands you will need

| Purpose            | Command                                                                                  | Expected on success                                   |
| ------------------ | ---------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| Focused tests      | `cargo test -p reklawdbox audio_file_mutation -- --nocapture`                            | exit 0; identity/serialization/concurrency tests pass |
| Existing tag tests | `cargo test -p reklawdbox tags::tests -- --nocapture`                                    | exit 0                                                |
| Format             | `cargo fmt --check`                                                                      | exit 0; no diff                                       |
| Docs/config format | `dprint check`                                                                           | exit 0                                                |
| Lint               | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                | exit 0; no warnings                                   |
| Tests              | `cargo test -p reklawdbox --no-fail-fast`                                                | exit 0; all tests pass                                |
| Release build      | `cargo build --release`                                                                  | exit 0                                                |
| CLI smoke          | `./target/release/reklawdbox --version && ./target/release/reklawdbox --help >/dev/null` | exit 0                                                |

## Scope

**In scope** (the only source files you should modify):

- `src/tools/mod.rs`
- `src/tools/file_tag_handlers.rs`
- `src/tools/tests.rs`

**Read for contract, but do not modify**:

- `src/tags.rs`

**Out of scope** (do not touch):

- Tag encoding, WAV copy/rename behavior, or cover-art algorithms in
  `src/tags.rs`
- Read-only tag reads, dry-run calculation, and cover-art extraction
- CLI tag commands; a single CLI process executes requested mutations
  sequentially
- Cross-process file locking; this plan coordinates one MCP server process
- Rekordbox DB access, `ChangeManager`, XML export, cache writes, and public
  response shapes
- A global one-per-server mutation mutex; it would unnecessarily remove the
  existing concurrency across independent files
- New dependencies; standard `Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>`
  is sufficient

## Git workflow

- Base: reviewed DONE commit from Plan 002
- Branch: `codex/003-serialize-audio-file-mutations`
- Commit: `fix(tags): serialize audio file mutations`
- Use Conventional Commits. Do not push or open a PR unless instructed.

## Steps

### Step 1: Add a bounded-lifecycle keyed lock registry

In `src/tools/mod.rs`, add a server-state field equivalent to:

```rust
Mutex<HashMap<PathBuf, Weak<tokio::sync::Mutex<()>>>>
```

Initialize it empty in `ReklawdboxServer::new` and the explicit test constructor
in `src/tools/tests.rs`. Add a small server/state helper that, while holding the
synchronous map mutex only briefly:

1. Removes entries whose `Weak` value can no longer be upgraded.
2. Upgrades and returns the existing lock for a canonical path when present.
3. Otherwise creates `Arc<tokio::sync::Mutex<()>>`, stores `Arc::downgrade`, and
   returns the strong `Arc`.

Never hold the map mutex across `.await`. Weak entries are required so a long-
running server does not permanently retain one lock per file ever edited.

**Verify**: `cargo check -p reklawdbox` → exit 0 with every `ServerState`
initializer updated and no sync guard crossing an await.

### Step 2: Resolve canonical identities and group aliases before scheduling

Change `handle_write_file_tags` and `handle_embed_cover_art` to accept
`&ReklawdboxServer`, and update the tool methods in `src/tools/mod.rs` to pass
`self`.

For non-dry tag writes and embeds, resolve every existing target with
`tokio::fs::canonicalize` before obtaining its keyed lock. Canonicalization
failure must become that target's existing per-file error result; do not use an
uncanonicalized fallback key because it would let aliases bypass serialization.

Group input operations by canonical `PathBuf` while retaining each input's
original index. Operations for one identity must stay in input order; results
must be restored to original request order. Symlink and relative/absolute
aliases of one file therefore share one group and one lock. Use std
collections—do not add an ordered-map dependency.

Dry runs remain read-only and keep their existing path. Do not canonicalize or
group them in this plan.

**Verify**: `cargo check -p reklawdbox` → exit 0; tool schemas and JSON result
types remain unchanged.

### Step 3: Run each identity group under one keyed async lock

For each canonical-identity group:

1. Acquire one permit from the existing request-local eight-way semaphore.
2. Obtain the shared lock `Arc` from the server registry.
3. Await its async mutex.
4. Run that group's operations sequentially, in input order, using
   `spawn_blocking`; keep the async guard alive through every blocking join.
5. Return indexed results, sort/assemble them into original request order, and
   preserve all current summary accounting.

Tag-write and embed handlers must call the same registry helper, so a tag update
and an artwork update for one physical file cannot overlap. Different canonical
files must still be able to occupy separate semaphore permits and mutate in
parallel.

**Verify**: `cargo test -p reklawdbox tags::tests -- --nocapture` → exit 0; all
existing native tag behavior remains intact.

### Step 4: Add deterministic identity and concurrency regressions

In `src/tools/tests.rs`, add tests named with the `audio_file_mutation` prefix.
Use synthetic files inside `tempfile::TempDir`; do not use private music.

Required cases:

1. Two aliases of the same target (relative/absolute where reliable, plus a
   symlink under `#[cfg(unix)]`) return the same lock `Arc` and execute serially.
2. Duplicate aliases in one tag-write request preserve input order and do not
   lose either append/prepend comment update.
3. Concurrent tag-write and artwork-embed calls against one file preserve both
   results, proving both handlers share the registry.
4. Hold file A's keyed lock, start operations for A and B, and assert B completes
   while A remains blocked; this proves different files retain concurrency.
5. Drop all strong lock references, request another lock, and assert stale weak
   entries are cleaned so registry size remains bounded by live operations.

Use `Barrier`/`Notify` and `tokio::time::timeout`, not arbitrary sleeps. A
test-only registry-length accessor may be `#[cfg(test)]`; do not expose it in
the production API.

Give every scenario a five-second outer timeout and bound each barrier/notify
wait and spawned-operation join individually. Own all join handles in a test
cleanup guard that aborts and awaits unfinished tasks on an early failure; an
alias/lock-order bug must fail rather than strand a file mutation task.

**Verify**: `cargo test -p reklawdbox audio_file_mutation -- --nocapture` → exit
0 and all five cases pass repeatedly.

### Step 5: Run the repository gate

Run every command in "Commands you will need". Fix only failures caused by the
three in-scope files.

**Verify**: all commands exit 0 with their listed expected result.

## Test plan

- Canonical identity convergence for relative/absolute and symlink aliases.
- In-request duplicate ordering and no lost read-modify-write update.
- Cross-request, cross-tool serialization for one physical file.
- Demonstrated parallel progress for two different files.
- Weak-entry cleanup to prevent registry growth.
- Existing `src/tags.rs` format-specific tests remain unchanged and passing.

## Machine-checkable done criteria

- [ ] Both non-dry tag writes and embeds canonicalize existing targets and use
      the same server-owned keyed registry.
- [ ] Aliases of one physical file cannot mutate concurrently, within or across
      requests.
- [ ] Different canonical files still make concurrent progress up to the
      request's eight-permit bound.
- [ ] Every keyed-lock race has bounded waits and deterministic task cleanup;
      no test can hang on a missed signal or retained lock owner.
- [ ] The registry removes dead `Weak` entries and retains no permanent per-file
      strong references.
- [ ] Result ordering, per-file errors, summary counts, and tool JSON shapes are
      unchanged.
- [ ] No new crate dependency is added.
- [ ] `cargo fmt --check`, `dprint check`, clippy, full tests, release build, and
      CLI smoke all exit 0.
- [ ] `git diff --name-only` contains only the three in-scope source/test files
      and the plan/index status update.
- [ ] `plans/README.md` marks plan 003 DONE, unless the dispatcher owns the index.

## STOP conditions

Stop and report back instead of improvising if:

- Plan 002 is not reviewed `DONE`, or its transactional XML-export state
  cannot be preserved while adding the mutation registry.
- Any current-state excerpt no longer matches after drift checking.
- A target must be mutated before it exists, making canonical identity
  unavailable; the selected design assumes existing audio targets.
- A mutation path other than tag write or artwork embed is found to edit audio
  concurrently; report it before broadening scope.
- Correctness appears to require cross-process locks, changing native tag bytes,
  or changing public response schemas.
- An implementation would hold the registry's synchronous map lock across
  `.await` or keep permanent strong lock references.
- Tests require a new runtime dependency or private local media.
- Any verification command fails twice for a reason unrelated to in-scope work.

## Maintenance notes

- Every future MCP audio-file mutator must canonicalize the existing target and
  use this registry for the complete read-modify-write lifecycle.
- Reviewers should test aliases, not just identical path strings.
- Keep result assembly indexed because grouping by identity otherwise changes
  externally visible response order.
- Cross-process coordination is intentionally deferred. If multiple server
  processes commonly mutate the same library, add an OS/file-lock design as a
  separate plan rather than weakening this registry.
- Preserve Plan 002's XML-export lock whenever `ServerState` construction or
  test fixtures change.
- Plans 019 and 020 must reconcile these reviewed handler changes and must not
  reintroduce per-request-only coordination.
