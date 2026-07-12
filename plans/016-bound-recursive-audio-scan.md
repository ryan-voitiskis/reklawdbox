# Plan 016: Prevent recursive audio-scan symlink cycles

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` unless the reviewer who dispatched you maintains the index.
>
> **Drift check (run first)**:
> `git diff --stat e6eb382..HEAD -- src/tools/audio_scan.rs`
> This plan has no dependency, so behavioral drift in this file is unexpected.
> Compare the current-state excerpt with live code and STOP on an unrelated
> mismatch.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

`scan_audio_directory` decides whether to recurse with `Path::is_dir`, which
follows symlinks. A self-link, parent-link, or two-name alias to an ancestor can
therefore keep pushing the same directory forever, growing the traversal stack
and result set until the process exhausts resources. Inspect each
`DirEntry` without following it, never recurse into a directory symlink, and
keep a canonical visited-directory set as defense in depth. Preserve current
support for symlinks that resolve to regular audio files.

This remains read-only filesystem inspection. It must not affect Rekordbox DB
access or add a metadata path outside `ChangeManager` and XML export.

## Current state

`src/tools/audio_scan.rs` is the shared scanner used by file-tag reads and
health tools. Its signature and callers are intentionally unchanged by this
plan.

Current traversal (`src/tools/audio_scan.rs:28-72`):

```rust
let mut files = Vec::new();
let mut dirs_to_scan = vec![dir_path.to_path_buf()];

while let Some(current_dir) = dirs_to_scan.pop() {
    let entries = std::fs::read_dir(&current_dir)
        .map_err(|e| format!("Failed to read directory {}: {e}", current_dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Directory entry error: {e}"))?;
        let path = entry.path();

        if path.is_dir() && recursive {
            dirs_to_scan.push(path);
            continue;
        }

        if !path.is_file() {
            continue;
        }
        // ... audio extension and glob checks ...
        files.push(path.display().to_string());
    }
}
```

The CLI's separate collector already uses the repository's intended basic
policy (`src/cli/mod.rs:366-385`): it obtains `entry.file_type()` and excludes
symlink directories from recursion. The shared MCP scanner needs equivalent
protection plus canonical visited-directory tracking, but this plan does not
refactor or merge the two implementations.

Selected symlink policy:

- A directory symlink discovered as a child `DirEntry`: never recurse and
  never return it.
- An explicitly supplied root path that is itself a symlink to a directory:
  preserve current behavior, canonicalize its target identity, and traverse it
  once. The caller chose that root; this exception does not authorize following
  child directory symlinks.
- Symlink to regular audio file: continue to return the symlink path when its
  extension/glob matches, preserving existing behavior.
- Broken or inaccessible symlink: skip it; do not fail the entire scan.
- Real directory reached under an already visited canonical identity: skip it.

## Commands you will need

| Purpose            | Command                                                                                  | Expected on success                             |
| ------------------ | ---------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Focused tests      | `cargo test -p reklawdbox audio_scan -- --nocapture`                                     | exit 0; cycle and ordinary-recursion tests pass |
| Format             | `cargo fmt --check`                                                                      | exit 0; no diff                                 |
| Docs/config format | `dprint check`                                                                           | exit 0                                          |
| Lint               | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                | exit 0; no warnings                             |
| Tests              | `cargo test -p reklawdbox --no-fail-fast`                                                | exit 0; all tests pass                          |
| Release build      | `cargo build --release`                                                                  | exit 0                                          |
| CLI smoke          | `./target/release/reklawdbox --version && ./target/release/reklawdbox --help >/dev/null` | exit 0                                          |

## Scope

**In scope** (the only source file you should modify):

- `src/tools/audio_scan.rs`

**Out of scope** (do not touch):

- `src/tools/file_tag_handlers.rs`, `src/tools/health_handlers.rs`, and every
  scanner caller
- `src/cli/mod.rs` and its independent CLI traversal
- `scan_audio_directory` parameters, return type, error strings, glob behavior,
  result sorting, or callers' post-scan limit/pagination semantics
- Early termination at `ReadFileTagsParams.limit`; that requires a separate
  benchmark and explicit subset-order contract
- Following child/discovered directory symlinks with inode-cycle tracking;
  selected policy is to skip them while preserving the explicit root exception
- A new traversal dependency
- Rekordbox DB, native file mutation, `ChangeManager`, or XML export

## Git workflow

- Branch: `codex/016-bound-recursive-audio-scan`
- Commit: `fix(tools): prevent recursive audio scan cycles`
- Use Conventional Commits. Do not push or open a PR unless instructed.

## Steps

### Step 1: Characterize ordinary traversal and prepare symlink fixtures

Add a `#[cfg(test)]` module in `src/tools/audio_scan.rs` with tests named using
the `audio_scan` prefix. Use `tempfile::TempDir` and synthetic empty files.

Required ordinary cases:

1. Non-recursive mode returns audio files in the root but not a real child
   directory.
2. Recursive mode returns audio files from ordinary nested directories exactly
   once and keeps results sorted.
3. Extension and glob filtering still work.

Prepare these Unix symlink cases under `#[cfg(unix)]`:

4. A root directory containing a symlink back to itself terminates and returns
   each real audio file once.
5. A nested directory containing a symlink to its parent/root terminates without
   duplicates.
6. Two directory aliases pointing to the same real directory are skipped rather
   than traversed.
7. A symlink to a regular audio file remains present once under its symlink path.
8. A broken symlink is ignored without failing the scan.
9. An explicitly supplied root symlink to a real directory remains supported,
   returns its audio files once, and still does not follow child directory
   symlinks beneath it.

No ordering assertion may rely on time-based sleeps. Keep every new Unix symlink-policy case
4-9 temporarily `#[ignore]` while running the Step 1 ordinary baseline: some
can loop forever and the alias/root-child assertions intentionally fail before
the implementation. Remove every temporary `#[ignore]` in Step 2 before
executing any symlink regression; do not commit ignored tests.

The self/parent-cycle regressions must run in a killable child process after
Step 2, not on the test runner's own thread. Add one exact-filter helper test
selected by a private environment case value and parent tests that spawn
`std::env::current_exe()`, wait with `Child::try_wait`, and enforce a five-second
deadline. On timeout, kill and reap the child before failing. A short polling
sleep is allowed only in this watchdog loop, never to assert traversal order.
The child builds its temporary symlink fixture, calls the synchronous scanner,
and asserts result identity/counts. Do not add a process-timeout dependency.

**Verify**: `cargo test -p reklawdbox audio_scan -- --nocapture` → ordinary
cases pass and the harness reports all prepared Unix symlink-policy cases as
ignored. Do not run any of them against the old traversal.

### Step 2: Classify entries without following directory symlinks

Inside the existing loop, call `entry.file_type()` before `Path::is_dir` or
`Path::is_file`:

- For a real directory, consider it for recursion.
- For a real regular file, run the existing extension/glob filters.
- For a symlink, call `std::fs::metadata(&path)` only to determine whether its
  target is a regular file. If so, retain current audio-file filtering and
  return the symlink path. If the target is a directory, broken, inaccessible,
  or another non-file type, skip it.

Do not use `path.is_dir()` on a symlink. Preserve existing errors for failure to
read real directory entries; only broken/inaccessible symlink targets are
best-effort skips.

This entry policy applies after the root has been validated. Preserve the
existing `dir_path.is_dir()` root acceptance so an explicitly passed directory
symlink remains valid; Step 3 canonicalizes that root before traversal.

Remove the temporary `#[ignore]` attributes from every symlink-policy
regression before running the focused suite.

Run self-link and parent-link cases through the child-process watchdog. Alias,
file-link, broken-link, and root-link cases may run directly because they are
finite once child directory symlinks are skipped.

**Verify**: `cargo test -p reklawdbox audio_scan -- --nocapture` → all symlink
policy tests pass and the scanner terminates.

### Step 3: Add canonical visited-directory defense

Maintain `HashSet<PathBuf>` of canonical directory identities:

1. Canonicalize and insert the validated root before traversal.
2. Before pushing a real child directory, canonicalize it and push only if its
   identity was newly inserted.
3. Treat failure to canonicalize a real directory consistently with current
   read errors: return an error containing the path rather than silently
   changing successful-directory semantics.

The visited set is defense against a real directory identity being exposed more
than once by platform/filesystem aliasing. It does not authorize following
directory symlinks.

**Verify**: `cargo test -p reklawdbox audio_scan -- --nocapture` → all ordinary,
alias, self-cycle, parent-cycle, file-symlink, and broken-link cases pass; no
test is ignored.

### Step 4: Run the repository gate

Run every command in "Commands you will need". Fix only failures caused by
`src/tools/audio_scan.rs`.

**Verify**: every command exits 0 with its listed expected result.

## Test plan

- Ordinary recursive/non-recursive traversal and sorted output.
- Existing extension/glob behavior.
- Self-link and parent-link cycles.
- Child-process watchdog kills/reaps a regressed infinite cycle within five
  seconds, so the crate test cannot hang.
- Multiple aliases to one directory identity.
- Preserved symlink-to-audio-file support.
- Preserved explicit root-directory-symlink support with child links skipped.
- Broken symlink skip behavior.
- Complete crate gate to cover all unchanged callers.

## Machine-checkable done criteria

- [ ] `scan_audio_directory` never pushes a directory entry whose
      `DirEntry::file_type().is_symlink()` is true.
- [ ] Real directories are visited at most once by canonical identity.
- [ ] Audio-file symlinks remain supported and broken symlinks are skipped.
- [ ] An explicit root directory symlink is traversed exactly once, while no
      child directory symlink is pushed.
- [ ] Self/parent cycle regressions run in a bounded child process that is
      killed and reaped on deadline; no infinite scanner thread can survive.
- [ ] `rg -n '#\[ignore\]' src/tools/audio_scan.rs` returns no matches.
- [ ] Function signature, result sorting, glob behavior, and caller limit
      semantics are unchanged.
- [ ] No caller or dependency file is modified.
- [ ] `cargo fmt --check`, `dprint check`, clippy, full tests, release build, and
      CLI smoke all exit 0.
- [ ] `git diff --name-only` contains only `src/tools/audio_scan.rs` and the
      plan/index status update.
- [ ] `plans/README.md` marks plan 016 DONE, unless the dispatcher owns the index.

## STOP conditions

Stop and report back instead of improvising if:

- The scanner implementation/signature differs from the current-state excerpt.
- A supported caller explicitly requires recursive traversal through directory
  symlinks; do not invent an inode-based followed-link mode.
- Preserving audio-file symlinks requires following a directory symlink or
  changing returned path identity.
- Canonicalizing a real directory conflicts with a supported noncanonical path
  contract or causes an existing fixture to fail for a legitimate reason.
- The fix appears to require changing caller limits, health semantics, or adding
  a dependency.
- Any verification command fails twice for a reason unrelated to this file.

## Maintenance notes

- Entry type must be checked before any convenience method that follows links.
- Keep the distinction between directory symlinks (never followed) and regular
  audio-file symlinks (supported) covered explicitly in tests.
- Keep the root exception explicit: caller-supplied directory symlinks are
  roots, while directory symlinks discovered during traversal are skipped.
- The canonical visited set is not a license to follow directory symlinks; it is
  defense in depth for real directory aliases.
- Early result-limit termination is intentionally deferred because it changes
  which sorted subset is returned and does not address the selected cycle bug.
