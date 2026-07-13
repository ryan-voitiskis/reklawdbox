# Plan 029: Make export backups fail closed and path-aware

> **Executor instructions**: Follow this plan in order, starting with red
> failure-path tests. Confirm every expected result. Stop on any STOP condition
> rather than weakening the backup promise. Update this plan's row in
> `plans/README.md` only if the orchestrator/reviewer does not own the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- src/backup.rs scripts/backup.sh src/tools/mod.rs src/tools/staging_handlers.rs src/tools/tests.rs site/src/data/workflows.mjs site/src/content/docs/reference/environment-variables.md site/src/content/docs/reference/xml-export.mdx site/src/content/docs/cli/index.mdx scripts/check-doc-contract.mjs scripts/check-doc-contract.test.mjs
> ```
>
> Plans 021, 024, and 027 must be integrated. Reconfirm the transactional
> `write_xml` guard from completed Plan 002 and all current server constructors.
> Any design that backs up a different database from the one the server opens,
> touches a real user library in tests, or weakens read-only DB access is a STOP
> condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: 021, 024, 027
- **Category**: bug / safety / docs
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

The XML reference promises every export is protected by a successful backup.
Today a configured custom backup script that does not exist is treated as a
successful skip, and the embedded shell script always reads the standard
Rekordbox directory even when the MCP server uses a custom
`REKORDBOX_DB_PATH`. `write_xml` can therefore proceed without a backup or with
an archive of the wrong library.

Export must resolve one effective `master.db` path, use it for both the
read-only connection and backup, and refuse XML output on every backup failure.

## Current state

- `src/backup.rs:7-20` defines `BackupStatus::CustomNotFound` as a non-error.
- `src/backup.rs:24-47` returns that status when
  `REKLAWDBOX_BACKUP_SCRIPT` points to a missing file.
- `scripts/backup.sh:15-20` hardcodes
  `$HOME/Library/Pioneer/rekordbox`.
- `src/tools/staging_handlers.rs:303-318` runs backup before export but passes
  no source DB path.
- `src/tools/mod.rs:103,121-135` holds an optional configured `db_path` but may
  independently resolve the environment/default only when the connection is
  first opened.
- `site/src/content/docs/reference/xml-export.mdx:100-108` says any backup
  failure blocks export.
- `src/tools/tests.rs:4629-4720` already tests that a nonzero custom backup
  script blocks XML and restores staged changes; preserve that transactional
  pattern.
- The following test currently expects a missing script to succeed and must be
  inverted.
- The public environment reference defines `REKORDBOX_DB_PATH` as the full path
  to `master.db`, including for nonstandard locations.

## Required safety contract

1. The server resolves an effective `master.db` path once and shares it between
   the read-only connection and pre-export backup.
2. For the embedded script, success means the archive was created from the
   effective path and moved into place. For a configured custom script, success
   means the script received `--pre-op` plus the effective path environment and
   exited zero; that exit is a trusted operator-script attestation because the
   server cannot prove what an arbitrary script archived.
3. Missing custom script, invalid DB path, script launch failure, nonzero exit,
   archive failure, or final move failure blocks XML creation.
4. The transactional staged-change guard restores changes after any failure.
5. The custom script keeps `--pre-op` as its first argument and receives the
   effective DB path through a child-only `REKORDBOX_DB_PATH` environment
   variable.
6. No path handling may expose credentials or change the
   `SQLITE_OPEN_READ_ONLY` database boundary.
7. Full archives retain the historical top-level `rekordbox/` root regardless
   of the configured target directory basename; restore maps that canonical
   archive root onto the effective configured directory.
8. A symlinked effective `master.db` path is rejected before canonicalization.
   Supported configured paths are direct regular files named `master.db`.

Export-workflow records use one exact exported condition value rather than
free-form success prose:

```js
export const XML_BACKUP_SUCCESS_CONDITION =
  'XML export proceeds only after the built-in backup succeeds or the configured custom script exits zero'
```

Every XML-producing record's sole `backup` entry must use that constant with
`mode: 'on-export'`. The custom-script clause is an operator-script zero-exit
attestation, not proof that the server inspected its archive.

## Commands you will need

| Purpose           | Command                                                                                                                         | Expected on success                           |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| Backup tests      | `cargo test -p reklawdbox pre_op_backup -- --nocapture`                                                                         | exit 0; missing/custom/path cases pass        |
| XML failure tests | `cargo test -p reklawdbox write_xml_fails_closed -- --nocapture`                                                                | exit 0; no output and staged changes restored |
| Full crate        | `cargo clippy -p reklawdbox --all-targets -- -D warnings && cargo test -p reklawdbox --no-fail-fast`                            | exit 0                                        |
| Release/MCP       | `cargo build --release && node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`            | exit 0                                        |
| Docs contract     | `cd site && npm run build && cd .. && node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist` | exit 0                                        |
| Format            | `cargo fmt --check && dprint check`                                                                                             | exit 0                                        |

## Scope

**In scope**:

- `src/backup.rs`
- `scripts/backup.sh`
- `src/tools/mod.rs`
- `src/tools/staging_handlers.rs`
- `src/tools/tests.rs`
- `site/src/content/docs/reference/environment-variables.md`
- `site/src/content/docs/reference/xml-export.mdx`
- `site/src/content/docs/cli/index.mdx`
- `site/src/data/workflows.mjs` to update Plan 026's structured backup effects
  from the prior conditional/skipped behavior to the integrated fail-closed
  contract
- `scripts/check-doc-contract.mjs` if a public command/env marker changes
- `scripts/check-doc-contract.test.mjs` for the matching fixture
- `plans/README.md` for the status row only

**Out of scope**:

- Changing archive format, retention counts, restore UX, or backup location.
- Backing up audio files or claiming perfect SQLite coherence while Rekordbox is
  actively running.
- Writing to, locking, checkpointing, or otherwise mutating `master.db`.
- Logging environment contents, encryption keys, tokens, or credentials.
- General server-state refactors unrelated to sharing the resolved path.
- Turning backup failure into an optional warning or adding a bypass.

## Git workflow

- Branch: `codex/029-make-export-backups-fail-closed`
- Preferred commit: `fix(backup): bind export backup to configured database`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Replace unsafe success expectations with red tests

Before implementation, add hermetic tests proving:

- `run_pre_op_backup` returns an error when the configured custom script path
  does not exist;
- `write_xml` creates no XML after that error and restores all staged changes;
- the existing nonzero-script case remains fail closed;
- the error names the missing script path but does not dump environment state.

Serialize environment-mutating tests using the repository's existing env-test
guard/pattern. Restore every environment variable on all exits. Do not use the
real `HOME`, store, Keychain, DB, or backup directory.

**Verify**:

```bash
if cargo test -p reklawdbox pre_op_backup_missing_script_fails_closed -- --nocapture; then
  printf 'expected the new missing-script regression to fail before implementation\n' >&2
  exit 1
fi
```

Expected at this step: the named test runs and fails because the missing custom
script is still treated as a skipped success. Zero tests, a user-path access, or
a compile failure is not an acceptable red result.

### Step 2: Resolve one effective DB path for server lifetime

Add a small `OnceLock<Result<PathBuf, String>>` or equivalent to `ServerState`.
Provide one helper that:

- honors the constructor override first, then `db::resolve_db_path`;
- requires an existing regular file representing the configured `master.db`;
- uses `symlink_metadata` before canonicalization and rejects a symlink at the
  effective path with a clear error; requires the direct filename
  `master.db`, then canonicalizes it once and rechecks it is a regular
  `master.db` file;
- is used by both `rekordbox_conn` and `write_xml` backup;
- does not reopen the connection writable.

Update every production and test constructor. Keep test injection possible with
temporary DB paths. This plan intentionally does not support symlinked DB paths:
the connection and embedded archive must refer to the same direct file bytes,
and no staging/dereference architecture is in scope. Document the rejection in
the environment reference.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "effective_db_path_shared_with_backup"
cargo test -p reklawdbox effective_db_path_shared_with_backup -- --nocapture
```

Expected: the named temp-path tests exist and pass, proving connection and
backup helpers receive the same canonical path without opening it writable and
that a symlink or non-`master.db` filename is rejected before any backup/export.

### Step 3: Make backup success unambiguous

Change `run_pre_op_backup` to accept the resolved DB path. Remove
`CustomNotFound` (or make it impossible) so `Ok(Success)` is the only success.
A missing custom script must return `Err` before spawning a child.

Refactor `execute_script` to accept explicit environment additions. For the
pre-op path:

- keep `--pre-op` as the first argument for custom-script compatibility;
- set child `REKORDBOX_DB_PATH` to the exact effective path;
- capture bounded stdout/stderr on failure without printing unrelated env;
- continue treating signal/nonzero/launch/join failures as errors.

Do not claim the server verified an arbitrary custom script's archive. A zero
exit is the custom script's trusted success attestation; built-in script tests
provide the stronger archive-membership proof for the shipped implementation.

Do not set or mutate the parent process environment to pass the path.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "pre_op_backup_path_env"
cargo test -p reklawdbox pre_op_backup_path_env -- --nocapture
```

Expected: the named tests pass for missing script, first `--pre-op` argument,
exact child path, nonzero exit, and restored parent environment.

### Step 4: Make the embedded shell script path-aware

After parsing the selected mode:

- for backup-producing modes (`--pre-op`, `--db-only`, and the default full
  backup), when `REKORDBOX_DB_PATH` is set, require it to be the regular
  `master.db` source and derive `RB_DATA` from its parent directory;
- confirm the configured file is the `master.db` source included in the
  database backup set;
- otherwise retain the standard auto/default path for manual CLI use;
- quote paths and arrays correctly, including spaces and glob characters.

`--list` and `--help` must not require a live source database. `--restore` must
use the configured path's parent as its target without requiring the source
`master.db` file to exist first; validate the archive and target directory
according to the existing restore safety rules. Add a hermetic missing-DB
restore regression so path-aware validation does not make disaster recovery
impossible. Do not silently fall back to the standard directory in any mode
after a configured path was supplied.

The database-only archive must take `master.db`, WAL/SHM, and the existing
allowlisted related files from the same derived directory. Never silently fall
back to the standard directory after receiving an invalid configured path.

Full-backup compatibility is exact: archives always contain one canonical
top-level directory named `rekordbox/`, even when the configured source parent
has a different basename. Full restore validates that canonical root and maps
its contents onto the configured `RB_DATA` target; it must not require the
target basename to equal the archive root. Apply the same canonical-root rule to
pre-restore full safety archives. Preserve support for historical full archives
that already use `rekordbox/`; do not emit a custom basename into a new archive.

For a confirmed DB restore into a configured target whose `master.db` is
missing, inspect the full allowlist before the pre-restore safety step. If any
current allowlisted file exists, the pre-restore DB archive remains mandatory
and any failure aborts before mutation. If none exists, print the explicit
message `No current database files to back up; continuing restore.` and proceed
without inventing an empty archive. The target directory and input archive must
still pass every existing restore validation.

Ensure manual backup/list/restore modes retain their existing safety semantics,
with the configured source/target honored where relevant. Any restore behavior
needed for a missing source DB must be covered by the hermetic regression above.

**Verify**:

```bash
bash -n scripts/backup.sh
cargo test -p reklawdbox -- --list | rg "embedded_backup_custom_db_path"
cargo test -p reklawdbox embedded_backup_custom_db_path -- --nocapture
cargo test -p reklawdbox -- --list | rg "embedded_backup_mode_specific_path"
cargo test -p reklawdbox embedded_backup_mode_specific_path -- --nocapture
```

Expected: shell syntax passes and named hermetic tests prove an invalid source
fails for backup modes rather than falling back, `--list`/`--help` work without
a source DB, DB restore with no current allowlisted files follows the exact
message/continue rule, DB restore with any current allowlisted file requires a
successful safety archive, and configured full backup/restore preserves the
canonical `rekordbox/` root including paths with spaces.

### Step 5: Pass the path through transactional XML export

In `write_xml`, obtain the effective path before invoking pre-op backup and pass
it directly. Preserve the export lock and staged-change RAII guard from Plan
002. Every path/backup error must occur before output-file creation, and the
guard must restore changes for retry.

Successful JSON should report only an unambiguous backup success. Remove the
skipped status from schemas/tests/docs.

**Verify**:

```bash
cargo test -p reklawdbox write_xml_fails_closed -- --nocapture
```

Expected: every focused backup failure creates no XML and restores staged
changes; the success case reports only `backup: "success"`.

### Step 6: Add hermetic child/script integration tests

Use temporary directories with spaces to cover:

- a synthetic custom Rekordbox directory containing `master.db` plus sentinel
  sidecars;
- a separate fake standard directory that must not appear in the archive;
- embedded database-only/pre-op script output whose member list contains the
  custom sentinel and correct DB files;
- a custom script that records its first argument and
  `REKORDBOX_DB_PATH` into a temp marker, proving `--pre-op` and the exact path;
- missing and non-file paths;
- successful standard/default behavior using only a temporary child `HOME`.
- `--list` and `--help` with a configured but missing DB source;
- DB restore into the configured target directory when its `master.db` is
  missing, without touching the standard-path sentinel;
- DB restore when no allowlisted target files exist (exact continue message)
  and when a sidecar exists (mandatory pre-restore safety archive);
- a configured full backup and full restore round trip where the target
  directory basename is not `rekordbox`, proving the archive still has exactly
  one `rekordbox/` root and restores into the configured target;
- rejection of a symlinked configured `master.db` before the script runs.

Inspect tar members without extracting over user paths. Bound child execution
and clean up processes/temp files on every test exit.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "backup_script_custom_path"
cargo test -p reklawdbox backup_script_custom_path -- --nocapture
```

Expected: the named integration tests pass; archive members come only from the
custom directory, the fake standard sentinel is absent, spaces are handled,
the canonical full archive round trip is backward-compatible, both missing-
source safety-backup branches are covered, symlinks fail closed, and child
processes are bounded/cleaned up.

### Step 7: Correct public backup and path documentation

Update the environment, XML export, and CLI backup references to state:

- automatic and manual backup source selection follows the effective
  `REKORDBOX_DB_PATH` when configured;
- configured paths must be direct regular files named `master.db`; symlinks are
  rejected before connection or backup;
- full archives retain the canonical `rekordbox/` root and restore it into the
  configured target directory regardless of that directory's basename;
- custom scripts receive `--pre-op` plus child `REKORDBOX_DB_PATH`;
- custom-script exit zero is trusted as that operator-supplied script's success
  attestation, while the shipped embedded script has archive-membership tests;
- a missing or failed custom script blocks XML export;
- staged changes remain available for retry after failure;
- closing Rekordbox produces the strongest backup consistency.

Update every XML-producing record in `site/src/data/workflows.mjs` so its
`backup` output entry uses the canonical `on-export` mode and states that XML
creation proceeds only after successful backup. Preserve every other
conditional effect and validate the module; do not flatten the structured
contract back into string arrays.

Export `XML_BACKUP_SUCCESS_CONDITION` with the exact value in the Required
Safety Contract and assign that constant to every XML-producing backup entry.
Extend `validateWorkflows` to require exact equality, not a word/regex match.

Extend Plan 027's checker and fixture tests with the same semantic rule: every
record that can create metadata or playlist XML has exactly one `backup` output
entry, its mode is `on-export`, and its condition equals the exported canonical
constant. Add negative fixtures that remove the entry, negate the condition,
and replace it with weaker text that merely contains the word `success`;
shape-only `validateWorkflows()` or a prose regex is not sufficient.

Do not claim audio files are backed up or that archives are an online database
snapshot. Preserve Plan 024's internal-store recovery wording.

**Verify**:

```bash
rg -n -e "REKORDBOX_DB_PATH" -e "--pre-op" -e "block.*export" -e "staged.*retry" -e "close Rekordbox" site/src/content/docs/reference/environment-variables.md site/src/content/docs/reference/xml-export.mdx site/src/content/docs/cli/index.mdx
! rg -n "skipped_custom_not_found" src site/src/content/docs
node -e "import('./site/src/data/workflows.mjs').then(m => m.validateWorkflows(m.workflows))"
node -e "import('./site/src/data/workflows.mjs').then(({workflows,XML_BACKUP_SUCCESS_CONDITION}) => { for (const w of workflows) { const outputs=w.sideEffects.outputs; const hasXml=outputs.some(x => x.kind==='metadata-xml' || x.kind==='playlist-xml'); if (!hasXml) continue; const backups=outputs.filter(x => x.kind==='backup'); if (backups.length !== 1 || backups[0].mode !== 'on-export' || backups[0].condition !== XML_BACKUP_SUCCESS_CONDITION) throw Error(w.id); } })"
```

Expected: every documented guarantee is present, the old skipped-success status
has no source or public-doc match, the structured module validates, and every
XML-producing workflow has exactly one canonically gated `on-export` backup
entry whose custom-script wording remains an attestation.

### Step 8: Run full safety and documentation gates

Run focused tests, full Rust gates, release/MCP smoke, site build, and the Plan
027 checker. Review the diff for path interpolation, shell quoting, accidental
secret output, and any write-capable SQLite flags.

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
cd site && npm run build && cd ..
node --test scripts/check-doc-contract.test.mjs
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: every command exits 0. Diff review confirms no writable DB flags,
unquoted path interpolation, parent-env mutation, credential output, or files
outside Scope.

## Test plan

- Unit tests invert the missing-script behavior and preserve nonzero handling.
- Transactional handler tests assert no XML and restored changes after every
  failure class.
- Temp child-process tests prove exact configured-path propagation and archive
  membership, including spaces.
- Existing standard/default backup behavior remains covered without user data.
- Full test suite and docs checker protect public schema/prose integration.

## Done criteria

- [ ] Missing custom script is an error, never a skipped success.
- [ ] XML is never created after any backup/path failure.
- [ ] Staged changes are restored for retry on every failure.
- [ ] Connection and backup use one resolved canonical DB path.
- [ ] The embedded script honors that path; custom scripts receive it and attest success by exiting zero.
- [ ] Archives use the configured directory, including paths with spaces.
- [ ] Full archives keep the historical `rekordbox/` root and round-trip into a differently named configured target.
- [ ] Empty-target DB restore skips only an impossible safety archive; any existing allowlisted file keeps it mandatory.
- [ ] Symlinked or misnamed configured DB paths fail before backup/export.
- [ ] Standard installs continue to use the standard location.
- [ ] Canonical XML-workflow backup effects reflect the fail-closed contract.
- [ ] Public docs distinguish built-in archive proof from trusted custom-script exit status.
- [ ] No credential or unrelated environment content is logged.
- [ ] Focused/full Rust, release/MCP, site, docs-contract, and format gates pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- The DB path cannot be shared exactly between connection and backup.
- Supporting it requires weakening read-only access or changing archive/restore
  compatibility.
- A test touches the user's real library, HOME backup directory, or secrets.
- Custom-script compatibility requires replacing the existing first argument.
- Correct behavior needs a wider restore redesign.
- Integrated transactional export no longer restores staged changes.

## Maintenance notes

- Built-in success must mean the archive was moved into place. Custom-script
  success is the operator-supplied script's trusted zero-exit attestation and
  must remain documented as such.
- Review shell quoting and path canonicalization on every backup change.
- Keep path selection centralized; do not let connection and safety tooling
  resolve environment/defaults independently.
- Rekordbox-running snapshot coherence remains a separately disclosed
  limitation, not solved by this plan.
