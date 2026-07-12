# Plan 015: Serialize Discogs device-auth transitions

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plans 003 and 012 are reviewed and marked `DONE` in
>    `plans/README.md`.
> 2. Create a base containing both reviewed dependency results, including Plan
>    002 transitively through Plan 003; do not start directly from the planning
>    commit.
> 3. Run:
>
> ```bash
> git diff --stat e6eb382..HEAD -- \
>   src/tools/discogs_auth.rs src/tools/mod.rs src/tools/tests.rs \
>   src/tools/enrich_handlers.rs
> git diff e6eb382..HEAD -- \
>   src/tools/discogs_auth.rs src/tools/mod.rs src/tools/tests.rs \
>   src/tools/enrich_handlers.rs
> ```
>
> Plan 003 is expected to add canonical audio-mutation state in
> `src/tools/mod.rs` and related tests. Plan 012 is expected to change
> `src/tools/enrich_handlers.rs` and `src/tools/tests.rs`. Reconcile both
> reviewed results, preserving Plan 003's keyed locks and Plan 012's
> acknowledged-cache-write contract. Changes outside those dependencies, an
> already-added device-auth transition lock, or a materially different auth
> state machine are STOP conditions.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/003-serialize-audio-file-mutations.md`, `plans/012-acknowledge-enrichment-cache-writes.md`
- **Category**: concurrency bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Concurrent Discogs lookups can all observe an empty or authorized pending
device session before any one caller records the transition. They may start
multiple browser-auth sessions, overwrite the pending session visible to the
user, finalize the same pending token more than once, or clear a newly stored
session after another caller succeeds. Batch enrichment makes this reachable
without unusual client behavior. Device-auth transitions need one async
critical section, while ordinary searches with an already-valid session
should remain outside it.

## Current state

- `ServerState` stores the pending session behind a synchronous mutex, but has
  no lock spanning the network and store operations that form one transition.
- The internal SQLite store is allowed to persist broker-session state.
  Rekordbox `master.db` remains read-only.
- Production session persistence also uses the macOS Keychain; the non-macOS
  adapter intentionally returns an error, so auth-transition tests cannot use
  the production persistence path on Linux CI.
- Plan 012 changes enrichment cache acknowledgements, not provider auth. Its
  durable success/failure accounting must remain intact.

Current `src/tools/mod.rs:89-101` has only the short-lived pending-state lock:

```rust
pub(super) struct ServerState {
    pub(super) db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) internal_db: OnceLock<Result<Mutex<Connection>, String>>,
    pub(super) essentia_python: OnceLock<Option<String>>,
    pub(super) essentia_python_override: Mutex<Option<String>>,
    pub(super) essentia_setup_lock: tokio::sync::Mutex<()>,
    pub(super) discogs_pending: Mutex<Option<discogs::PendingDeviceSession>>,
```

Current `src/tools/discogs_auth.rs:58-75` clones pending state, releases that
mutex, and then awaits the broker status call:

```rust
let pending = {
    let lock = server
        .state
        .discogs_pending
        .lock()
        .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))?;
    lock.clone()
};

let status = if let Some(ref p) = pending {
    if p.expires_at > now {
        Some(
            discogs::device_session_status(&server.state.http, cfg, p)
                .await
```

Current `src/tools/discogs_auth.rs:158-174` starts remotely before it records
the new pending session, so two callers can both start:

```rust
let started = discogs::device_session_start(&server.state.http, cfg)
    .await
    .map_err(|e| discogs::LookupError::message(format!("Discogs broker start error: {e}")))?;
{
    let mut lock = server
        .state
        .discogs_pending
        .lock()
        .map_err(|_| discogs::LookupError::message("Discogs auth state lock poisoned"))?;
    *lock = Some(started.clone());
}
```

Current `src/tools/discogs_auth.rs:204-230` also clears a rejected persisted
session without checking whether another caller has already replaced it:

```rust
SessionState::Valid(token) => {
    match discogs::lookup_via_broker(
        &server.state.http,
        &cfg,
        &token,
        artist,
        title,
        album,
    )
    .await
    {
        Ok(result) => return Ok(result),
        Err(discogs::LookupError::AuthRequired(_)) => {
            // Session rejected by broker — clear it and fall through
            let store = server.cache_store_conn().map_err(|e| {
                discogs::LookupError::message(format!("Internal store error: {e}"))
            })?;
            store::clear_broker_discogs_session(&store, &cfg.base_url).map_err(
```

## Target state and invariants

Implement these exact semantics:

1. At most one task at a time may inspect and advance persisted/pending
   Discogs authentication state for a server.
2. The lock covers status, start, finalize, pending replacement/clear, and
   persisted-session replacement/clear as one transition.
3. A caller waiting for the lock re-reads both persisted and pending state;
   it never acts on a snapshot captured before waiting.
4. A normal Discogs search using a valid token runs after the transition lock
   is released. Do not serialize all search traffic behind device auth.
5. Rejection of token `T` may clear the store only if the currently persisted
   token is still `T`; it must not clear a newer token.
6. Two concurrent unauthenticated calls start one device session and return
   remediation for that same session. Two concurrent calls observing an
   authorized pending session finalize it once.
7. No synchronous mutex guard or SQLite connection guard is held across an
   `.await`.
8. Auth errors and remediation must not include pending tokens, session tokens,
   broker credentials, or response headers.

## Commands you will need

- Focused auth tests: `cargo test -p reklawdbox discogs_auth -- --nocapture` —
  transition/concurrency tests pass.
- Enrichment regressions: `cargo test -p reklawdbox enrich_tracks` — Plan 012
  accounting remains correct.
- Format: `cargo fmt --check` — no diff.
- Docs/config format: `dprint check` — exits 0.
- Lint: `cargo clippy -p reklawdbox --all-targets -- -D warnings` — no warnings.
- Full crate tests: `cargo test -p reklawdbox --no-fail-fast` — all tests pass.
- Release build: `cargo build --release` — exits 0.
- Version smoke: `./target/release/reklawdbox --version` — version prints.
- Help smoke: `./target/release/reklawdbox --help` — help prints.

## Scope

**In scope** (the only source files you may modify):

- `src/tools/mod.rs`
- `src/tools/discogs_auth.rs`
- `src/tools/tests.rs`
- `plans/README.md` for the status row only

**Out of scope**:

- Plan 003's keyed audio-mutation registry; preserve it in `ServerState`.
- `src/tools/enrich_handlers.rs`; preserve the reviewed Plan 012 result.
- Discogs matching, rate limiting, broker endpoint schemas, or broker worker
  code.
- Adding a process-global lock, a cross-process lock, or a new dependency.
- Serializing ordinary searches once a valid session token has been resolved.
- Changing internal-store schema or exposing broker-session values.
- Any direct write to Rekordbox `master.db` or bypass of `ChangeManager`/XML
  export for user-visible metadata.

## Git workflow

- Base: a reviewed composite containing DONE Plans 003 and 012
- Branch: `codex/015-serialize-discogs-device-auth`
- Use Conventional Commits; preferred final message:
  `fix(discogs): serialize device auth transitions`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Add deterministic concurrent regressions

Before changing production behavior, add a test-only broker fixture using
`tokio::net::TcpListener` and the existing `reqwest::Client`; do not add a mock
server dependency. It should bind to `127.0.0.1:0`, return minimal valid JSON,
and atomically count requests by endpoint. Never place real credentials in
fixtures or failure output.

Expose/refactor a private `lookup_discogs_with_config` entry point that accepts
an explicit `BrokerConfig`, a controllable `now`, and a private synchronous
session-persistence interface. Define that interface in `discogs_auth.rs` with
load, store, and clear operations. Its production adapter delegates to the
existing `store::{get,set,clear}_broker_discogs_session` functions; its
test-only in-memory adapter stores rows behind a mutex and supports injected
write/clear errors. Do not change `src/store.rs`, bypass Keychain in production,
or select the fake through an environment variable.

To exercise the real `enrich_tracks` call path, add a per-`ServerState`,
`#[cfg(test)]` dependency override containing an explicit `BrokerConfig` and
`Arc` to the fake persistence adapter. `lookup_discogs_remote` must clone that
server-owned override before any await and otherwise construct the production
config/adapter. Do not use a static, process environment mutation, or a global
test hook: parallel server tests must be isolated. Both paths call the same
`lookup_discogs_with_config` resolver; the existing lookup-result override is
not sufficient because it bypasses auth transitions.

All finalize, persisted-token, conditional-clear, and persistence-failure race
tests must inject the in-memory adapter and run on every platform without
`#[cfg(target_os = "macos")]`. The production wrapper must be a thin testable
call into the same resolver with the real adapter. Cover:

1. two concurrent calls with no persisted or pending session: exactly one
   `/v1/device/session/start`, both receive `AuthRequired` remediation with the
   same public auth URL, and state contains that one pending device ID;
2. a pending session whose status is `authorized`: two concurrent calls make
   one finalize request, persist one session, and both continue with that
   token;
3. an expired pending session: concurrent callers clear it and start exactly
   one replacement;
4. a deliberately delayed first start/status/finalize response, proving the
   second task waits and re-reads committed state instead of using an early
   snapshot;
5. a rejected old persisted token racing a newly persisted token: conditional
   invalidation leaves the new token present.

Use barriers/oneshots and endpoint counters, not sleeps, to control ordering.
Assert stable state/counts, not request arrival timing.

Every concurrency scenario must be time-bounded. Wrap the complete scenario and each
intentional barrier/oneshot wait in `tokio::time::timeout` with a five-second test
deadline and a phase-specific failure message. The fixture must own a shutdown sender
and server `JoinHandle`: shut down and await it on success, and implement a drop guard
that aborts the task on early assertion failure/panic. No test may leave a listener,
blocked response task, or client future alive after completion.

**Verify**: `cargo test -p reklawdbox discogs_auth_concurrent -- --nocapture`
must fail against the current implementation by observing duplicate start or
finalize behavior; pure `resolve_*` tests remain green.

### Step 2: Add one server-owned async auth-transition lock

Add `discogs_auth_lock: tokio::sync::Mutex<()>` to `ServerState` and initialize
it in every production and test constructor next to `discogs_pending`. Keep
`discogs_pending`'s synchronous mutex for short clone/set operations only.
Initialize the cfg-test dependency override in the same constructors and add a
small test-only setter that replaces dependencies for one server instance.

Do not use a static/global mutex: separate server instances and unrelated test
fixtures must not interfere. Do not replace it with a read/write lock; every
auth-state inspection can lead to mutation after network I/O.

**Verify**:

```bash
rg -n 'discogs_auth_lock' src/tools/mod.rs src/tools/tests.rs
```

Expected: one field and all constructor initializers; no `static` declaration.
Then run `cargo check -p reklawdbox` and expect exit 0.

### Step 3: Collapse auth advancement into one locked resolver

Replace the split `fetch_pending_state`/`dispatch_pending`/`start_new_session`
flow with one private async resolver returning a small internal outcome such
as `ReadyToken(String)` or `AuthRequired(remediation)`. The resolver must:

1. acquire `discogs_auth_lock`;
2. read persisted state through the injected persistence interface only after
   acquiring it;
3. return a valid persisted token immediately;
4. clear an expired persisted session;
5. clone pending state under its synchronous mutex and release that guard;
6. for a non-expired pending session, await status while retaining only the
   async transition guard;
7. on `pending`, return its remediation; on `authorized`/`finalized`, finalize,
   persist the returned session through the same injected adapter, clear pending,
   and return the token;
8. on expired/unknown pending state, clear it before starting exactly one new
   device session; record the new pending session before returning remediation.

The async lock deliberately spans broker network I/O for an auth transition.
It must not span a normal `lookup_via_broker` search. Open and use the
synchronous internal-store guard in small blocks between awaits.

If finalize succeeds remotely but persistence fails, keep the pending session
so a later call can retry/reconcile; do not clear it before durable persistence.
If start succeeds remotely but recording the pending value fails because the
mutex is poisoned, return an error without leaking its private token.

**Verify**: `cargo test -p reklawdbox discogs_auth_transition` → start,
waiting, authorized/finalized, expiry, persistence-error, and retry cases pass.

### Step 4: Make rejected-token invalidation compare-and-clear

Keep ordinary broker lookup outside the auth-transition lock. If it returns
`LookupError::AuthRequired`, acquire the same transition lock and re-read the
persisted row. Clear it only when its token exactly equals the rejected token.
Release store guards before any await, then re-enter the locked resolver (or a
lock-assuming private core) without recursively acquiring the mutex.

Bound recovery to one retry:

- if resolution returns remediation, return `AuthRequired`;
- if another caller installed a different ready token, retry the search once
  with that token;
- if that retry is also rejected, conditionally invalidate it and return
  remediation/error rather than looping.

Structure lock-owning and lock-assuming helpers so the code cannot self-deadlock.
Do not compare or log tokens in assertion/error text.

**Verify**: `cargo test -p reklawdbox discogs_auth_rejected_token` → an old
token cannot clear a new one, and recovery performs at most two search calls.

### Step 5: Verify batch enrichment and cancellation behavior

Add an integration-style test through the existing concurrent enrichment path
with two Discogs misses and the local broker fixture. It must observe one
device-session start and structured auth failures/remediation for both tracks;
Plan 012's acknowledged cache writer must terminate normally with no false
enriched/skipped count.

Install the explicit config and in-memory persistence through the per-server
test override before invoking the handler. The test must therefore exercise
`enrich_handlers.rs -> lookup_discogs_remote -> locked resolver` on Linux and
macOS without changing process environment or touching Keychain.

Abort one task while it waits for the auth lock, then let the owner finish.
The waiting cancellation must not poison or retain the Tokio mutex, and a
subsequent lookup must use the committed pending/persisted state without a
duplicate start. Do not abort the owner in the middle of remote finalize: this
plan does not introduce a distributed transaction with the broker.

**Verify**:

```bash
cargo test -p reklawdbox discogs_auth -- --nocapture
cargo test -p reklawdbox enrich_tracks
```

Expected: all auth concurrency tests pass, Plan 012's cache-write accounting
tests remain green, and endpoint counters prove single start/finalize.

### Step 6: Run the repository gate and review the diff

Run every command in the command table, then inspect:

```bash
git diff --check
git diff -- src/tools/mod.rs src/tools/discogs_auth.rs src/tools/tests.rs
git status --short
```

Expected: only the three allowed source files, this plan, and the permitted
README status row are changed; no broker token, pending token, session token,
credential header, or captured response body appears in test snapshots/logs.

## Test plan

- Unit: existing pure persisted/pending state classification boundaries.
- Persistence seam: cross-platform in-memory load/store/clear, injected errors,
  and proof that the production wrapper selects the real store/Keychain adapter.
- Concurrency: single start, single finalize, expired replacement, delayed
  transition re-read, cancelled waiter, and conditional invalidation race.
- Integration: concurrent Discogs batch lookup through the server and Plan 012
  cache-write accounting.
- Regression: full `reklawdbox` test suite, clippy, Rust format, and dprint.
- Security: inspect failure strings/fixtures to confirm auth material is never
  emitted.

## Machine-checkable done criteria

- [ ] Plans 003 and 012 are reviewed `DONE`; their mutation-lock and cache
      acknowledgement regressions pass.
- [ ] `ServerState` owns exactly one non-global `discogs_auth_lock` initialized
      by every constructor.
- [ ] Two concurrent unauthenticated resolutions produce exactly one start.
- [ ] Two concurrent authorized resolutions produce exactly one finalize.
- [ ] Waiters re-read state after acquiring the lock.
- [ ] Every local-broker race is protected by bounded Tokio timeouts, and the
      fixture deterministically shuts down or aborts its server task.
- [ ] Normal valid-token searches execute outside the auth-transition lock.
- [ ] Rejection of an old token cannot clear a newer persisted token.
- [ ] Finalize/persist/conditional-clear concurrency tests run on non-macOS CI
      through the in-memory persistence adapter; no auth correctness test is
      hidden behind a macOS cfg.
- [ ] The batch enrichment race uses a per-server dependency override and
      reaches the same locked resolver as production without globals, env
      mutation, or Keychain.
- [ ] No synchronous mutex/store guard crosses an `.await`.
- [ ] Auth concurrency, enrichment regression, full tests, clippy, fmt, and
      dprint all exit 0.
- [ ] `git diff --check` is clean and the final diff stays within scope.

## STOP conditions

Stop and report if:

- Plan 003 or Plan 012 is not reviewed `DONE`, or their changes cannot be
  reconciled without changing the keyed-lock or cache-acknowledgement
  contracts.
- The broker's start/status/finalize contract differs from the current code or
  cannot support idempotent re-poll/finalize as assumed.
- Correctness would require holding a synchronous mutex or SQLite guard across
  an `.await`.
- A deterministic race test requires real network credentials, a real browser
  authorization, or process-global environment mutation.
- Cross-platform tests would require weakening or bypassing production Keychain
  behavior rather than injecting the private persistence interface.
- Existing unrelated changes touch the auth state machine or server
  constructors and ownership is unclear.
- The implementation would expose auth material, serialize all Discogs
  searches, change broker APIs, or write Rekordbox `master.db`.

## Maintenance notes

- Any new Discogs auth transition must go through the same resolver/lock; do
  not add a second read-then-await-then-write path.
- Keep auth-transition tests deterministic with barriers and a local fixture;
  endpoint-count assertions are the regression oracle.
- Keep the fixture's timeout and shutdown guard when adding race cases so a
  failed endpoint-count assumption cannot hang the crate test process.
- If broker sessions become account-scoped, replace the single server lock
  with keyed locks only as a separate reviewed change with equivalent tests.
- Keep cache-writer durability semantics from Plan 012 independent of provider
  auth: an auth failure must never be counted as a persisted match/no-match.
- Preserve Plan 003's canonical-path audio mutation registry when adding the
  auth-transition lock to `ServerState`.
- Internal cache/session writes remain permitted; Rekordbox `master.db` remains
  read-only, and user-visible metadata still flows through `ChangeManager` and
  XML export.
