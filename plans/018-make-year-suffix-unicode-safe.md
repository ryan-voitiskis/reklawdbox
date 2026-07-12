# Plan 018: Make year-suffix parsing Unicode-safe

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan in
> `plans/README.md` unless the reviewer who dispatched you maintains the index.
>
> **Dependency and drift check (run first)**: This plan must start from the
> reviewed DONE commit for `plans/011-make-audit-freshness-complete.md`, then run
> `git diff --stat e6eb382..HEAD -- src/audit.rs`. Changes made by plan 011 are
> expected and must be retained. Reconcile the excerpt below against the live
> `has_year_suffix` implementation; unrelated drift or a changed helper contract
> is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: S
- **Risk**: LOW
- **Depends on**: `plans/011-make-audit-freshness-complete.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

`has_year_suffix` finds a parenthesized suffix using valid UTF-8 boundaries, but
then slices `inside[..4]` by byte offset. If the parenthetical text begins with
multibyte characters and byte four is not a character boundary, collection
audit panics instead of reporting a normal non-year suffix. Use checked string
slicing for the four-character ASCII year prefix and add Unicode regressions
without changing the accepted year range or compound-suffix behavior.

This remains a read-only audit classification change. It must not affect
Rekordbox database access or metadata persistence through `ChangeManager`/XML.

## Current state

`src/audit.rs` contains directory-name normalization, year classification, and
its unit tests. Plan 011 may have changed audit freshness orchestration elsewhere
in this file; preserve all of those reviewed changes.

Current parser (`src/audit.rs:259-281` at planned commit):

```rust
fn has_year_suffix(name: &str) -> bool {
    let trimmed = name.trim_end();
    if trimmed.len() < 6 {
        return false;
    }
    let bytes = trimmed.as_bytes();
    if bytes[bytes.len() - 1] != b')' {
        return false;
    }
    if let Some(open) = trimmed.rfind('(') {
        let inside = &trimmed[open + 1..trimmed.len() - 1];
        if inside.len() >= 4
            && inside[..4].bytes().all(|b| b.is_ascii_digit())
            && let Ok(year) = inside[..4].parse::<u16>()
        {
            return (1900..=2099).contains(&year);
        }
        false
    } else {
        false
    }
}
```

Existing tests establish intended semantics (`src/audit.rs:1500-1513`,
`src/audit.rs:2341-2357`): plain `(2024)` is accepted; non-year parentheses and
out-of-range years are rejected; `(1969, 2004)`, `(2017, Label - Cat)`, and
`(2020 Remaster)` are accepted because the first four characters are a valid
year.

The correct local Rust pattern is checked `str::get(range)`, not byte indexing
or allocating a new `String` from `chars()`.

## Commands you will need

| Purpose            | Command                                                                                  | Expected on success                           |
| ------------------ | ---------------------------------------------------------------------------------------- | --------------------------------------------- |
| Focused tests      | `cargo test -p reklawdbox year_suffix -- --nocapture`                                    | exit 0; existing and Unicode regressions pass |
| Audit tests        | `cargo test -p reklawdbox audit::tests -- --nocapture`                                   | exit 0                                        |
| Format             | `cargo fmt --check`                                                                      | exit 0; no diff                               |
| Docs/config format | `dprint check`                                                                           | exit 0                                        |
| Lint               | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                | exit 0; no warnings                           |
| Tests              | `cargo test -p reklawdbox --no-fail-fast`                                                | exit 0; all tests pass                        |
| Release build      | `cargo build --release`                                                                  | exit 0                                        |
| CLI smoke          | `./target/release/reklawdbox --version && ./target/release/reklawdbox --help >/dev/null` | exit 0                                        |

## Scope

**In scope** (the only source file you should modify):

- `src/audit.rs` — only `has_year_suffix` and its directly adjacent/unit tests

**Out of scope** (do not touch):

- Any audit freshness logic delivered by plan 011
- `normalize_dir_name`, `has_year_range`, directory classification policy, or
  accepted 1900–2099 range
- Treating Unicode decimal digits as years; year prefixes remain four ASCII
  digits
- Audit issue schemas, store migrations, CLI/MCP parameters, or docs
- Rekordbox DB or metadata write paths

## Git workflow

- Base: reviewed DONE commit from plan 011
- Branch: `codex/018-make-year-suffix-unicode-safe`
- Commit: `fix(audit): make year suffix parsing unicode safe`
- Use Conventional Commits. Do not push or open a PR unless instructed.

## Steps

### Step 1: Add Unicode boundary regressions

Next to the existing `year_suffix_*` tests in `src/audit.rs`, add cases that:

1. Call `has_year_suffix("Album (日本2024)")` and assert `false` without panic;
   this input places byte offset four inside a multibyte code point under the
   current implementation.
2. Reject another non-ASCII prefix such as an emoji before `2024`.
3. Accept a multibyte album name followed by an ordinary ASCII `(2024)`.
4. Accept `(2024 日本盤)` because existing compound-suffix semantics inspect
   only the first four ASCII digits.
5. Preserve existing range-boundary and compound-year cases.

Do not wrap the calls in `catch_unwind`; the test itself should fail naturally
if the parser panics.

**Verify**: `cargo test -p reklawdbox year_suffix -- --nocapture` → before Step
2, the multibyte-prefix regression panics at the current slice; after Step 2,
all cases pass.

### Step 2: Replace unchecked byte slicing with checked UTF-8 slicing

After extracting `inside`, obtain the prefix with `inside.get(..4)`. If it
returns `None`, return `false`. Validate that checked `&str` with
`bytes().all(u8::is_ascii_digit)` and parse the same checked prefix once.

Do not use `unsafe`, byte-to-string reconstruction, lossy conversion, regex,
or allocation. Keep the existing trailing-parenthesis check, range, and
first-four-character compound-suffix semantics.

**Verify**: `cargo test -p reklawdbox year_suffix -- --nocapture` → exit 0; all
old and new year-suffix tests pass.

### Step 3: Re-run the complete audit and repository gates

Run the focused audit suite and then every remaining command in "Commands you
will need". Do not weaken or delete plan 011's freshness tests if they fail.

**Verify**: every command exits 0 with the listed expected result.

## Test plan

- Non-boundary multibyte prefix returns `false` rather than panicking.
- Emoji/non-ASCII prefix is rejected.
- Unicode elsewhere in the album/suffix remains supported when the first four
  suffix characters are ASCII year digits.
- Existing exact, compound, absent, and range tests remain unchanged and pass.
- Full audit suite confirms no freshness regression from plan 011.

## Machine-checkable done criteria

- [ ] `rg -n 'inside\[\.\.4\]' src/audit.rs` returns no matches.
- [ ] `has_year_suffix` uses a checked `str::get(..4)` result for both digit
      validation and parsing.
- [ ] All Unicode, compound, and range-boundary tests pass.
- [ ] Plan 011's reviewed freshness behavior and tests remain intact.
- [ ] `cargo fmt --check`, `dprint check`, clippy, full tests, release build, and
      CLI smoke all exit 0.
- [ ] `git diff --name-only` contains only `src/audit.rs` and the plan/index
      status update relative to the plan-011 base.
- [ ] `plans/README.md` marks plan 018 DONE, unless the dispatcher owns the index.

## STOP conditions

Stop and report back instead of improvising if:

- Plan 011 is not reviewed and DONE on the branch base.
- `has_year_suffix` or its accepted compound-suffix contract differs from the
  excerpt after reconciling expected plan-011 changes.
- The fix appears to require modifying freshness, directory normalization,
  issue persistence, or accepting non-ASCII numeric characters.
- Any plan-011 audit regression fails after this small parser change.
- A verification command fails twice for a reason unrelated to in-scope work.

## Maintenance notes

- Rust string ranges are byte offsets and must be obtained with checked slicing
  whenever arbitrary filesystem Unicode can reach them.
- Reviewers should retain a test whose fourth byte is genuinely inside a
  multibyte code point; a Unicode example with an accidental boundary will not
  exercise this bug.
- If year parsing later expands beyond an ASCII four-digit prefix, replace this
  helper under a separately specified policy rather than broadening this fix.
- Audit freshness from plan 011 is a dependency invariant and must remain
  covered after conflict resolution.
