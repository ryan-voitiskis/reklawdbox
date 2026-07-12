# Plan 011: Make incremental audit freshness complete

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless an orchestrator/reviewer owns the index.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plan 010 is reviewed and marked `DONE` in `plans/README.md`.
> 2. Start from that reviewed result, then run:
>    ```bash
>    git diff --stat e6eb382..HEAD -- src/audit.rs src/store.rs site/src/partials/sops/collection-audit.mdx site/src/content/docs/mcp-tools/files-system.mdx
>    ```
>
> Plan 010 is expected to change store schema/migration and timbral-statistics
> code in `src/store.rs`; preserve that reviewed result. Compare the current
> scan/freshness excerpts below with live code. A new audit schema, a new
> freshness token, or changed `FileReadResult::Error` handling outside Plan
> 010 is a STOP condition. Any pre-existing change to the collection-audit SOP
> or audit-state MCP reference is unrelated drift and must also be reviewed
> before proceeding.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/010-version-timbral-normalization-stats.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Incremental collection audits currently treat a failed tag read as a completed
scan, reduce file modification time to whole seconds, and ignore directory
context when deciding whether to rescan an unchanged file. A transient read
failure can therefore be cached forever, same-size edits in one second can be
missed, and adding/removing a numbered sibling can change AlbumTrack versus
LooseTrack rules without rechecking existing files. The freshness identity
must cover the exact successful inputs, and unsuccessful reads must remain
retryable without deleting existing issues.

## Current state

- `src/audit.rs` walks files, derives album/loose context, reads tags, applies
  checks, and persists scan state/issues.
- `src/store.rs` persists `audit_files` and child `audit_issues` in the writable
  internal SQLite store. This plan must not touch Rekordbox `master.db`.
- `audit_files.file_mtime` is a `TEXT` column. Retain that physical column for
  compatibility, but treat it as an opaque, versioned freshness key in Rust.
  Existing ISO values should cause a safe one-time rescan.

Current `src/audit.rs:1038-1047` discards subsecond precision:

```rust
fn file_mtime_iso(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .and_then(|t| {
            let duration = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            let dt = chrono::DateTime::from_timestamp(duration.as_secs() as i64, 0)?;
            Some(dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_default()
}
```

Current `src/audit.rs:1132-1166` computes directory context separately from
the freshness decision:

```rust
let album_dirs = detect_album_dirs(&disk_files);
// ...
let mtime = file_mtime_iso(&metadata);
let size = metadata.len() as i64;

let needs_scan = match existing_file {
    None => true,
    Some(ef) => {
        if revalidate { true } else {
            ef.file_mtime != mtime || ef.file_size != size
        }
    }
};
```

Current `src/audit.rs:1171-1188` suppresses checks on read error, but current
`src/audit.rs:1213-1252` still writes a fresh row and increments `scanned`:

```rust
let read_result = tags::read_file_tags(file_path, None, false);
let context = classify_track_context(file_path, &album_dirs);

let mut detected: Vec<DetectedIssue> = Vec::new();
if !matches!(read_result, FileReadResult::Error { .. }) {
    detected.extend(check_tags(...));
    detected.extend(check_filename(...));
}

store::upsert_audit_file(&tx, &path_str, &now, &mtime, size)?;
// ...
scanned += 1;
```

`tags::read_file_tags` returns the source error as data
(`src/tags.rs:531-542`):

```rust
Err(e) => {
    return FileReadResult::Error {
        path: path_str,
        error: e,
    };
}
```

Current `src/store.rs:945-983` calls the persisted value `file_mtime` and
blindly upserts it. Rename the Rust-level field/argument to `freshness_key`,
while keeping the existing SQL column name to avoid a destructive table/FK
migration:

```rust
pub struct AuditFile {
    pub path: String,
    pub last_audited: String,
    pub file_mtime: String,
    pub file_size: i64,
}
```

Applicable conventions:

- A scan transaction batches 500 paths. Failed reads must still participate in
  batch accounting so a directory of failures does not create one unbounded
  transaction.
- Existing issues are auto-resolved only after a successful reread. Preserve
  that safety property and never delete/resolve issues based on an error.
- Filesystem-walk errors already produce warnings and suppress missing-file
  cleanup; follow that transparent partial-success style.
- `ScanSummary` is serialized directly. Additive counters are compatible; do
  not rename existing response fields.

## Commands you will need

| Purpose            | Command                                                   | Expected on success          |
| ------------------ | --------------------------------------------------------- | ---------------------------- |
| Audit tests        | `cargo test -p reklawdbox audit::tests`                   | exit 0; all audit tests pass |
| Store audit tests  | `cargo test -p reklawdbox store::tests::test_audit`       | exit 0; matching tests pass  |
| Format             | `cargo fmt --check`                                       | exit 0, no diff              |
| Docs/config format | `dprint check`                                            | exit 0                       |
| Lint               | `cargo clippy -p reklawdbox --all-targets -- -D warnings` | exit 0, no warnings          |
| Full crate tests   | `cargo test -p reklawdbox --no-fail-fast`                 | exit 0; all tests pass       |
| Docs build         | `(cd site && npm ci && npm run build)`                    | exit 0; docs build passes    |

## Scope

**In scope** (the only source/docs files you may modify):

- `src/audit.rs`
- `src/store.rs`
- `site/src/partials/sops/collection-audit.mdx`
- `site/src/content/docs/mcp-tools/files-system.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Timbral-normalization identity or statistics delivered by Plan 010.
- Adding/removing audit issue types or changing their safety tiers.
- Changing filename/tag rules, album-directory thresholds, or issue details.
- Following directory symlinks or changing filesystem traversal policy.
- Rebuilding/renaming `audit_files` or changing the `audit_issues` foreign key;
  retain the physical `file_mtime` column as a compatibility container.
- Changing generic audio-analysis cache identity; this plan is audit-specific.
- Direct writes to Rekordbox `master.db` or user-visible metadata.

## Git workflow

- Base: reviewed DONE commit from Plan 010
- Branch: `codex/011-make-audit-freshness-complete`
- Use Conventional Commits; preferred final message:
  `fix(audit): make freshness checks complete`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Characterize the three freshness failures

Add synthetic tests under `src/audit.rs` before changing production logic.
Create a small test helper that writes the same minimal valid PCM WAV structure
used by `src/tags.rs:1362-1388`; do not use private audio fixtures.

Add regressions for:

1. An invalid `.flac` produces a tag-read error on two consecutive scans;
   neither scan counts it as successfully `scanned`, and the second scan does
   not count it as `skipped_unchanged`.
2. A pre-existing open issue remains open when a forced revalidation read
   fails.
3. A pure freshness-key test distinguishes two mtimes in the same second with
   different nanoseconds.
4. Scan one valid numbered WAV in a directory (LooseTrack context), then add a
   second numbered WAV so `detect_album_dirs` changes the first file to
   AlbumTrack. A non-revalidate scan must rescan the unchanged first file.

The current code should fail these assertions for the intended reasons.

**Verify**: `cargo test -p reklawdbox audit::tests -- --nocapture` → the new
regressions fail before the production change; record which assertions fail.

### Step 2: Define one versioned successful freshness key

In `src/audit.rs`, replace `file_mtime_iso` with a small pure helper plus a
metadata adapter. A successful key must be stable and contain:

- an explicit format version, initially `v2`;
- the full `SystemTime::duration_since(UNIX_EPOCH).as_nanos()` value;
- the effective `AuditContext` (`album` or `loose`).

File size remains the separate `audit_files.file_size` field and therefore is
part of the complete comparison without duplication in the string. Suggested
shape: `v2:<mtime-nanos>:album` or `v2:<mtime-nanos>:loose`.

If modified time cannot be read or predates the epoch, return no successful
key and force a retry on every scan; do not use an empty string that could
compare equal across failures.

Use a distinct non-success domain for attempts that can read tags but cannot
form a successful metadata key: `retry:metadata:<attempt-nanos>`. This token is
persistable for the parent-row foreign key but can never compare equal to a
`v2:` key. It must not be treated as a tag-read failure.

Rename the Rust `AuditFile.file_mtime` field and `upsert_audit_file` argument to
`freshness_key`, including tests and SQL row mapping. Keep the SQL column name
`file_mtime` and add a comment explaining it stores the opaque versioned audit
identity for backwards compatibility. Do not bump `STORE_SCHEMA_VERSION`:
existing ISO rows naturally fail the `v2:` comparison and are replaced after
one successful scan.

**Verify**:

```bash
cargo test -p reklawdbox audit_freshness_key
cargo test -p reklawdbox store::tests::test_audit
```

Expected: both commands exit 0; nanosecond/context keys and store round-trips
pass; legacy arbitrary strings remain readable and simply do not equal a
current `v2` key.

### Step 3: Compare context before deciding to skip

Move `classify_track_context(file_path, &album_dirs)` before `needs_scan` and
build the expected successful freshness key from metadata plus that context.
An existing row is skippable only when all are true:

- `revalidate` is false;
- a current successful key exists;
- persisted key exactly equals it;
- persisted file size equals current size.

This ensures a sibling-set change that flips AlbumTrack/LooseTrack causes a
rescan even when the file bytes did not change. Do not rescan merely because an
unrelated directory elsewhere in the scope changed.

**Verify**: `cargo test -p reklawdbox album_context_change_reaudits_unchanged_file` →
the first WAV is scanned again after the second numbered sibling is added; an
immediately repeated third scan skips both unchanged files.

### Step 4: Persist read attempts as retryable, not fresh

Handle `FileReadResult::Error { error, .. }` before issue detection:

1. increment a new additive `ScanSummary.failed_reads` counter;
2. append a warning containing the path and read error;
3. upsert the `audit_files` parent with the exact deliberately non-success key
   `retry:read:<attempt-nanos>` so new files still have a parent row and every
   future scan retries;
4. do not insert issues, auto-resolve issues, or increment `scanned`;
5. retain existing child issues untouched;
6. include the failed path in normal `BATCH_SIZE` transaction accounting and
   commit behavior.

On a successful read, always run checks/reconcile issues and increment
`scanned`. Persist the current `v2` key when available. If metadata could not
produce that key, persist `retry:metadata:<attempt-nanos>`, append a warning
that the file was audited but will be retried, and leave `failed_reads`
unchanged. A later attempt with usable metadata replaces either retry token
with `v2:` and returns the row to normal incremental skipping.

Do not use `continue` in a way that bypasses the batch counter/commit block. A
per-file outcome enum or a common post-processing block is acceptable if it
makes this invariant clear.

**Verify**:

```bash
cargo test -p reklawdbox audit_read_failure_is_retried
cargo test -p reklawdbox audit_read_failure_preserves_existing_issues
```

Expected: both exit 0; consecutive scans report `failed_reads == 1`,
`scanned == 0`, and the existing issue remains unchanged.

### Step 5: Update store and summary regression coverage

Update `src/store.rs` audit tests to use `freshness_key` terminology and assert
that retry and successful keys round-trip. Keep the table/FK behavior
unchanged.

Update existing `ScanSummary` assertions as needed for the additive
`failed_reads` field. Add a success test showing a valid unchanged file is
skipped after one successful audit, so the fix does not turn all scans into
full rescans.

**Verify**:

```bash
cargo test -p reklawdbox store::tests::test_audit
cargo test -p reklawdbox audit::tests
```

Expected: both commands exit 0; all matching tests pass.

### Step 6: Make partial scans impossible to report as clean

Update `site/src/partials/sops/collection-audit.mdx` so every scan review and
verification checks `failed_reads` alongside `files_in_scope`, `new_issues`,
and `total_open`. The SOP may advance to a clean final report only when both
`total_open == 0` and `failed_reads == 0`. When failures are nonzero, it must
surface the warnings/paths, retry the scan, and report persistent unreadable
files as an incomplete audit rather than silently accepting them. Include
`failed_reads` in the final report.

Update the `audit_state` scan section in
`site/src/content/docs/mcp-tools/files-system.mdx` to document additive
`failed_reads`, its retryable meaning, and that `total_open == 0` is not a
complete/clean result while reads failed.

Run the doc-drift workflow in `docs/workflows/doc-drift/README.md`. This SOP is
embedded with `include_str!`, so a Rust rebuild is required before an MCP host
can observe it; do not treat the site build alone as deployment.

**Verify**:

```bash
dprint check
(cd site && npm ci && npm run build)
rg -n 'failed_reads|incomplete audit' \
  site/src/partials/sops/collection-audit.mdx \
  site/src/content/docs/mcp-tools/files-system.mdx
```

Expected: docs formatting/build pass, and the SOP cannot take the
`total_open == 0` clean path while reads failed.

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

Expected: all commands exit 0; the embedded SOP is rebuilt and no files
outside scope are changed.

## Test plan

- Pure freshness-key tests:
  - same second/different nanoseconds differ;
  - album versus loose context differs;
  - missing/invalid mtime cannot create a successful key;
  - legacy ISO values are not accepted as `v2`.
  - `retry:read:` and `retry:metadata:` are never successful/skippable domains.
- Scan tests with synthetic files:
  - invalid tag read retries forever until success and is counted separately;
  - successful tag read with unavailable mtime reconciles issues, increments
    `scanned`, warns, persists `retry:metadata:`, and remains non-skippable;
  - failed revalidation preserves open issues;
  - sibling addition changes context and reaudits unchanged files;
  - successful unchanged files still skip;
  - successful retry replaces the retry token and becomes skippable.
- Store tests:
  - successful and retry freshness strings round-trip;
  - upsert does not delete child issues.
- SOP/MCP docs: a read failure blocks a clean report; doc-drift and a
  clean-site install/build pass.

## Done criteria

- [ ] `rg "file_mtime_iso" src/audit.rs` returns no matches.
- [ ] Successful freshness includes nanosecond mtime and effective context;
      size is compared separately.
- [ ] Existing legacy ISO rows force exactly one successful rescan.
- [ ] Failed reads persist `retry:read:<attempt-nanos>` and successful reads
      without usable mtime persist `retry:metadata:<attempt-nanos>`; neither is
      ever skippable.
- [ ] A tag-read error increments `failed_reads`, not `scanned`, and is retried
      on the next non-revalidate scan.
- [ ] A successful read without a usable mtime increments `scanned`, not
      `failed_reads`; it reconciles issues, warns, and persists a
      `retry:metadata:` token that forces the next scan.
- [ ] A failed read never inserts/resolves/deletes audit issues.
- [ ] A context flip caused by siblings reaudits the affected unchanged file.
- [ ] An unchanged successfully audited file remains skippable.
- [ ] Failed paths still respect 500-row transaction batching.
- [ ] Plan 010's timbral-statistics identity and migration tests remain green.
- [ ] The collection-audit SOP reports `failed_reads`, retries them, and cannot
      declare a clean audit while any remain.
- [ ] The MCP file-system reference defines the new counter and incomplete
      result semantics.
- [ ] Targeted tests, format, dprint, clippy, and full crate tests exit 0.
- [ ] `git diff --name-only` lists only `src/audit.rs`, `src/store.rs`, the
      collection-audit SOP, the MCP file-system reference, and optionally
      `plans/README.md`.
- [ ] Rekordbox `master.db` remains read-only.

## STOP conditions

Stop and report back if:

- Plan 010 is not reviewed `DONE`, or its store migration cannot be preserved
  without broadening this plan.
- Another change has already repurposed or migrated
  `audit_files.file_mtime`; do not layer a second encoding over it.
- The effective audit context is no longer only AlbumTrack/LooseTrack; expand
  the key deliberately and add one test per new context before proceeding.
- A valid synthetic WAV cannot be parsed by `read_file_tags`; fix the fixture
  based on the existing `tags.rs` helper, not by weakening assertions or using
  private audio.
- Preserving old issues on failure appears to require deleting/recreating the
  `audit_files` parent row. It must be an upsert/update that retains children.
- A proposed fix would globally rescan every file whenever any sibling anywhere
  in the scope changes; use the per-file effective context instead.
- Any step requires a direct Rekordbox write.
- Doc drift requires a public audit surface outside the declared scope; report
  it before expanding the change.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- Increment the freshness-key version when any persisted input to audit rules
  changes. Old versions should cause a one-time rescan, never be interpreted as
  current.
- If `AuditContext` gains a variant, add its stable token and context-change
  regression in the same change.
- Keep `retry:read:` and `retry:metadata:` accounting distinct: only the former
  is a failed tag read, while neither token is ever skippable.
- Reviewers should inspect error paths for accidental issue resolution and for
  `continue` statements that skip transaction accounting.
- Future store migrations must preserve both Plan 010's timbral-statistics
  provenance and this plan's versioned audit freshness token.
- `collection-audit.mdx` is embedded with `include_str!`; every SOP change
  requires a server rebuild/release before MCP hosts receive it.
- The physical `file_mtime` column name is legacy. A future schema-cleanup plan
  may rename it with a tested FK-preserving migration, but that is deliberately
  out of scope here.
