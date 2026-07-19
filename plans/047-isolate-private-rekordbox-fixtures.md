# Plan 047: Isolate private Rekordbox and audio fixtures

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report rather than broadening the design.
> Update this plan's row in `plans/README.md` only after independent review and
> all mandatory gates pass.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/main.rs \
>   src/adapters/audio/tests.rs \
>   src/adapters/rekordbox/connection.rs \
>   src/adapters/rekordbox/mod.rs \
>   src/adapters/rekordbox/tests.rs \
>   src/adapters/rekordbox/xml.rs \
>   src/mcp/tests/common.rs \
>   src/mcp/tests/analysis.rs \
>   src/mcp/tests/classification.rs \
>   src/mcp/tests/enrichment
> ```
>
> Reconcile test-only drift. STOP if production Rekordbox opening is no longer
> SQLCipher read-only, if another reviewed change already owns private fixture
> lifetime, or if a current private test intentionally mutates the extracted
> database rather than a disposable copy.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: test architecture / private-data safety / navigation
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

Private integration coverage is useful here, but the current test helper hides
too much mutable global state:

- `src/adapters/rekordbox/connection.rs:47-89` extracts a dated backup into the
  fixed `/tmp/reklawdbox-test` directory behind `OnceLock<bool>`;
- it reuses any pre-existing `master.db` without proving that it came from the
  requested archive;
- it opens the extracted database with `Connection::open`, not the production
  read-only SQLCipher path;
- its fixed directory survives the test process and can collide across
  branches, users, or concurrent worktrees; and
- private callers repeat track/path selection and disagree about whether a
  missing fixture should fail or silently skip.

The 1,593-line `src/adapters/rekordbox/tests.rs` also combines connection,
track, playlist, history, health, rating/date, and private-library tests around
one large seed function. The size is a symptom; the real problem is that
portable tests and opt-in private tests have no explicit ownership boundary.

This plan creates that boundary. It does not add private data to the repository
or make private tests mandatory.

## Target design

### One RAII-owned private fixture

Add a test-only `PrivateRekordboxFixture` in explicit test support. It owns:

- a unique `tempfile::TempDir`;
- the extracted `master.db` and any requested WAL/SHM siblings;
- the source archive identity used for diagnostics; and
- methods that open the extracted database through the production
  `adapters::rekordbox::open` read-only SQLCipher path.

The fixture must remain alive for as long as any connection or server uses its
database. Callers that move a connection into an MCP server must retain the
fixture guard in the returned test tuple. Do not unlink an open database and
rely on Unix behavior.

Use `REKORDBOX_TEST_BACKUP` as the explicit opt-in source. Remove the hard-coded
dated archive fallback. Missing configuration is a distinct `Unconfigured`
outcome; an unreadable, malformed, or wrong-key archive is a typed fixture
failure. Targeted private-gate commands must treat `Unconfigured` as a clear
failure or print a clear skip according to the test's documented mode—never
silently reuse old `/tmp` state.

### Exact, injectable extraction

Keep archive extraction test-only. A small injected extractor seam must let
mandatory tests populate a fixture without private data or a shell tarball.
The system extractor must:

- pass arguments directly to `tar` (no shell interpolation);
- request only `master.db`, `master.db-wal`, and `master.db-shm`;
- tolerate absent WAL/SHM siblings but require `master.db`;
- write only beneath the fixture's unique temporary directory;
- bound execution with a short timeout and include bounded diagnostics; and
- leave cleanup to `TempDir` ownership.

Do not introduce a repository-wide subprocess abstraction. This is a
test-support operation with a narrow purpose.

### Safe private audio copies

Add a test helper that resolves one accessible audio path from a read-only
private fixture and, for mutation tests, copies it into a caller-owned
`TempDir`. The helper returns both the original and copied hashes so tests can
prove the original was not changed. No audio-tag or cover-art test may write to
the path returned from Rekordbox directly.

### Capability-oriented tests

Convert `src/adapters/rekordbox/tests.rs` into a test directory with a
navigation-only `mod.rs` and cohesive modules:

- `support.rs` — portable schema/row builders only;
- `connection.rs` — path selection, SQLCipher, and read-only invariants;
- `tracks.rs` — search, dates, ratings, and track lookup;
- `playlists.rs` — playlist pagination and membership;
- `history.rs` — sessions and play statistics;
- `health.rs` — health and duplicate queries; and
- `private.rs` — all ignored real-library checks.

Do not create a mega fixture builder with dozens of optional knobs. Keep the
existing canonical seed plus small capability-local additions.

## Scope

**In scope**:

- `src/main.rs` test call site only
- `src/adapters/audio/tests.rs` private call sites only
- `src/adapters/rekordbox/connection.rs` test-only helper removal
- `src/adapters/rekordbox/mod.rs`
- `src/adapters/rekordbox/tests.rs` (replaced by the directory below)
- `src/adapters/rekordbox/tests/mod.rs` (new; declarations only)
- `src/adapters/rekordbox/tests/support.rs` (new)
- `src/adapters/rekordbox/tests/connection.rs` (new)
- `src/adapters/rekordbox/tests/tracks.rs` (new)
- `src/adapters/rekordbox/tests/playlists.rs` (new)
- `src/adapters/rekordbox/tests/history.rs` (new)
- `src/adapters/rekordbox/tests/health.rs` (new)
- `src/adapters/rekordbox/tests/private.rs` (new)
- `src/adapters/rekordbox/test_support.rs` (new; `#[cfg(test)]` only)
- `src/adapters/rekordbox/xml.rs` private tests only
- `src/mcp/tests/common.rs`
- private-fixture call sites in `src/mcp/tests/analysis.rs`,
  `src/mcp/tests/classification.rs`, and
  `src/mcp/tests/enrichment/resolve.rs`
- `CONTRIBUTING.md` only for the exact opt-in fixture command and safety rule
- `plans/README.md` status row only during execution

**Out of scope**:

- Production Rekordbox path resolution, SQLCipher keying, queries, schemas, or
  serialization.
- Any SQL mutation path for the real or extracted `master.db`.
- Committing an archive, database, audio file, path, track ID, checksum, cache,
  credential, or generated test output.
- Making ignored/private tests part of `cargo test --workspace`.
- Genre taxonomy, classifier thresholds, DSP expectations, or provider calls.
- A general-purpose fixture framework shared outside Rekordbox/audio tests.

## Steps

### Step 1: Characterize the current portable and private contracts

Before changing helpers, record:

```bash
cargo test -p reklawdbox adapters::rekordbox::tests -- --nocapture
cargo test -p reklawdbox sanitized_sqlcipher_fixture_opens_read_only -- --nocapture
cargo test -p reklawdbox --test source_boundaries
```

Add mandatory tests around an injected fixture extractor that prove:

1. two fixtures use different roots and database paths;
2. the opened SQLCipher connection rejects an `INSERT`;
3. corrupt/wrong-key content is reported as a fixture error;
4. missing `master.db` is rejected even when sidecar files exist;
5. fixture cleanup removes its root after every connection is dropped; and
6. a fixture configured for archive A cannot reuse archive B's extracted DB.

These tests must be green without `REKORDBOX_TEST_BACKUP`.

### Step 2: Introduce the owned private fixture

Move `open_real_db` behavior out of production `connection.rs`. Implement the
fixture and exact extraction seam in `test_support.rs`. Use a focused error enum
for at least configuration, extraction, missing database, and SQLCipher open
failures. Its `Display` must not print home-directory paths unless the test is
already running locally with `--nocapture`; never print data from the database.

Open the extracted database by calling the production read-only `open` helper.
Do not duplicate `PRAGMA key`, `busy_timeout`, or validation SQL in test
support.

### Step 3: Migrate every private caller with explicit lifetime

Replace all `open_real_db()` call sites. MCP helpers must return the fixture
guard alongside the server and temporary writable store. Audio tests must keep
the fixture alive while selecting tracks and must use a temporary audio copy
for any file write.

After migration this check must be empty:

```bash
! rg -n "open_real_db|/tmp/reklawdbox-test|db_20260215_233936" src
```

Do not hide the guard in a leaked allocation, global `OnceLock`, or persistent
`target/` directory.

### Step 4: Split adapter tests by capability

Move tests without changing their assertions. Keep portable helpers in
`support.rs`; keep private-only helpers in `private.rs` or `test_support.rs`.
Name private tests with a shared `private_rekordbox_` prefix so the opt-in gate
selects only intended tests. Keep `mod.rs` to declarations and narrowly scoped
test re-exports.

Run each capability filter independently to prove navigation works:

```bash
cargo test -p reklawdbox rekordbox_connection -- --nocapture
cargo test -p reklawdbox rekordbox_tracks -- --nocapture
cargo test -p reklawdbox rekordbox_playlists -- --nocapture
cargo test -p reklawdbox rekordbox_history -- --nocapture
cargo test -p reklawdbox rekordbox_health -- --nocapture
```

If renaming an existing test is needed for filters, preserve its assertion and
add the capability prefix; do not weaken it.

### Step 5: Run the opt-in private gate when configured

If a safe archive and accessible audio are available, run:

```bash
REKORDBOX_TEST_BACKUP=/absolute/path/to/backup.tar.gz \
  cargo test -p reklawdbox private_rekordbox_ -- --ignored --nocapture
```

The private gate must prove:

- SQLCipher opens through the read-only production adapter;
- representative queries, playlists, history, and Unicode rows still work;
- at least one accessible track can be resolved when audio-specific checks are
  selected;
- any mutation round trip happens only on a temporary copy; and
- the source archive, source audio hash, real `master.db`, and local cache are
  unchanged.

If private data is unavailable, record this gate as `NOT RUN`; mandatory
synthetic tests still decide merge readiness. Never invent or commit a fixture.

### Step 6: Full verification and diff review

Run:

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
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
git diff --check
```

Also inspect the complete diff and confirm no private paths or fixture output
are staged.

Require an independent architecture/test-quality review to trace fixture
lifetime through every connection/server caller, verify production read-only
open reuse, compare moved assertions, and search the complete diff for private
paths or data. Remediate concrete findings and rerun the focused plus full
gates before marking `DONE`.

## Machine-checkable done criteria

- [ ] No fixed `/tmp/reklawdbox-test`, dated default archive, or global
      extraction boolean remains.
- [ ] Every private fixture has an explicit owner and unique temporary root.
- [ ] Extracted databases open only through the production SQLCipher read-only
      adapter, with a regression proving SQL writes fail.
- [ ] Audio mutation tests write only to temporary copies and prove original
      hashes are unchanged.
- [ ] Portable Rekordbox tests are split by capability with a navigation-only
      `mod.rs` and no assertion loss.
- [ ] Missing, corrupt, and cross-archive fixture cases are mandatory synthetic
      tests.
- [ ] Private tests remain ignored/opt-in and no private data or path is
      committed.
- [ ] Architecture, workspace, release, MCP, docs-contract, site, and diff
      gates pass.

## STOP conditions

Stop and report if:

- a private test requires opening the user's live `master.db` writable;
- an audio test cannot be made safe without mutating the source track;
- fixture lifetime cannot be kept explicit without leaking or persisting a
  temporary directory;
- archive extraction would require shell interpolation or broad extraction;
- a portable regression depends on private library contents;
- the SQLCipher archive format differs such that a cache/schema/product change
  would be needed; or
- moving the tests would require changing production query behavior.

## Complexity accounting

This plan removes hidden global extraction state and duplicate private-fixture
selection. It localizes unavoidable private-data setup in test support and
makes portable/private ownership visible. Splitting the test file alone is not
success: the fixed directory, writable open, repeated path selection, and
implicit lifetime must all be gone.

## Git workflow

- Branch: `codex/047-isolate-private-fixtures`
- Preferred commit: `test(rekordbox): isolate private fixtures`
- Do not push, merge, release, deploy, or remove private files.
