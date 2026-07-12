# Plan 002: Make XML export cancellation-safe and single-flight

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` unless the reviewer who dispatched you maintains the index.
>
> **Drift check (run first)**:
> `git diff --stat e6eb382..HEAD -- src/changes.rs src/tools/mod.rs src/tools/staging_handlers.rs src/tools/tests.rs`
> If any in-scope file changed since this plan was written, compare the
> "Current state" excerpts against the live code before proceeding; on a
> mismatch, treat it as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

`handle_write_xml` drains all staged changes before its first `await`. If the
MCP request is cancelled while backup or later work is pending, Rust drops the
owned snapshot and those user changes disappear because restoration currently
occurs only in explicit error branches. Concurrent exports can also overlap
`take()`/`restore()` operations even though `ChangeManager` has one global
`touched_since_take` ledger, allowing one export to clear or interpret another
export's mutation history. XML export needs an RAII snapshot guard that restores
on every uncommitted drop and a server-level async mutex held from before
`take()` through commit.

This plan preserves the non-negotiable architecture: Rekordbox `master.db`
remains read-only, and user-visible metadata still moves only through
`ChangeManager` and `write_xml`.

## Current state

- `src/changes.rs` owns staged changes and the touched-field merge ledger.
- `src/tools/mod.rs` owns shared server state and initializes existing locks.
- `src/tools/staging_handlers.rs` implements the async export lifecycle.
- `src/tools/tests.rs` contains MCP-level export/rollback tests and test server
  construction.

The shared ledger is reset by every full take (`src/changes.rs:133-153`):

```rust
pub fn take(&self, track_ids: Option<Vec<String>>) -> Vec<TrackChange> {
    let mut map = acquire_or_recover_lock(&self.changes);
    let mut touched = acquire_or_recover_lock(&self.touched_since_take);
    match track_ids {
        // ...
        None => {
            touched.clear();
            self.cleared_all_since_take.store(false, Ordering::Release);
            let mut drained: Vec<TrackChange> = map.drain().map(|(_, change)| change).collect();
            drained.sort_by(|a, b| a.track_id.cmp(&b.track_id));
            drained
        }
    }
}
```

Restoration merges the snapshot around fields touched after the take
(`src/changes.rs:155-189`):

```rust
pub fn restore(&self, snapshot: Vec<TrackChange>) -> (usize, usize) {
    // ... cleared-all handling ...
    let mut map = acquire_or_recover_lock(&self.changes);
    let touched = acquire_or_recover_lock(&self.touched_since_take);
    for change in snapshot {
        let touched_fields = touched.get(&change.track_id).unwrap_or(&empty_set);
        // merge only untouched fields back into current staged state
    }
    // ...
}
```

The handler drains before awaiting backup and restores only named error paths
(`src/tools/staging_handlers.rs:303-321`, `src/tools/staging_handlers.rs:338-392`):

```rust
let snapshot = server.state.changes.take(None);
// ...
let backup_status = match crate::backup::run_pre_op_backup().await {
    Ok(status) => status,
    Err(err) => {
        server.state.changes.restore(snapshot);
        return Err(mcp_internal_error(format!("pre-op {err}")));
    }
};
```

`ServerState` already centralizes process-wide coordination
(`src/tools/mod.rs:89-105`):

```rust
pub(super) struct ServerState {
    // ...
    pub(super) essentia_setup_lock: tokio::sync::Mutex<()>,
    pub(super) discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
    pub(super) changes: ChangeManager,
    // ...
}
```

Match the poison-recovery convention in `acquire_or_recover_lock` for sync
state. Use `tokio::sync::Mutex<()>` for the export mutex because it is held
across backup and other awaits.

## Commands you will need

| Purpose                  | Command                                                                                  | Expected on success                               |
| ------------------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------- |
| Change guard tests       | `cargo test -p reklawdbox change_snapshot_guard -- --nocapture`                          | exit 0; guard drop/commit tests pass              |
| Export concurrency tests | `cargo test -p reklawdbox write_xml -- --nocapture`                                      | exit 0; cancellation and overlap regressions pass |
| Format                   | `cargo fmt --check`                                                                      | exit 0; no diff                                   |
| Docs/config format       | `dprint check`                                                                           | exit 0                                            |
| Lint                     | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                | exit 0; no warnings                               |
| Tests                    | `cargo test -p reklawdbox --no-fail-fast`                                                | exit 0; all tests pass                            |
| Release build            | `cargo build --release`                                                                  | exit 0                                            |
| CLI smoke                | `./target/release/reklawdbox --version && ./target/release/reklawdbox --help >/dev/null` | exit 0                                            |

## Scope

**In scope** (the only source files you should modify):

- `src/changes.rs`
- `src/tools/mod.rs`
- `src/tools/staging_handlers.rs`
- `src/tools/tests.rs`

**Out of scope** (do not touch):

- `src/xml.rs`; destination replacement semantics are not this finding
- `src/backup.rs` and backup scripts
- `src/db.rs` or any Rekordbox database access mode
- XML schema/content, tool parameters, label-gate behavior, or playlist logic
- Replacing the touched-field ledger with a multi-generation redesign; the
  server mutex makes one export generation active at a time
- Any direct write to Rekordbox `master.db`

## Git workflow

- Branch: `codex/002-transactional-xml-export`
- Commit: `fix(xml): make export lifecycle transactional`
- Use Conventional Commits. Do not push or open a PR unless instructed.

## Steps

### Step 1: Add an RAII guard for taken changes

In `src/changes.rs`, add a public-to-crate guard type (for example,
`ChangeSnapshotGuard<'a>`) that:

- borrows its originating `ChangeManager`;
- owns the taken snapshot as `Option<Vec<TrackChange>>`;
- exposes `changes(&self) -> &[TrackChange]` and `len`/`is_empty` accessors;
- offers `commit(mut self)` that consumes or clears the snapshot so `Drop` does
  not restore it;
- calls `ChangeManager::restore` from `Drop` whenever a snapshot remains.

Add `ChangeManager::take_guard(...)` that performs the existing `take` and
immediately wraps the result. Keep the existing `take` and `restore` methods for
current unit tests and any future non-async low-level use; do not silently alter
their semantics.

Add tests named with the `change_snapshot_guard` prefix:

1. Dropping an uncommitted guard restores the snapshot.
2. Calling `commit` leaves the exported changes drained.
3. Staging or clearing fields after guard creation still wins over restored
   snapshot fields, matching existing touched-field behavior.
4. Spawn an async task that creates a guard, signals a `Notify`, then awaits
   forever; abort the task after the signal and assert the guard's `Drop`
   restored the changes. This is the cancellation regression and requires no
   filesystem or backup-process timing.

Wrap the complete cancellation scenario and every `Notify`/barrier wait and
task join in a five-second `tokio::time::timeout`. On every failure path, abort
and await any spawned task so the test cannot leak the deliberately pending
future. A missed signal must fail with a phase-specific message, not hang the
filtered crate test.

**Verify**: `cargo test -p reklawdbox change_snapshot_guard -- --nocapture` →
exit 0; all four guard lifecycle cases pass.

### Step 2: Add a server-level single-flight XML export mutex

Add `xml_export_lock: tokio::sync::Mutex<()>` to `ServerState` in
`src/tools/mod.rs`. Initialize it in both production construction
(`ReklawdboxServer::new`) and the explicit test constructor in
`src/tools/tests.rs`.

This is intentionally one mutex per server state, not a global process static
and not a file-path lock: the protected resource is the single
`ChangeManager` snapshot/touched ledger, regardless of output filename.

**Verify**: `cargo check -p reklawdbox` → exit 0 with every `ServerState`
initializer updated.

### Step 3: Hold the mutex and RAII guard across the full export

Refactor `handle_write_xml` in `src/tools/staging_handlers.rs`:

1. Preserve the label gate before any snapshot is taken.
2. Acquire `server.state.xml_export_lock.lock().await` before calling
   `take_guard(None)`.
3. Declare the snapshot guard after the mutex guard so cancellation drops and
   restores the snapshot before releasing the export lock.
4. Replace direct `snapshot` vector access with `guard.changes()`.
5. Remove every explicit `changes.restore(snapshot)` branch; ordinary `?` or
   early returns must rely on RAII restoration.
6. After XML writing succeeds, capture `changes_applied`, call `guard.commit()`,
   then build the success response. Never commit before the write succeeds.
7. Hold the export mutex until the guard is committed or dropped.

Playlist-only exports must still work with an empty change guard. A no-change,
no-playlist request may return while holding the guards; dropping an empty guard
is a no-op.

**Verify**: `cargo test -p reklawdbox write_xml -- --nocapture` → all existing
backup failure, missing-ID, label-gate, playlist-only, and retry tests pass.

### Step 4: Add overlapping-export regressions

In `src/tools/tests.rs`, add deterministic tests that do not depend on sleeps:

- Hold `xml_export_lock` from the test, queue two `write_xml` calls, and verify
  both remain before the `take` boundary while the lock is held (the staged
  count remains unchanged).
- Release the lock, await both calls, and assert exactly one export reports the
  staged change while the other reports no staged changes; neither loses or
  resurrects data and both futures complete.
- Cancel a queued second export and verify it never affects the active/next
  snapshot.

Use `tokio::sync::Barrier` or `Notify` plus `tokio::time::timeout` for ordering;
do not use arbitrary timing sleeps. Use separate tempfile output paths so the
test isolates `ChangeManager` coordination from destination-file contention.
Bound each complete overlap/cancellation scenario as well as its individual
waits, and keep all join handles in a cleanup guard that aborts/awaits them on
early assertion failure.

**Verify**: `cargo test -p reklawdbox write_xml -- --nocapture` → exit 0 and the
new overlap/cancellation tests pass repeatedly.

### Step 5: Run the repository gate

Run every command in "Commands you will need". Fix only failures caused by
in-scope edits.

**Verify**: each command exits 0 with its listed expected result.

## Test plan

- `src/changes.rs`: drop rollback, explicit commit, touched-field merge, and
  async task-abort rollback for the snapshot guard.
- `src/tools/tests.rs`: exports queue before `take`, one export owns one
  generation, cancelled waiters do not disturb it, and existing error retries
  still restore correctly.
- Existing `take`/`restore` unit tests remain unchanged and passing.
- Verification: targeted guard and `write_xml` filters, then the complete crate
  gate.

## Machine-checkable done criteria

- [ ] `handle_write_xml` contains no direct call to `changes.restore`.
- [ ] `handle_write_xml` acquires `xml_export_lock` before `take_guard` and calls
      `commit` only after successful XML writing.
- [ ] Aborting a future after its snapshot is taken restores all untouched
      staged fields.
- [ ] Two concurrent exports cannot have live snapshots simultaneously.
- [ ] Existing staging during an export still wins field-by-field if the export
      is cancelled or errors.
- [ ] Every cancellation/concurrency test has bounded waits and deterministic
      spawned-task cleanup; no pending future survives the scenario.
- [ ] `cargo fmt --check`, `dprint check`, clippy, full tests, release build, and
      CLI smoke all exit 0.
- [ ] `git diff --name-only` contains only the four in-scope source/test files
      and the plan/index status update.
- [ ] `plans/README.md` marks plan 002 DONE, unless the dispatcher owns the index.

## STOP conditions

Stop and report back instead of improvising if:

- Any current-state excerpt no longer matches after the drift check.
- A correct guard would require bypassing `restore`'s touched-field semantics or
  holding a synchronous mutex across `.await`.
- Another production caller begins an overlapping full `take(None)` lifecycle
  that cannot share the same export mutex.
- Tests show `Drop` restoration can panic; `Drop` must not panic during async
  cancellation/unwinding.
- The change appears to require editing `src/xml.rs`, backup behavior, tool
  schema, or any Rekordbox DB write boundary.
- Any verification fails twice for a reason unrelated to in-scope work.

## Maintenance notes

- The RAII guard is the cancellation-safety boundary; every future async export
  of taken changes must use it rather than owning a raw `Vec<TrackChange>`.
- The mutex protects one global touched ledger, not an output path. Do not
  weaken it to per-destination locking without first generation-scoping the
  ledger in `ChangeManager`.
- Review declaration and drop order carefully: snapshot restoration must occur
  while the export mutex is still held.
- A future generation-scoped `ChangeManager` could permit parallel exports, but
  that redesign is explicitly deferred until parallel export has a demonstrated
  product need.
