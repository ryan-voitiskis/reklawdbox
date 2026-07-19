# Plan 052: Encapsulate backup process supervision

> **Executor instructions**: This plan changes ownership around a deliberately
> defensive subprocess state machine. Characterize every success, failure,
> timeout, cancellation, and cleanup path before moving it. Preserve exact
> caller-visible messages and the fail-closed `write_xml` transaction. STOP if
> process-group identity or detached cleanup cannot be proven. Update the
> tracker only after independent process-lifecycle and API review plus the full
> gate.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/adapters/rekordbox/backup.rs \
>   src/adapters/rekordbox/mod.rs \
>   src/adapters/platform/mod.rs \
>   src/adapters/platform/process_group.rs \
>   src/application/metadata/export.rs \
>   src/cli/backup.rs \
>   src/mcp/tests/metadata/export_backup.rs \
>   src/mcp/tests/metadata/support.rs \
>   scripts/backup.sh
> ```
>
> Reconcile only reviewed test/module moves. STOP if the 120-second timeout,
> script-resolution order, process-group algorithm, output bound, embedded
> script, `write_xml` snapshot restoration, or CLI backup/restore contract
> changed after this planning commit.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: none
- **Category**: async lifecycle / typed errors / decomplexification
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

`src/adapters/rekordbox/backup.rs` is 1,120 lines. The pressure is not its line
count alone: one module currently owns five distinct responsibilities:

- custom-versus-embedded script resolution and temporary script lifetime;
- interactive CLI execution;
- bounded, captured pre-operation execution;
- child, process-group, deadline, cancellation, and reader-task supervision;
- platform-specific pre-reap process-group inspection and output rendering.

The captured path is intentionally spawned as a supervisor task. If its caller
is cancelled, dropping the outer future must detach rather than abort that task
so it can terminate descendants, reap the child, and join output readers. The
Unix path also must inspect group membership while the leader remains unreaped;
reaping first would release the numeric process-group identity and make later
group signalling unsafe.

Those invariants are correct but implicit across locals, cleanup helpers, and
`Result<_, String>` values. Moving helpers into files without making the one
supervisor's ownership and terminal states explicit would only disperse the
risk.

## Frozen behavior and safety contract

This plan must preserve:

- resolution order: non-empty `REKLAWDBOX_BACKUP_SCRIPT`, then the embedded
  `scripts/backup.sh`;
- `REKORDBOX_DB_PATH` forwarding and argument order;
- the test-build pre-op shortcut when no custom script is configured;
- a 120-second pre-operation timeout, one-second termination/reader cleanup
  bounds, five-millisecond leader polling, and strict rejection at the exact
  deadline;
- an 8 KiB bound per captured stream and the exact `" …[truncated]"` suffix;
- stderr-before-stdout diagnostic preference for non-zero exits;
- whole-process-group termination, direct-child reap, and reader abort/join on
  every incomplete path;
- pre-reap leader observation, group freeze/inspection, and refusal to release
  a group that still has descendants;
- cleanup continuing after caller cancellation because the supervisor task is
  detached, not aborted;
- current public/error strings, including cleanup context ordering;
- fail-closed XML export, staged-change restoration, and export-lock release
  after any backup failure; and
- the separate interactive CLI backup/restore flow with inherited stdio.

The embedded shell script's archive, restore, path, sidecar, and safety-backup
semantics are out of scope. No SQL mutation path may be added; optional
integration tests may operate only on a disposable database copy.

## Target ownership

Replace the single file with a directory whose canonical homes are:

- `backup/mod.rs` — public adapter entry points and narrow re-exports only;
- `script.rs` — embedded/custom script selection and temporary executable
  ownership;
- `interactive.rs` — inherited-stdio CLI execution only;
- `supervisor.rs` — the captured command request, detached supervisor task,
  lifecycle transitions, and final outcome;
- `backup/process_group.rs` — Tokio polling and guarded child-reap integration;
- `adapters/platform/process_group.rs` — the narrow non-async Unix group
  identity, pre-reap observation, membership inspection, release, termination,
  and platform cfg code shared later by Plan 051;
- `output.rs` — bounded stream capture, reader-task ownership, rendering, and
  truncation;
- `error.rs` — focused internal error kind, cleanup report, and stable display;
  and
- `tests/` — process lifecycle, output, script resolution, and platform parsing
  tests grouped by capability.

After adapter behavior is green, replace the 1,681-line
`src/mcp/tests/metadata/export_backup.rs` and its exclusively owned 648-line
support module with:

- `metadata/export_backup/mod.rs` — declarations only;
- `write_xml.rs` — export serialization, lock, staged-snapshot restoration,
  and retry behavior;
- `supervision.rs` — captured pre-op timeout, descendant, output, and
  cancellation behavior;
- `script.rs` — embedded shell backup/restore and path-safety integration; and
- `support.rs` — only the small environment/server fixtures genuinely shared
  by two or more of those modules.

Keep process/PID fixtures in `supervision.rs`, archive helpers in `script.rs`,
and task guards in `write_xml.rs`; do not recreate the old 648-line grab-bag
support file under a new path.

`mod.rs` must stay a navigation surface. Do not introduce a generic subprocess
crate or share this supervisor with the synchronous Essentia runner from Plan
051: their cancellation, I/O, and ownership contracts differ.

### One narrow platform primitive

Move the existing tested raw process-group ownership into
`adapters/platform/process_group.rs`. Its API may own only a leader PID/PGID
and synchronous operations to:

- observe leader exit without reaping;
- freeze and inspect membership while identity is reserved;
- terminate or relinquish the owned group; and
- report whether guarded child reap is now safe.

It must not own a `Child`, Tokio task, timeout, sleep/poll loop, command,
reader, or output. Backup keeps five-millisecond Tokio polling and child reap
in its adapter wrapper; Plan 051 later supplies its own synchronous polling.
This is the only shared process code authorized across the two plans.

### One concrete supervisor owner

Introduce a concrete `BackupSupervisor` that owns, from successful spawn until
one terminal outcome:

- `tokio::process::Child`;
- `ProcessGroupOwnership`;
- stdout and stderr `OutputReaderTask`s;
- the strict deadline; and
- test-only reader activity observation.

Its implementation may use a small state enum such as `Running`,
`LeaderObserved`, `GroupReleased`, and `Reaped`, but must not become a generic
typestate framework. Every normal exit should call one of two explicit terminal
operations:

1. `finish`, which observes the leader, inspects/releases the group, reaps,
   joins both readers, and renders status; or
2. `terminate`, which kills the owned group and direct child, reaps, aborts and
   joins readers, and returns a `CleanupReport`.

`Drop` remains emergency-only: it may signal/abort owned resources and log, but
normal control flow must not rely on asynchronous work from `Drop`.

### Make detached cancellation intentional

Keep an outer function that starts `tokio::spawn(supervisor.run(...))` and
awaits its `JoinHandle`. Encapsulate that handle in a narrowly named owner such
as `DetachedCleanupTask` whose `Drop` deliberately does **not** call `abort`.
Document why dropping the caller's future leaves cleanup running. Do not use
`JoinHandle::abort_on_drop`, a general task registry, or a background daemon.

The cancellation regression must drop the awaiting caller, then prove both
fixture PIDs exit and the reader activity count reaches zero. A passing
function result is not an oracle for this path because cancellation removes
the result receiver.

### Type decisions without changing edge strings

Replace lifecycle-internal `Result<_, String>` with a focused `BackupError`
and `BackupErrorKind`. Include only categories on which lifecycle or callers
make decisions, for example:

- script preparation or launch;
- output capture setup/read/join;
- deadline exceeded;
- exit observation or group inspection;
- descendant process detected;
- wait/reap; and
- non-zero exit.

Use a `CleanupReport` to collect ordered cleanup failures and captured cleanup
diagnostics. Convert to the existing string exactly once at the current adapter
edge; add snapshots for every established message. Do not expose these
internal variants through MCP/CLI schemas or replace the application's stable
error category unless a separate contract change is approved.

## Scope

**In scope**:

- `src/adapters/rekordbox/backup.rs` (replaced by the directory)
- `src/adapters/rekordbox/backup/mod.rs`
- `src/adapters/rekordbox/backup/script.rs`
- `src/adapters/rekordbox/backup/interactive.rs`
- `src/adapters/rekordbox/backup/supervisor.rs`
- `src/adapters/rekordbox/backup/process_group.rs`
- `src/adapters/rekordbox/backup/output.rs`
- `src/adapters/rekordbox/backup/error.rs`
- `src/adapters/rekordbox/backup/tests/**`
- `src/adapters/rekordbox/mod.rs`
- `src/adapters/platform/mod.rs`
- `src/adapters/platform/process_group.rs`
- `src/application/metadata/export.rs` only if the typed adapter error needs a
  single stable-string conversion
- `src/cli/backup.rs` only for import/type reconciliation with identical output
- `src/mcp/tests/metadata/export_backup.rs` (replaced by the directory below)
- `src/mcp/tests/metadata/export_backup/mod.rs`
- `src/mcp/tests/metadata/export_backup/write_xml.rs`
- `src/mcp/tests/metadata/export_backup/supervision.rs`
- `src/mcp/tests/metadata/export_backup/script.rs`
- `src/mcp/tests/metadata/export_backup/support.rs`
- `src/mcp/tests/metadata/support.rs` (removed after its helpers move to their
  capability owners)
- `src/mcp/tests/metadata/mod.rs`
- `tests/source_boundaries.rs` only for a narrow ownership regression
- `plans/README.md` status row only during execution

**Out of scope**:

- `scripts/backup.sh` behavior, archive formats, filenames, retention, restore
  algorithms, or interactive prompts.
- Rekordbox database writes, online-snapshot claims, cache/schema changes, MCP
  schemas, CLI flags, or user-visible message changes.
- Sharing a command runner, supervisor, timeout, output capture, or cleanup
  policy with Essentia, test extraction, or other commands. Only the narrow
  process-group identity primitive described above is shared.
- Replacing the existing process-group algorithm, unsafe calls, `/proc` scan,
  or macOS `proc_listpgrppids` implementation without a focused failing test
  and separately approved scope.
- Private fixture data, live `master.db`, user backup directories, deployment,
  release, or host-binary testing.

## Steps

### Step 1: Freeze lifecycle and public-message behavior

Before moving code, retain or add deterministic tests for:

1. success with stdout and stderr;
2. non-zero exit with stderr preference, stdout fallback, and no-output text;
3. direct-child timeout;
4. descendant timeout while output pipes remain open;
5. leader early exit with a descendant holding pipes;
6. leader early exit with a descendant that closed both pipes;
7. caller cancellation while parent, descendant, and readers are live;
8. stdout/stderr setup failure and reader failure/panic;
9. output truncation at, below, and above 8 KiB;
10. completion observed at or after the exact deadline;
11. process-group inspection/setup failure with cleanup context; and
12. cleanup failure aggregation and stable ordering.

Every fixture-ready marker, PID observation, deadline, reader count, and
complete scenario must have a short timeout. Use files/channels and PID probes,
not unbounded waits or sleeps as the correctness oracle. After every incomplete
path, assert no direct or descendant PID and no output reader remains.

Focused commands:

```bash
cargo test -p reklawdbox pre_op_backup_ -- --nocapture
cargo test -p reklawdbox write_xml_backup_ -- --nocapture
cargo test -p reklawdbox adapters::rekordbox::backup -- --nocapture
```

### Step 2: Extract bounded output as one resource owner

Move `BoundedOutput`, `OutputReaderTask`, activity tracking, rendering, and
cleanup result mapping into `output.rs`. Keep each reader's handle private and
enforce exactly one finish/abort-and-join path. Preserve cancellation
responsiveness when a noisy descendant keeps a pipe continuously readable.

Do not make output capture configurable beyond the current internal test seam.
The 8 KiB limit and diagnostic precedence are product behavior for failures.

### Step 3: Isolate process-group identity without algorithm drift

Move raw `ProcessGroupOwnership`, leader observation, member inspection, and
signal/release logic into the platform primitive. Keep Tokio polling and
guarded child reap in `backup/process_group.rs`. Keep unsafe calls minimal and
adjacent to their current SAFETY explanation. Preserve these transitions:

1. the unreaped leader reserves the process-group ID;
2. the group is frozen before membership inspection;
3. a leader-only group is relinquished without signalling;
4. a group with descendants is killed before ownership is released; and
5. the direct child is reaped only after group ownership is released.

Keep Linux proc-stat parser and macOS membership tests platform-gated. Require
manual independent review of this diff even if every test passes.

Add an architecture/source regression proving the platform module imports no
audio, Rekordbox, Tokio, CLI, or MCP types. Its API must remain useful to the
future synchronous Essentia runner without becoming an executor abstraction.

### Step 4: Introduce typed supervisor state and cleanup reporting

Create `BackupSupervisor` and migrate the existing sequence without changing
ordering or deadlines. Make partial setup transfer ownership immediately so a
missing pipe or group-setup error cannot leak the already-spawned child. Make
cleanup idempotent: once group ownership, a reader handle, or child reap is
consumed, a later emergency path must not repeat it.

Return typed errors internally, attach `CleanupReport` once, and snapshot the
final display for all existing failure categories. Do not erase primary errors
when cleanup also fails.

### Step 5: Make detached cleanup and script lifetime explicit

Move script selection/materialization to `script.rs`; its owner must keep the
temporary directory alive until the supervisor completes, including after
caller cancellation. Keep the custom-script existence check and environment
handling unchanged.

Wrap the spawned supervisor handle so its intentional detach-on-drop contract
is documented and tested. Interactive CLI execution remains in
`interactive.rs` and must not silently adopt captured pre-op timeouts or
process-group semantics.

### Step 6: Preserve export transaction behavior

Rerun the `write_xml` failure matrix for non-zero exit, missing script,
timeout, early-exit descendant, and caller cancellation. Each failing backup
must:

- prevent XML creation;
- restore exactly the staged snapshot;
- release the export lock;
- permit a subsequent successful retry; and
- preserve current error category/message.

This plan may adjust one conversion at the application boundary, but it must
not move `ChangeManager` ownership or backup sequencing out of the current
export workflow.

Only after the adapter and export transaction tests are green, split the MCP
test files into the capability layout above without changing or deleting an
assertion. Move each helper to the narrowest consumer; `support.rs` must remain
small and contain no process state machine, archive builder suite, or task
cleanup implementation.

### Step 7: Run safe script integration and the full gate

Mandatory embedded-script integration uses a synthetic/sanitized SQLCipher
fixture under a temporary `HOME` with an explicit temporary
`REKORDBOX_DB_PATH`. It must prove the live source fixture checksum is
unchanged and all archives are confined to the temporary home. Never point a
test at the user's live `master.db` or normal backup directory.

Private Rekordbox data is not needed for this supervisor refactor. If Plan 047
is already available and an executor chooses an opt-in private smoke, first
copy the database and any needed sidecars into a unique temporary directory,
hash the originals before/after, and set a temporary `HOME`. Mark the gate
`NOT RUN` when unavailable; do not weaken mandatory synthetic coverage.

Run:

```bash
cargo test -p reklawdbox pre_op_backup_ -- --nocapture
cargo test -p reklawdbox write_xml_backup_ -- --nocapture
cargo test -p reklawdbox backup_script_custom_path_ -- --nocapture
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

Require independent async/process review to trace spawn failure, partial setup,
success, non-zero exit, exact deadline, early leader exit, descendants,
reader failure, caller cancellation, and every cleanup failure. Require a
separate API review of all final strings and `write_xml` state restoration.

## Machine-checkable done criteria

- [ ] One concrete supervisor owns the child, process group, deadline, and both
      readers from spawn through finish or termination.
- [ ] Caller cancellation intentionally detaches the supervisor, and a bounded
      regression proves eventual child/descendant/reader quiescence.
- [ ] Process-group identity remains protected until inspection/release, and
      the leader cannot be reaped early.
- [ ] The shared platform primitive owns only synchronous PGID identity and
      signalling; backup retains all command, Tokio, deadline, child, output,
      and cancellation policy.
- [ ] Every reader task is joined or aborted then joined on every path.
- [ ] Internal lifecycle decisions use typed errors and an ordered cleanup
      report; all edge-visible messages remain byte-for-byte stable.
- [ ] Script selection, temporary-script lifetime, environment, arguments,
      timeout, output limit, and interactive CLI behavior are unchanged.
- [ ] `write_xml` still fails closed, restores staged changes, releases its
      lock, and supports retry after every backup failure.
- [ ] Export/backup MCP tests are split into write-XML, supervision, and script
      capabilities with no assertion loss and no replacement mega-support
      module.
- [ ] `mod.rs` is navigation only; no shared/general subprocess abstraction was
      introduced.
- [ ] Mandatory synthetic process/script tests leave no process or temporary
      resource behind; any optional private gate passed or is explicitly
      `NOT RUN`.
- [ ] Full architecture, workspace, release, MCP, docs-contract, site, and
      diff gates pass.

## STOP conditions

Stop and report if:

- caller cancellation cannot be proven to leave the detached supervisor
  cleaning up;
- safe descendant detection requires reaping the leader before group
  inspection or otherwise changing process-group identity rules;
- any platform's unsafe/process-group algorithm must change;
- cleanup ownership cannot be made idempotent without changing timeout or
  failure precedence;
- typed errors alter a public category, exact message, serialization, CLI/MCP
  output, or `write_xml` retry semantics;
- validation would require writing the live Rekordbox database or normal user
  backup directory; or
- the implementation tends toward a generic cross-command process framework.

## Complexity accounting

Success localizes each hazardous primitive, replaces implicit cleanup coupling
with one explicit supervisor owner, and removes stringly lifecycle decisions
without changing the string boundary. Splitting 1,120 lines while preserving
the same cross-file mutable state, or wrapping helpers without deleting the
old orchestration, is only movement and must be rejected.

## Git workflow

- Branch: `codex/052-encapsulate-backup-supervision`
- Preferred commit: `refactor(backup): encapsulate process supervision`
- Do not push, merge, release, deploy, or touch a live database/backup.
