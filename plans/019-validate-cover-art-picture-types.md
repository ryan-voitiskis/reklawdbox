# Plan 019: Validate cover-art picture types

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plan 003 is reviewed and marked `DONE` in `plans/README.md`.
> 2. Start from that reviewed result, then run:
>
> ```bash
> git diff --stat e6eb382..HEAD -- \
>   src/tags.rs src/cli/tags.rs src/tools/file_tag_handlers.rs src/tools/params.rs \
>   src/tools/tests.rs site/src/content/docs/cli/index.mdx \
>   site/src/content/docs/mcp-tools/files-system.mdx
> git diff e6eb382..HEAD -- \
>   src/tags.rs src/cli/tags.rs src/tools/file_tag_handlers.rs src/tools/params.rs \
>   src/tools/tests.rs
> ```
>
> Plan 003 is expected to change `src/tools/file_tag_handlers.rs`,
> `src/tools/tests.rs`, and server state. Reconcile and preserve its canonical
> path-keyed audio-file locks, duplicate grouping, result ordering, and
> eight-file concurrency. Unrelated drift, an existing fallible picture-type
> parser, or a changed public naming convention is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: `plans/003-serialize-audio-file-mutations.md`
- **Category**: validation bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

An unknown cover-art `picture_type` is silently treated as `front_cover`.
Misspellings can therefore extract the wrong image or replace front-cover art
when the caller intended another slot. Reject unknown names at the input
boundary, before any image/audio file I/O, while retaining every currently
documented/recognized value and alias.

## Current state

Current `src/tags.rs:395-420` has an infallible parser whose catch-all changes
the caller's meaning:

```rust
/// Defaults to `CoverFront` for unrecognised names.
pub fn parse_picture_type(name: &str) -> PictureType {
    match name {
        "other" => PictureType::Other,
        "icon" => PictureType::Icon,
        "other_icon" => PictureType::OtherIcon,
        "front_cover" | "cover_front" => PictureType::CoverFront,
        "back_cover" | "cover_back" => PictureType::CoverBack,
        // ...
        "publisher_logo" => PictureType::PublisherLogo,
        _ => PictureType::CoverFront,
    }
}
```

Current `src/tags.rs:1050-1069` parses before reading, but cannot report an
invalid name. For a valid type that is absent, it deliberately falls back to
the first embedded image:

```rust
let path_str = path.display().to_string();
let pic_type = parse_picture_type(picture_type);

let tagged_file = Probe::open(path)
    .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
    .options(parse_options(true))
    .read()
    .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

// ...
let picture = tag
    .pictures()
    .iter()
    .find(|p| p.pic_type() == pic_type)
    .or_else(|| tag.pictures().first())
```

Current `src/tags.rs:1130-1139` likewise turns an unknown embed type into a
front-cover mutation:

```rust
fn embed_cover_art_inner(
    image_path: &Path,
    target_path: &Path,
    picture_type_str: &str,
) -> Result<(), TagError> {
    let pic_type = parse_picture_type(picture_type_str);

    let image_data =
        fs::read(image_path).map_err(|e| TagError::Io(format!("Failed to read image: {e}")))?;
```

Current tests codify the unsafe fallback:

```rust
#[test]
fn parse_picture_type_default() {
    assert_eq!(parse_picture_type("garbage"), PictureType::CoverFront);
}
```

The MCP parameter descriptions currently say only "Which art"/"Picture type"
and the Clap help in `src/cli/tags.rs` says only "Picture type (default:
front_cover)"; neither enumerates accepted spellings. Plan 003 may restructure
the embed handler around canonical keyed locks; validation must happen before
that work is scheduled and must not weaken those locks.

## Target contract

`parse_picture_type` becomes fallible and accepts exactly these case-sensitive,
untrimmed values:

```text
other, icon, other_icon, front_cover, cover_front, back_cover, cover_back,
leaflet, media, lead_artist, artist, conductor, band, composer, lyricist,
recording_location, during_recording, during_performance, screen_capture,
bright_fish, illustration, band_logo, publisher_logo
```

`bright_fish` is included because `picture_type_name` already emits it. The
canonical names shown in errors/docs are `front_cover` and `back_cover`; the
reversed aliases remain accepted for compatibility. Empty strings, different
case, surrounding whitespace, and all other values return
`TagError::Validation` with a stable message containing the rejected value and
the accepted values. Do not echo paths or file contents in that error.

This plan does **not** change extraction's existing behavior when a valid
requested type is absent: it still falls back to the first embedded picture.

## Commands you will need

- Parser tests: `cargo test -p reklawdbox parse_picture_type` — valid and invalid
  cases pass.
- Cover-art tests: `cargo test -p reklawdbox cover_art` — core and MCP validation
  tests pass.
- Plan 003 regression: `cargo test -p reklawdbox audio_file_mutation` — keyed
  mutation-lock tests pass.
- Format: `cargo fmt --check` — no diff.
- Docs/config format: `dprint check` — exits 0.
- Lint: `cargo clippy -p reklawdbox --all-targets -- -D warnings` — no warnings.
- Full crate tests: `cargo test -p reklawdbox --no-fail-fast` — all tests pass.
- Docs build: `(cd site && npm ci && npm run build)` — locked install/build passes.
- Release build: `cargo build --release` — exits 0.
- Version smoke: `./target/release/reklawdbox --version` — version prints.
- Help smoke: `./target/release/reklawdbox --help` — help prints.

## Scope

**In scope** (the only source/docs files you may modify):

- `src/tags.rs`
- `src/cli/tags.rs`
- `src/tools/file_tag_handlers.rs`
- `src/tools/params.rs`
- `src/tools/tests.rs`
- `site/src/content/docs/cli/index.mdx`
- `site/src/content/docs/mcp-tools/files-system.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Plan 003's keyed-lock implementation or concurrency/result-order contract.
- Changing the default from `front_cover`.
- Removing `cover_front`/`cover_back` aliases, adding fuzzy matching,
  lowercasing, or trimming user input.
- Changing the valid-type-missing extraction fallback to the first picture.
- Image format detection, resizing, transcoding, tag-layer selection, or
  embed write mechanics.
- Turning the parameter into a breaking JSON enum or changing response shape.
- Direct Rekordbox writes; user-visible metadata continues through the existing
  file-tag/XML boundaries, and `master.db` remains read-only.

## Git workflow

- Branch: `codex/019-validate-cover-art-picture-types`
- Use Conventional Commits; preferred final message:
  `fix(tags): reject unknown cover art picture types`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Characterize the silent default through stable public results

Before changing the parser signature, keep the existing known-value/alias
tests and add ordering regressions through the already-fallible
`extract_cover_art` and embed entry point. With nonexistent image/audio paths,
`garbage`, `""`, `Front_Cover`, and `" front_cover "` must be expected to
return `TagError::Validation` before any I/O error. These tests compile against
the current API and fail because the silent default proceeds to filesystem I/O.
Retain a passing test proving that a valid-but-absent type still falls back to
the first embedded picture.

Do not write direct `.is_err()` assertions against `parse_picture_type` in
this step: its current return type is `PictureType`, so that would create a
compile failure rather than a behavioral red test.

**Verify**:
`cargo test -p reklawdbox cover_art_invalid_picture_type -- --nocapture` → the
new validation-before-I/O assertions fail for the intended current behavior;
existing `parse_picture_type` known-value tests remain green.

### Step 2: Make the core parser fallible and exhaustive

Change the signature to:

```rust
pub fn parse_picture_type(name: &str) -> Result<PictureType, TagError>
```

Return `Ok(...)` for the exact contract above and
`Err(TagError::Validation(...))` for the catch-all. Keep one static ordered
accepted-values list in `src/tags.rs` so parser tests and error construction
share one source. Add explicit parity tests for the generated schemars
descriptions and rendered Clap help because attribute/doc literals cannot be
generated from that runtime list. Add the missing `bright_fish` arm.

Now replace `parse_picture_type_default` with table-driven direct parser tests
that prove every accepted spelling maps to its exact `lofty::PictureType`, the
two cover aliases remain equivalent, `bright_fish` round-trips through
`picture_type_name`, all four invalid spellings return validation errors, and
the error contains the rejected value plus canonical accepted names without
asserting the whole sentence byte-for-byte.

Propagate `?` from `extract_cover_art` and `embed_cover_art_inner`. Do not use
`unwrap`, `unwrap_or(CoverFront)`, lossy normalization, or allocate on the
successful match path beyond the existing result construction.

**Verify**: `cargo test -p reklawdbox parse_picture_type` → all parser and
validation-before-I/O tests pass.

### Step 3: Validate MCP input once before any batch/file work

In `handle_extract_cover_art` and `handle_embed_cover_art`, resolve the default
then call the fallible parser before path metadata/read, semaphore acquisition,
canonical grouping, lock lookup, or task spawning. Map validation failures to
`McpError::invalid_params(..., None)`, not `internal_error` and not a per-file
result. The core functions must still validate independently for CLI callers.

For embed, validation occurs once for the whole request. An invalid value must
produce no partial target results and no target/image mutations. Integrate at
the top of the reviewed Plan 003 handler without altering its canonical path
identity, grouping, weak-lock cleanup, eight-way different-file concurrency,
or input/result ordering.

Add MCP tests using nonexistent paths to prove the stable invalid-parameters
error wins over image/path errors and that no per-file task is started. Add a
valid-alias test through each handler.

**Verify**:

```bash
cargo test -p reklawdbox cover_art_invalid_picture_type -- --nocapture
cargo test -p reklawdbox audio_file_mutation
```

Expected: invalid requests fail once as invalid params; Plan 003's alias and
concurrency tests remain green.

### Step 4: Publish the accepted values in schemas and docs

Update both `picture_type` schemars descriptions in `src/tools/params.rs` with
the default, canonical values, and the two accepted cover aliases. Keep the
Rust field type `Option<String>` to avoid a response/request compatibility
break.

Update both `picture_type` help strings in `src/cli/tags.rs` to enumerate the
same accepted values and rejection behavior. Add a module test that renders
the extract/embed Clap argument help and asserts every name from the canonical
`src/tags.rs` list appears. In `src/tools/tests.rs`, generate schemas for both
cover-art parameter structs and assert their `picture_type` descriptions also
contain every canonical/alias name. These parity tests are required because
the schemars and Clap descriptions remain separate compile-time literals.

Update both CLI rows in `site/src/content/docs/cli/index.mdx` and both MCP rows
in `site/src/content/docs/mcp-tools/files-system.mdx`. State that unknown
values are rejected and list/link the same accepted values. Do not claim that
a valid missing extraction type errors, because fallback remains unchanged.

Run the tool/parameter doc-drift workflow in
`docs/workflows/doc-drift/README.md`. If it identifies another generated or
hand-maintained public surface for these parameter descriptions, stop and
report before expanding scope.

**Verify**:

```bash
dprint check
(cd site && npm ci && npm run build)
cargo build --release
rg -n 'bright_fish|cover_front|cover_back' \
  src/cli/tags.rs \
  src/tools/params.rs \
  site/src/content/docs/cli/index.mdx \
  site/src/content/docs/mcp-tools/files-system.mdx
./target/release/reklawdbox extract-art --help | rg 'bright_fish'
./target/release/reklawdbox embed-art --help | rg 'bright_fish'
```

Expected: formatting/build and both help checks pass; CLI help, generated
schemas, and both docs surfaces include the accepted aliases/value.

### Step 5: Run the full gate and inspect the diff

Run all commands in the command table, then:

```bash
git diff --check
git diff -- \
  src/tags.rs src/cli/tags.rs src/tools/file_tag_handlers.rs src/tools/params.rs \
  src/tools/tests.rs site/src/content/docs/cli/index.mdx \
  site/src/content/docs/mcp-tools/files-system.mdx
git status --short
```

Expected: changes remain within the allowed files plus this plan/status row;
there is no silent `CoverFront` catch-all and no loss of Plan 003 locking.

## Test plan

- Unit: table-driven accepted values, aliases, round-trip, and rejected input.
- Ordering: invalid input returns validation before nonexistent-file I/O.
- MCP: invalid params once per request, no partial embed work, valid aliases.
- Contract parity: every canonical/alias value appears in both generated
  schemars descriptions and both rendered Clap help surfaces.
- Dependency regression: Plan 003 canonical lock, duplicate, cross-operation,
  weak-cleanup, concurrency, and ordering tests.
- Docs: dprint, doc-drift workflow, and production Starlight build.
- Repository: fmt, clippy, and full crate tests.

## Machine-checkable done criteria

- [ ] Plan 003 is reviewed `DONE` and all of its lock regressions pass.
- [ ] `parse_picture_type` returns `Result<PictureType, TagError>` with no
      default catch-all.
- [ ] Every listed value and alias passes; `bright_fish` round-trips.
- [ ] Unknown, empty, mixed-case, and whitespace-padded names fail validation.
- [ ] Core extract/embed validate before file I/O.
- [ ] MCP extract/embed map invalid names to invalid params before any batch
      scheduling or mutation.
- [ ] Valid-type-missing extraction fallback remains covered and unchanged.
- [ ] Generated params schemas, rendered extract/embed Clap help, and both
      CLI/MCP docs enumerate the same public contract; parity tests consume the
      canonical list from `src/tags.rs`.
- [ ] Focused/full tests, Plan 003 regressions, fmt, clippy, dprint, and docs
      build all exit 0.
- [ ] `git diff --check` is clean and the final diff stays within scope.

## STOP conditions

Stop and report if:

- Plan 003 is not reviewed `DONE`, or its handler changes cannot be reconciled
  without weakening canonical keyed mutation locks or result ordering.
- `lofty::PictureType` no longer supports one of the listed variants or exposes
  a changed naming contract.
- Existing callers depend on arbitrary unknown names becoming front cover and
  compatibility requires a deprecation period rather than immediate rejection.
- The doc-drift workflow requires changing a public surface outside scope.
- Tests would need real audio/image assets rather than temporary synthetic
  fixtures.
- The change would alter valid-type-missing fallback, introduce fuzzy parsing,
  change JSON response shape, or write Rekordbox `master.db`.

## Maintenance notes

- Add future accepted values to the parser, reverse name mapping, tests,
  schemars descriptions, Clap help, and CLI/MCP docs in the same change; the
  schema/help parity tests must fail until every public literal is updated.
- Preserve validation-before-I/O so invalid requests are side-effect free and
  produce deterministic errors independent of filesystem state.
- Keep Plan 003's canonical per-file lock as the single mutation coordination
  mechanism for embed operations.
- This validation does not authorize direct Rekordbox writes: local file-tag
  writes remain the explicit tool action, `master.db` stays read-only, and
  Rekordbox metadata staging/export stays behind `ChangeManager` and XML.
