# Plan 024: Preserve internal state during broker recovery

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- src/cli/mod.rs src/store.rs site/src/content/docs/troubleshooting/index.mdx site/src/content/docs/reference/environment-variables.md site/src/content/docs/concepts/index.mdx site/src/content/docs/cli/index.mdx
> ```
>
> If broker credentials or internal-store path resolution have changed, compare
> the live behavior with this plan before proceeding. Never inspect or print a
> real credential while performing the drift check.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 021
- **Category**: bug
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

Troubleshooting currently recommends deleting the entire internal SQLite file
or running raw SQL to recover a Discogs session. That file contains more than
reconstructible caches: it also stores audit history, custom scoring presets,
calibration/profile statistics, and broker session metadata, while the session
credential itself lives in macOS Keychain. A targeted `disconnect-broker`
command already exists, but it currently ignores `CRATE_DIG_STORE_PATH` and
opens the default store. Recovery should use the supported targeted command,
preserve unrelated state, and describe destructive last-resort actions
honestly.

## Current state

- `site/src/content/docs/troubleshooting/index.mdx:36-61` calls whole-file or
  session-table deletion the hard reset path.
- `site/src/content/docs/reference/environment-variables.md:50-56` calls the
  internal database a disposable cache containing broker tokens.
- `site/src/content/docs/concepts/index.mdx:21-25` says the SQLite file can be
  safely deleted and everything will be re-fetched.
- `site/src/content/docs/cli/index.mdx:300-305` accurately documents the
  targeted `reklawdbox disconnect-broker` command.

The store schema at `src/store.rs:85-155` includes:

- broker session metadata;
- audit files, issues, statuses, resolutions, and notes;
- timbral normalization statistics;
- custom weight presets;
- genre audio profiles and timbral centroids;
- global genre statistics.

`src/store.rs:1033-1078` migrates legacy plaintext credentials into Keychain
and stores an empty credential column plus metadata in SQLite. Do not reproduce
any credential, token, broker secret, Keychain value, or database encryption
key in documentation, tests, logs, or this plan's implementation.

`src/store.rs:1107-1119` already provides targeted clearing that removes the
Keychain credential and the selected broker's SQLite metadata.

Current `src/cli/mod.rs:240-252` does not honor the configured store path:

```rust
Cli::DisconnectBroker => {
    // resolve broker configuration
    let store_path = store::default_path();
    let conn = store::open(store_path.to_str().unwrap_or("internal.sqlite3"))?;
    store::clear_broker_discogs_session(&conn, &cfg.base_url)?;
    // ...
}
```

`src/store.rs:13-25` defines `resolve_path()` as the existing
`CRATE_DIG_STORE_PATH`-aware path resolver. Match other CLI cache consumers by
using that function.

## Commands you will need

| Purpose                       | Command                                                                                                                                                                                   | Expected on success                 |
| ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| Configured path regression    | `cargo test -p reklawdbox store::tests::configured_store_path_is_used_verbatim -- --exact`                                                                                                | exit 0; exactly 1 test passes       |
| Static CLI path check         | `rg -n "let store_path = store::default_path" src/cli/mod.rs`                                                                                                                             | exit 1 after the fix                |
| Destructive normal-path check | `rg -n -e "rm .*internal\.sqlite3" -e "DELETE FROM broker_discogs_session" -e "delete-generic-password" site/src/content/docs/troubleshooting/index.mdx`                                  | exit 1 after the rewrite            |
| Disposable-state check        | `rg -n -e "safe to delete" -e "Everything gets re-fetched" -e "stores.*session tokens" site/src/content/docs/reference/environment-variables.md site/src/content/docs/concepts/index.mdx` | exit 1 after the rewrite            |
| Format                        | `cargo fmt --check && dprint check`                                                                                                                                                       | exit 0                              |
| Lint                          | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                                                                                                                 | exit 0; no warnings                 |
| Tests                         | `cargo test -p reklawdbox --no-fail-fast`                                                                                                                                                 | exit 0; all tests pass              |
| CLI help                      | `cargo build --release && ./target/release/reklawdbox disconnect-broker --help`                                                                                                           | exit 0; describes targeted clearing |
| Site build                    | `cd site && npm ci && npm run build`                                                                                                                                                      | exit 0                              |

## Scope

**In scope** (the only source/documentation files you may modify):

- `src/cli/mod.rs`
- `site/src/content/docs/troubleshooting/index.mdx`
- `site/src/content/docs/reference/environment-variables.md`
- `site/src/content/docs/concepts/index.mdx`
- `site/src/content/docs/cli/index.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Changing the OAuth/device authorization protocol or broker endpoints.
- Changing Keychain service/account design, exposing credentials, or logging
  credential material.
- Deleting, migrating, or reclassifying store tables.
- Redefining `clear_caches` or adding a general destructive reset command.
- Running `disconnect-broker` against the user's live store or Keychain in a
  test.
- Adding a new internal-state backup feature.
- Changing `CRATE_DIG_STORE_PATH` semantics.

## Git workflow

- Branch: `codex/024-preserve-internal-state-during-recovery`
- Use Conventional Commits; preferred final message:
  `fix(cli): preserve configured state during broker recovery`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Make the supported command honor the configured store

In the `Cli::DisconnectBroker` branch in `src/cli/mod.rs`, replace
`store::default_path()` with `store::resolve_path()`. Preserve:

- existing broker URL validation;
- the targeted `clear_broker_discogs_session` call;
- idempotent behavior;
- absence of credential logging.

Do not add a fallback that silently opens another store when the configured
path is invalid. Path-open failures must remain visible.

**Verify**:

```bash
rg -n "store::resolve_path\(\)" src/cli/mod.rs
rg -n "let store_path = store::default_path" src/cli/mod.rs
```

Expected: first command finds the disconnect branch; second exits 1.

### Step 2: Replace destructive authentication recovery with a targeted ladder

Rewrite the affected troubleshooting section in this order:

1. finish or restart the normal `lookup_discogs` authorization flow;
2. run `reklawdbox disconnect-broker` if the stored session is bad;
3. restart/reload the MCP host;
4. call `lookup_discogs` again to begin a fresh flow.

Explain that the command clears only the current broker session from Keychain
and the configured internal store. Remove whole-file deletion, raw SQL, and
raw Keychain commands from the normal recovery path.

If a last-resort corruption section remains, it must be clearly separate and
must instruct the user to close the host and **move** the database to a backup
filename rather than immediately deleting it. Enumerate the durable categories
that would be lost. Do not present this as an authentication reset and do not
claim it clears an orphaned Keychain credential.

**Verify**:

```bash
rg -n "disconnect-broker|configured internal|audit|preset|calibration" site/src/content/docs/troubleshooting/index.mdx
```

Expected: exit 0; the supported command and preservation consequences appear.

### Step 3: Rename and classify the internal state database

In `reference/environment-variables.md`, replace "cache database" with
"internal state database" and classify its contents:

- reconstructible enrichment and audio-analysis caches;
- audit history and user resolutions;
- weight presets;
- calibration/profile statistics;
- broker session metadata;
- credential stored separately in macOS Keychain.

Describe `CRATE_DIG_STORE_PATH` as selecting all of that state. State that
deleting or moving the file loses durable decisions/settings even though some
cache rows can be rebuilt. Link the targeted recovery command.

Do not list private values or imply that the SQLite credential column contains
the active secret.

**Verify**:

```bash
rg -n "internal state|reconstruct|audit|preset|calibration|Keychain" site/src/content/docs/reference/environment-variables.md
```

Expected: exit 0; all storage categories are represented.

### Step 4: Align the concepts and CLI summaries

In `concepts/index.mdx`, keep a short overview: some cached results are
reconstructible, but the same local database contains durable audit and
customization state. Remove the universal safe-delete claim.

In `cli/index.mdx`, state that `disconnect-broker` respects
`CRATE_DIG_STORE_PATH` and preserves unrelated cache/audit/preset/calibration
state. Keep the command idempotent and targeted.

**Verify**:

```bash
rg -n "CRATE_DIG_STORE_PATH|preserv" site/src/content/docs/cli/index.mdx
```

Expected: exit 0; both configured-path and preservation behavior are explicit.

### Step 5: Run focused and full verification

Do not invoke the real disconnect command during tests because it would touch
Keychain. Use the existing pure path-resolution test plus the static code
assertion. Run the full crate and site gates.

**Verify**:

```bash
cargo test -p reklawdbox store::tests::configured_store_path_is_used_verbatim -- --exact
cargo fmt --check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox disconnect-broker --help
dprint check
cd site && npm run build
```

Expected: every command exits 0; no test accesses a live credential.

## Test plan

- Reuse `src/store.rs::tests::configured_store_path_is_used_verbatim` as the
  pure path-resolution regression.
- The code-level regression oracle is that the CLI branch calls
  `store::resolve_path`, not `default_path`.
- Do not add an integration test that calls Keychain or the user's live store.
- Documentation negative checks prohibit destructive commands in the normal
  recovery path and disposable-state claims.
- Build the site to validate MDX and links.

## Done criteria

- [ ] `disconnect-broker` uses the configured internal-store path.
- [ ] Normal recovery uses the supported targeted command.
- [ ] Raw whole-file, SQL, and Keychain deletion commands are absent from the normal path.
- [ ] Last-resort wording preserves a backup and discloses durable data loss.
- [ ] Internal storage is split into reconstructible and durable categories.
- [ ] Credential location is accurate without exposing a value.
- [ ] Focused test, full Rust gates, CLI help, format, and site build pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- The disconnect command has changed to another store-selection mechanism.
- Correct behavior would require reading, displaying, or logging a credential.
- Respecting `CRATE_DIG_STORE_PATH` requires a store migration.
- Store schema contents no longer match the documented categories.
- A test would touch the user's real Keychain entry or internal database.
- In-scope files materially differ from the recorded excerpts.

## Maintenance notes

- Classify every new internal table as reconstructible cache or durable user
  state and update the storage reference.
- Authentication recovery must remain targeted; never reuse whole-store
  deletion as a shortcut.
- Reviewers should scrutinize configured-path behavior because the default path
  can hide this regression.
- Plan 029 edits the same environment-variable page for backup guarantees and
  must start from this completed wording.
