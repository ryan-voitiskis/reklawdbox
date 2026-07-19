# Plan 051: Separate the Essentia runtime lifecycle

> **Executor instructions**: This plan touches subprocess, filesystem
> transaction, symlink, advisory-lock, and unsafe platform code. Add/confirm
> focused lifecycle regressions before moving code. Preserve the exact managed
> runtime, cache identity, error categories, rollback, cleanup, and CLI/MCP
> setup behavior. STOP if an invariant cannot be proven. Update the tracker
> only after independent async/process and API review plus the full gate.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plans 047, 048, and 052 are reviewed `DONE`; start from their
>    integrated head. Plans 047/048 touch `adapters/audio` tests/module
>    declarations, while 052 establishes the narrow platform process-group
>    ownership primitive required to avoid unsafe post-reap signalling.
> 2. Run:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/adapters/audio/essentia_environment.rs \
>   src/adapters/audio/essentia.rs \
>   src/adapters/audio/mod.rs \
>   src/adapters/audio/tests.rs \
>   src/adapters/platform/process_group.rs \
>   src/application/analysis/setup.rs \
>   src/cli/setup.rs \
>   src/mcp/analysis/handlers.rs \
>   src/mcp/tests/analysis.rs
> ```
>
> Reconcile only reviewed module/test moves. STOP if the pinned manifest,
> managed path, generation layout, lock, schema v3 identity, or setup result
> changed after this planning commit.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: HIGH
- **Depends on**: 047, 048, 052
- **Category**: process lifecycle / transactional filesystem / decomplexification
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

`src/adapters/audio/essentia_environment.rs` is 1,672 lines, with 1,022 lines
before its main test module. It owns four separate mechanisms:

- exact runtime contract parsing and source priority;
- synchronous child-process execution and timeout cleanup;
- managed generation installation under an advisory lock; and
- atomic stable-path activation, rollback, and pruning with platform-specific
  unsafe calls.

Those are real ownership boundaries. More importantly, runtime probing
(`inspect_essentia_python_with_timeout_result`, lines 143-270) and installer
command execution (`SystemEnvironmentOps::run`, lines 366-464) independently
spawn a process group, drain two pipes on threads, poll a timeout, terminate,
wait, and join readers. Their failure categories already diverge in places.

The install function then implements a long implicit transaction from stable
probe through lock, generation build, candidate validation, symlink switch,
stable validation, rollback, commit, and pruning. Moving chunks into files
without one command owner and one explicit activation transaction would only
hide the lifecycle.

## Frozen runtime and compatibility contract

This plan must not change:

- managed path: `~/.local/share/reklawdbox/essentia-venv`;
- generation root: `essentia-venv.generations`;
- CPython `3.14.*` requirement and candidate order `python3.14`, `python3`;
- Essentia `2.1b6.dev1438`, module `2.1-beta6-dev`, NumPy `2.5.1`, PyYAML
  `6.0.3`, and six `1.17.0`;
- `ESSENTIA_CONTRACT_ID` and `ESSENTIA_SCHEMA_VERSION = "3"`;
- wheel-only, no-deps installation arguments;
- explicit override priority over the managed default;
- interprocess `flock` timeout/poll behavior;
- validate-before-switch, post-switch validation, rollback, and safe pruning;
- `EssentiaSetupErrorKind` serialized spellings and edge-visible messages; and
- graceful availability of non-classification workflows without Python.

No cache bump is expected. If output or runtime identity must change, STOP and
write a compatibility plan first.

## Target ownership

Replace the single file with:

- `essentia_environment/mod.rs` — constants/re-exports and public adapter
  entry points only;
- `contract.rs` — manifest value, probe JSON parsing, validation, and source
  priority;
- `process.rs` — the one synchronous bounded command owner;
- `install.rs` — candidate selection and generation build orchestration;
- `activation.rs` — managed paths, advisory lock, activation transaction,
  rollback, and pruning;
- `platform.rs` — only platform-specific atomic exchange/lock primitives and
  their safety comments; and
- `tests/` — contract, process, installation, and activation capabilities with
  explicit fake-command support.

### One command owner

Replace `EnvironmentOps::{run, inspect_runtime}` with a narrow command runner
whose one operation returns status/stdout/stderr. Runtime probing builds a
command request, calls that runner, then parses/validates JSON. Installation
uses the same runner for Python selection, venv creation, pip, and candidate or
stable probes.

The production runner must own, on every exit:

1. a child in a new process group;
2. stdout/stderr reader threads;
3. one total deadline covering leader wait and reader completion;
4. leader-exit observation without reap;
5. group freeze/inspection/release through Plan 052's reviewed
   `adapters::platform::process_group` primitive;
6. whole-group termination on timeout, error, or surviving descendants;
7. direct-child reap only after group ownership is released; and
8. reader joins.

Do not share this synchronous runner with the Tokio backup supervisor. Their
cancellation and output contracts are different. The two runners may share
only the low-level, non-async process-group identity primitive established by
Plan 052; it owns no command, timeout, child handle, output, or task.

### An explicit activation transaction

Introduce a concrete `ActivationTransaction` (not generic typestate) with
observable phases:

- prepared replacement;
- switched with remembered previous target;
- stable path validated;
- committed and previous runtime pruned; or
- rolled back with the rejected generation retained/removed according to the
  existing failure case.

Rollback must be an explicit fallible operation so its error can be combined
with the primary validation failure exactly as today. `Drop` may perform only
best-effort emergency cleanup and must log failures; normal paths must call
commit or rollback. Keep `IncompleteGeneration` ownership or absorb it into a
similarly narrow generation guard.

## Scope

**In scope**:

- `src/adapters/audio/essentia_environment.rs` (replaced by the directory)
- `src/adapters/audio/essentia_environment/mod.rs`
- `src/adapters/audio/essentia_environment/contract.rs`
- `src/adapters/audio/essentia_environment/process.rs`
- `src/adapters/audio/essentia_environment/install.rs`
- `src/adapters/audio/essentia_environment/activation.rs`
- `src/adapters/audio/essentia_environment/platform.rs`
- `src/adapters/audio/essentia_environment/tests/**`
- `src/adapters/audio/essentia.rs` only for import/re-export reconciliation
- `src/adapters/audio/mod.rs`
- `src/adapters/audio/tests.rs` only for existing/private round-trip wiring
- `src/adapters/platform/process_group.rs` only to consume its reviewed API;
  change it only if a missing invariant is proven and Plan 052 is re-reviewed
- `src/application/analysis/setup.rs`
- `src/cli/setup.rs` only if typed errors need identical rendering
- `src/mcp/analysis/handlers.rs` only if typed errors need identical rendering
- focused setup/analysis tests in `src/mcp/tests/analysis.rs`
- `tests/source_boundaries.rs` only for a narrow module-direction regression
- `plans/README.md` status row only during execution

**Out of scope**:

- Dependency/package upgrades, alternate Python versions, source builds,
  redistribution, license changes, or another managed path.
- Essentia analysis Python, feature extraction, output JSON, classifier
  readiness, calibration, cache schema/version, or analyzer contract changes.
- CLI/MCP setup schema, strings, auth, host deployment, or documentation.
- An async subprocess framework, shared command runner, executor crate, or
  trait hierarchy. Sharing is limited to Plan 052's process-group identity
  primitive.
- Rewriting unsafe calls or platform behavior unless a focused failing
  regression proves the current algorithm wrong and the user approves scope.
- Deleting any installed runtime, ignored `.mcp.json`, or private audio.

## Steps

### Step 1: Characterize process ownership before refactoring

Add mandatory Unix tests using temporary executable scripts and PID files for:

1. success with stdout and stderr;
2. non-zero exit with both diagnostic streams;
3. timeout of the direct child;
4. timeout with a descendant that holds stdout/stderr open;
5. leader early exit with a descendant that holds pipes open;
6. leader early exit with a descendant that closes both pipes;
7. reader-thread completion/panic reporting;
8. missing stdout/stderr setup failure; and
9. concurrent probes using independent process groups.

Every PID wait, reader join, barrier, and complete scenario must be bounded.
After each timeout/failure, prove child and descendant PIDs are gone and no
reader thread remains active. Do not use fixed sleeps as the oracle.

The planning-base loops call `Child::try_wait` before joining readers. The
early-exit fixtures must run behind an outer helper-process watchdog so a
pre-existing blocked reader cannot hang the Rust test process. If the reviewed
Plan 052 primitive cannot preserve group identity until descendant inspection,
STOP; never signal a numeric process-group ID after reaping its leader.

Focused command:

```bash
cargo test -p reklawdbox essentia_environment_process_ -- --nocapture
```

### Step 2: Route every command through one runner

Implement the command request/result and production runner in `process.rs`.
Make contract probing consume it, then delete the duplicate spawn/poll/read
loop. Preserve exact timeout values and diagnostic precedence. Keep runtime
JSON parsing/manifest mismatch in `contract.rs`, not the process runner.

Use the platform process-group primitive to observe the leader without reap,
freeze/inspect the group, reject and terminate surviving descendants, release
ownership, then reap. Reader completion remains inside the same command
deadline. This deliberately closes the pre-existing early-exit pipe-holder
hang; it must not change successful managed Python/pip behavior or established
error categories/messages.

The fake runner should script command results and record calls; avoid a mock
framework. Existing installer tests must continue to assert candidate order,
arguments, timeouts, and direct/stable probe sequence.

### Step 3: Type internal filesystem and activation errors

Make `ManagedEnvironmentPaths::from_stable`, switch, restore, and atomic
exchange return `EssentiaSetupError` (or a small activation-internal typed
error converted once), eliminating the remaining internal `Result<_, String>`
boundaries where the installer branches on category.

Preserve `EssentiaSetupErrorKind` and the exact edge-facing message text with
snapshot assertions. Do not expose low-level path/OS variants in MCP schema.

### Step 4: Extract the activation transaction unchanged

Move switch/restore/prune code into `activation.rs`, introduce the explicit
transaction owner, and migrate the installer without changing algorithm order.
Tests must cover:

- no prior stable path;
- prior managed relative symlink;
- external absolute symlink (never pruned);
- legacy directory atomic exchange;
- candidate succeeds but stable probe fails and rollback succeeds;
- rollback fails and the directly validated generation is preserved;
- generation root is a symlink or non-directory;
- lock timeout/reacquisition; and
- incomplete generation cleanup on venv, pip, wheel, and probe failures.

Keep unsafe exchange code in `platform.rs` with current SAFETY reasoning and
platform cfg branches. A reviewer must inspect that diff independent of tests.

### Step 5: Make install orchestration read as phases

`install_managed_essentia_at` should become a short orchestration over:

1. fast stable probe;
2. parent + lock;
3. second stable probe;
4. candidate selection;
5. generation build and validation;
6. activation transaction and stable validation; and
7. commit/prune.

Do not hide all state in an `InstallerContext`. Paths, runner, and lock policy
may be explicit inputs to a concrete installer request because they define one
installation transaction.

### Step 6: Run managed-runtime and private-audio validation

Mandatory offline tests use fake commands and temporary files. When the
managed runtime and Plan 047 private fixture are available, also run:

```bash
CRATE_DIG_ESSENTIA_PYTHON="$HOME/.local/share/reklawdbox/essentia-venv/bin/python" \
  cargo test -p reklawdbox \
    analyze_track_audio_essentia_cache_round_trip_real_track \
    -- --ignored --nocapture
```

Before that test, directly validate the exact manifest. Use a temporary
Reklawdbox-owned cache store. Do not refresh the user's shared cache or alter
source audio. If unavailable, report the gate `NOT RUN`; do not install or
delete environments as an incidental test action.

### Step 7: Full gate and lifecycle review

Run:

```bash
cargo test -p reklawdbox essentia_environment_ -- --nocapture
cargo test -p reklawdbox setup_essentia -- --nocapture
cargo test -p reklawdbox adapters::state -- --nocapture
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

Require an independent process/resource review to trace success, non-zero,
timeout, reader failure, switch failure, stable-probe failure, rollback failure,
and drop paths. Also cancel an MCP setup future after its blocking transaction
starts and prove the installation guard/lock, generation cleanup, and a
subsequent setup remain sound. “Tests pass” is not sufficient.

## Machine-checkable done criteria

- [ ] One production command runner owns every Essentia child process, process
      group, output reader, timeout, termination, reap, and join.
- [ ] Runtime probing parses/validates command output and no longer implements
      a second lifecycle loop.
- [ ] Leader exit is observed without reap; surviving descendants are
      terminated, group ownership is released, and only then is the child
      reaped.
- [ ] Activation/rollback/pruning are owned by one explicit fallible
      transaction with emergency-only drop cleanup.
- [ ] Internal path/activation decisions are typed; edge error kinds/messages
      remain unchanged.
- [ ] `mod.rs` is navigation only and unsafe code is isolated with unchanged
      safety invariants.
- [ ] Pinned manifest, candidate order, arguments, timeouts, lock, managed path,
      generation layout, contract ID, and cache schema are unchanged.
- [ ] Mandatory child/descendant/reader and transaction tests are bounded and
      leave no processes or temporary generations.
- [ ] The opt-in managed real-audio gate passed or is explicitly `NOT RUN`.
- [ ] Full architecture, workspace, release, MCP, docs-contract, site, and
      diff gates pass.

## STOP conditions

Stop and report if:

- child/descendant/reaper semantics cannot be proven with deterministic tests;
- the reviewed Plan 052 process-group primitive is insufficient and would need
  a new ownership or signalling algorithm;
- a platform's atomic exchange or rollback behavior would need an algorithm
  change;
- a rollback failure cannot preserve the validated generation as today;
- consolidating runners changes error text/category or diagnostic precedence;
- the exact managed manifest or real-audio round trip fails and a version/cache
  change would be needed;
- unsafe or process code expands beyond the existing boundary; or
- implementation tends toward a cross-repository subprocess framework.

## Complexity accounting

Success removes one duplicate subprocess state machine, makes installation
phases and rollback ownership explicit, and confines unsafe platform
primitives. Splitting 1,672 lines across files without those changes is only
movement and must be rejected.

## Git workflow

- Branch: `codex/051-separate-essentia-lifecycle`
- Preferred commit: `refactor(audio): separate essentia runtime lifecycle`
- Do not push, merge, release, deploy, install, or delete a runtime.
