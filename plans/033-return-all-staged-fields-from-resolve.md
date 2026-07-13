# Plan 033: Return every staged editable field from resolve tools

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` only if the reviewer has not told you that they own the
> index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat fb5e210..HEAD -- src/tools/resolve_handlers.rs src/tools/tests.rs
> ```
>
> Start from reviewed portfolio commit `fb5e210`. The command must print no
> changes. If either in-scope file has changed, compare the live code with the
> excerpts below and STOP on any semantic mismatch before editing.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 031 (the current reviewed documentation portfolio base)
- **Category**: bug / public runtime contract
- **Planned at**: commit `fb5e210`, 2026-07-13

## Why this matters

`resolve_track_data` and full-format `resolve_tracks_data` promise the caller's
staged changes alongside Rekordbox and cached provider data. The shared response
builder currently drops staged `year` and `album`, even though both fields are
accepted by `update_tracks`, retained by `ChangeManager`, previewed, and written
to XML. An agent that resolves a track after staging either field receives an
incomplete view and can make a later decision from stale metadata.

This plan restores the existing public promise by serializing every field in
`TrackChange`. It does not add a new editable field, change staging semantics,
write to Rekordbox, or broaden the intentionally compact classification format.

## Current state

- `src/types.rs` defines seven editable staged fields on `TrackChange`:
  `genre`, `comments`, `rating`, `color`, `label`, `year`, and `album`.
- `src/tools/resolve_handlers.rs::handle_resolve_track_data` passes the staged
  `TrackChange` to `resolve_single_track`.
- `src/tools/resolve_handlers.rs::handle_resolve_tracks_data` does the same for
  `ResolveFormat::Full`. `ResolveFormat::Classification` deliberately calls
  `resolve_single_track_compact`, whose narrow decision-tree payload is not part
  of this fix.
- At `fb5e210`, the shared full-response builder contains only five fields:

  ```rust
  let staged_val = staged.map(|s| {
      serde_json::json!({
          "genre": s.genre,
          "comments": s.comments,
          "rating": s.rating,
          "color": s.color,
          "label": s.label,
      })
  });
  ```

  The missing entries are `"year": s.year` and `"album": s.album`.
- `src/tools/tests.rs::resolve_single_track_with_staged_changes` is the closest
  focused regression test. It currently stages only genre/rating and checks
  null semantics for other fields, but it does not prove that the year/album
  keys exist.
- Test helpers in `src/tools/tests.rs` already provide
  `create_single_track_test_db`, `create_server_with_connections`,
  `extract_json`, and direct access to `server.state.changes.stage(...)`.
- Preserve the repository's non-negotiable boundary: Rekordbox `master.db`
  remains read-only. This response-only fix must not add a database write path.

## Commands you will need

| Purpose                   | Command                                                                                         | Expected on success                                                                   |
| ------------------------- | ----------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| Focused helper regression | `cargo test -p reklawdbox resolve_single_track_with_staged_changes -- --nocapture`              | exit 0 after implementation; before implementation the new year/album assertions fail |
| Public handler regression | `cargo test -p reklawdbox resolve_tools_return_all_staged_fields_in_full_format -- --nocapture` | exit 0; single and batch full responses contain all seven staged fields               |
| Formatting                | `cargo fmt --check`                                                                             | exit 0                                                                                |
| Lint                      | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                       | exit 0                                                                                |
| Crate tests               | `cargo test -p reklawdbox --no-fail-fast`                                                       | exit 0                                                                                |
| Release build             | `cargo build --release`                                                                         | exit 0                                                                                |
| MCP smoke                 | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`     | exit 0 and zero protocol violations                                                   |

## Scope

**In scope — the only source files to modify**:

- `src/tools/resolve_handlers.rs`
- `src/tools/tests.rs`

**Reviewer-owned plan artifacts**:

- `plans/README.md`

**Out of scope**:

- `src/types.rs`, `src/changes.rs`, `src/xml.rs`, staging behavior, editable-field
  validation, and any Rekordbox database access.
- The compact `ResolveFormat::Classification` response shape.
- Provider, audio-cache, taxonomy, selection, pagination, and coverage logic.
- Documentation copy. Plan 032 owns the integrated documentation sweep once
  this runtime prerequisite is reviewed and integrated.
- Output-schema introduction, version bumps, CLI changes, release automation,
  dependency updates, pushing, or opening a pull request.

## Git workflow

- Branch: `codex/033-return-all-staged-fields-from-resolve`
- Commit: `fix(resolve): return all staged track fields`
- Do not push or open a pull request.
- The reviewer owns `plans/README.md`; do not edit the tracker in the executor
  worktree.

## Steps

### Step 1: Add red regressions for the complete staged surface

In `src/tools/tests.rs`, preserve
`resolve_single_track_with_staged_changes` as the existing partial-change/null
semantics test. Add assertions using `get(...)` (not JSON indexing) that prove
the unset `year` and `album` keys are present and explicitly JSON `null`. These
assertions must fail while the keys are absent.

Add a DB-free async regression named
`resolve_tools_return_all_staged_fields_in_full_format`, using the existing
single-track database and temporary-store helpers. Stage all seven fields in
`server.state.changes`, then prove:

1. `resolve_track_data` returns the exact seven-field object and its key set
   exactly matches the names derived from `EditableField::ALL`, preventing the
   resolver from drifting behind a future editable-field addition;
2. full `resolve_tracks_data` returns the same object for that track;
3. classification-format `resolve_tracks_data` retains its existing compact
   shape and does not acquire `staged_changes` in this plan.

Run both focused tests before changing the serializer. The new assertions must
fail only because `year` and `album` are missing/null. A setup, selection, or
unrelated response failure is a STOP condition.

**Verify**:

```bash
cargo test -p reklawdbox resolve_single_track_with_staged_changes -- --nocapture
cargo test -p reklawdbox resolve_tools_return_all_staged_fields_in_full_format -- --nocapture
```

Expected before Step 2: both commands compile; at least one assertion in each
test fails specifically on staged `year` or `album`. Record this red evidence.

### Step 2: Complete the shared full-response serializer

In `src/tools/resolve_handlers.rs::resolve_single_track`, add only these two
entries to the existing `staged_val` object:

```rust
"year": s.year,
"album": s.album,
```

Keep the existing five fields and their null serialization unchanged. Do not
alter either handler's selection path, `ChangeManager`, the compact response,
or Rekordbox metadata. Do not serialize `TrackChange` wholesale: that would
also expose its internal `track_id`, which is not part of the staged overlay.

**Verify**:

```bash
cargo test -p reklawdbox resolve_single_track_with_staged_changes -- --nocapture
cargo test -p reklawdbox resolve_tools_return_all_staged_fields_in_full_format -- --nocapture
```

Expected: both focused regressions exit 0. The single and batch full responses
are identical for `staged_changes`; the compact response remains compact.

### Step 3: Run the runtime gate and inspect scope

Run the formatting, lint, full crate-test, release-build, and DB-free MCP-smoke
commands from the table. Then inspect the complete diff against `fb5e210`.

**Verify**:

```bash
cargo fmt --check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
git diff --check
git status --short
```

Expected: every command exits 0; smoke reports zero protocol violations; only
the two in-scope source files are modified before the conventional commit.

## Test plan

- Preserve `resolve_single_track_with_staged_changes` as the direct
  partial-change/null regression, adding presence checks for null year/album.
- Add `resolve_tools_return_all_staged_fields_in_full_format` to exercise both
  public full-resolution paths, derive its exact key set from
  `EditableField::ALL`, and explicitly preserve compact classification behavior.
- Use representative non-null values for every field, including a valid year
  and album distinct from the Rekordbox source values.
- Preserve existing tests that prove `staged_changes` is null when nothing is
  staged and optional entries serialize as null when not staged.
- Run the full crate suite because both tools share the modified response
  builder with enrichment, audio-analysis, and taxonomy payloads.

## Done criteria

- [ ] The added assertions fail against `fb5e210` specifically because staged
      `year` and `album` are absent.
- [ ] Full `resolve_track_data` returns all seven `TrackChange` fields.
- [ ] Full `resolve_tracks_data` returns the same seven-field staged object.
- [ ] The complete staged key set is checked against `EditableField::ALL`.
- [ ] Unstaged optional fields continue to serialize as `null`.
- [ ] Classification-format resolution remains compact and unchanged.
- [ ] No Rekordbox write path, staging behavior, provider logic, or output
      schema is changed.
- [ ] Focused tests, formatting, clippy, full crate tests, release build, MCP
      smoke, and diff checks all pass.
- [ ] Exactly `src/tools/resolve_handlers.rs` and `src/tools/tests.rs` are in the
      source commit.

## STOP conditions

Stop and report rather than improvising if:

- the live full handlers no longer share `resolve_single_track`;
- `TrackChange` or `EditableField::ALL` no longer defines exactly the seven
  fields listed above;
- the compact classification response already promises staged changes through
  a live output schema or contract test;
- making the tests reach either public full handler requires private Rekordbox
  data, network access, or a production database;
- the fix requires changes outside the two in-scope source files;
- a focused test fails for anything other than the missing staged fields; or
- two reasonable attempts cannot make a verification command pass.

## Maintenance notes

The staged-response key-set assertion is the durable sentinel: any future
editable field added to `EditableField::ALL` must also be deliberately exposed
by full resolve responses or accompanied by an explicit contract decision.
Reviewers should reject fixes that instead remove `year`/`album` from staging,
teach compact classification to return the full payload, or weaken the test to
accept a subset.
