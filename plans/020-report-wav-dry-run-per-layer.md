# Plan 020: Report WAV dry-run changes per tag layer

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plan 019 is reviewed and marked `DONE` in `plans/README.md`.
> 2. Start from that reviewed result (which also includes Plan 003), then run:
>
> ```bash
> git diff --stat e6eb382..HEAD -- \
>   src/tags.rs src/cli/tags.rs src/tools/tests.rs \
>   site/src/content/docs/cli/index.mdx \
>   site/src/content/docs/mcp-tools/files-system.mdx
> git diff e6eb382..HEAD -- \
>   src/tags.rs src/cli/tags.rs src/tools/tests.rs
> ```
>
> Plan 019 is expected to change `src/tags.rs`, `src/cli/tags.rs`,
> `src/tools/tests.rs`, and both listed docs; reconcile and preserve its
> fallible cover-art picture-type validation plus Clap/schema help parity. Its
> Plan 003 prerequisite may also have changed handler tests.
> Unrelated drift, an existing per-layer dry-run response, or removal/renaming
> of the legacy `changes` field is a STOP condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: `plans/019-validate-cover-art-picture-types.md`
- **Category**: correctness / API observability
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

A WAV write can target ID3v2 and RIFF INFO together, but dry-run compares only
against ID3v2 unless RIFF INFO is the sole target. If the two layers have
different existing values, the preview hides the RIFF mutation (including its
layer-specific old value and comment merge result). Operators cannot verify
the exact dual-layer write before applying it.

## Current state

Current `src/tags.rs:242-258` exposes one undifferentiated changes map:

```rust
/// A single field change in a dry-run result.
#[derive(Debug, Serialize)]
pub struct DryRunChange {
    pub old: Option<String>,
    pub new: Option<String>,
}

/// Dry-run result for a single file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum FileDryRunResult {
    Preview {
        path: String,
        status: String,
        changes: HashMap<String, DryRunChange>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wav_targets: Option<Vec<String>>,
    },
```

Current `src/tags.rs:975-1000` chooses only one source layer:

```rust
// For the dry-run diff, read from the tag layer that will be written.
// WAV with riff_info-only target: diff against RIFF INFO.
// WAV with id3v2-only or both: diff against ID3v2.
// Non-WAV: use primary tag.
let riff_only = is_wav && wav_targets.len() == 1 && wav_targets[0] == WavTarget::RiffInfo;
let primary_tag = if is_wav {
    if riff_only {
        tagged_file.tag(TagType::RiffInfo)
    } else {
        tagged_file.tag(TagType::Id3v2)
    }
} else {
    tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag())
};
let mut changes = HashMap::new();

for (field, new_value) in &entry.tags {
    if riff_only && !is_riff_info_field(field) {
        continue;
    }
```

The loop then calculates comment append/prepend from that single layer's old
value. For both-layer writes, RIFF INFO differences are never returned.

Current `src/cli/tags.rs:285-314` prints only `changes`, followed by a list of
targets:

```rust
tags::FileDryRunResult::Preview {
    path,
    changes,
    wav_targets,
    ..
} => {
    tracing::info!("=== {} (dry run) ===", path);
    if changes.is_empty() {
        println!("No changes.");
        return;
    }
    for &field in tags::ALL_FIELDS {
        if let Some(change) = changes.get(field) {
            // ...
        }
    }
    if let Some(targets) = wav_targets {
        println!("WAV targets: {}", targets.join(", "));
    }
```

The only focused WAV preview test is RIFF-only and asserts that unsupported
fields are omitted; there is no dual-layer oracle.

## Target response contract

Add this optional field to `FileDryRunResult::Preview`:

```rust
#[serde(skip_serializing_if = "Option::is_none")]
changes_by_layer: Option<BTreeMap<String, BTreeMap<String, DryRunChange>>>,
```

Contract:

- Non-WAV previews omit `changes_by_layer` and preserve `changes` byte-for-byte
  in meaning.
- WAV previews always include `changes_by_layer`, even when all requested
  layer maps are empty.
- It contains exactly the requested/effective targets, keyed `id3v2` and/or
  `riff_info`; default-both contains both.
- Each layer compares against that layer's own old values. Comment
  prepend/append is computed independently from that layer's old comment.
- RIFF INFO omits fields it cannot write; ID3v2 retains them.
- The legacy top-level `changes` field remains for compatibility and preserves
  current semantics: RIFF-only mirrors `riff_info`; ID3-only or both mirrors
  `id3v2`; non-WAV remains the primary-tag diff.
- New maps use `BTreeMap` at both levels for deterministic JSON and tests.
- Human CLI output prints per-layer sections for WAV and does not print the
  legacy map a second time.

This is additive. Do not rename/remove `changes`, change `status`, or alter
write behavior.

## Commands you will need

- Focused dry-run tests: `cargo test -p reklawdbox wav_dry_run -- --nocapture` —
  per-layer cases pass.
- MCP response tests: `cargo test -p reklawdbox dry_run_response` — additive JSON
  shape passes.
- Plan 019 regression: `cargo test -p reklawdbox picture_type` — validation remains
  correct.
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
- `src/tools/tests.rs`
- `site/src/content/docs/cli/index.mdx`
- `site/src/content/docs/mcp-tools/files-system.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Actual WAV write order, copy/write/rename atomicity, or target defaults.
- Removing, renaming, or repurposing the legacy `changes` field.
- Changing non-WAV dry-run output or write behavior.
- Adding fields to input parameters or changing `wav_targets` parsing.
- Cover-art validation delivered by Plan 019 or keyed embed locks from Plan
  003.
- Reading either layer from Rekordbox `master.db`; previews inspect audio files
  only and the database remains read-only.

## Git workflow

- Branch: `codex/020-report-wav-dry-run-per-layer`
- Use Conventional Commits; preferred final message:
  `fix(tags): report wav dry runs per layer`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Build a synthetic dual-layer regression fixture

In `src/tags.rs` tests, factor the existing minimal WAV byte builder into a
small reusable helper. Use the production tag-writing path to give the same
temporary WAV different initial values in ID3v2 and RIFF INFO, one layer at a
time. Do not depend on local/private audio files.

Create focused tests named with the `wav_dry_run` prefix:

1. default-both preview where ID3v2 artist is `ID3 old`, RIFF artist is
   `RIFF old`, and the requested artist is `new`; assert two distinct old
   values under the correct layer;
2. append/prepend comment where each layer has a different old comment; assert
   independent effective new values;
3. a field supported only by ID3v2 (for example `key`); assert it exists only
   under `id3v2`;
4. ID3-only and RIFF-only requests contain exactly one layer key;
5. no-op values still produce both requested keys with empty maps;
6. a non-WAV temporary synthetic fixture has no `changes_by_layer` and keeps
   its legacy map.

Also serialize results and assert the JSON keys. Avoid comparing a whole JSON
string; compare parsed values/maps so unrelated field ordering is immaterial.

**Verify**: `cargo test -p reklawdbox wav_dry_run -- --nocapture` → dual-layer
tests fail against the current single-map output; the existing RIFF-only
unsupported-field test stays green.

### Step 2: Extract one layer-aware diff function

In `src/tags.rs`, add a private helper that accepts the entry, an optional tag
reference, and whether RIFF INFO field restrictions apply. It returns a
deterministic `BTreeMap<String, DryRunChange>` and must reuse the current rules:

- empty string and `None` mean deletion;
- absent tags produce `old: None`;
- replace/prepend/append comments use the supplied layer's old value;
- equal old/effective-new values are omitted;
- unsupported RIFF INFO fields are skipped, not reported as changes.

Derive only the traits needed to clone/compare `DryRunChange` in tests and when
constructing the compatibility map. Do not duplicate merge logic between
layers.

**Verify**: add helper-level tests for deletion, equality, comment modes,
missing tag, and RIFF filtering; then run
`cargo test -p reklawdbox dry_run_layer_diff` → all pass.

### Step 3: Produce additive per-layer WAV output

For WAV input, resolve effective `wav_targets` exactly as today. Iterate those
targets once and call the helper with:

- `TagType::Id3v2`, unrestricted fields, key `id3v2`;
- `TagType::RiffInfo`, RIFF restrictions, key `riff_info`.

Insert each requested target into an outer `BTreeMap` even when its diff is
empty. Convert/clone the selected layer map into the existing top-level
`HashMap` using the compatibility rule above. For non-WAV, call the same helper
on the primary/first tag, convert to the existing `HashMap`, and set
`changes_by_layer` to `None`.

Do not infer values from the other layer when a tag is absent. Do not merge
the two maps: the whole point is to expose their independent pre-write state.

**Verify**:

```bash
cargo test -p reklawdbox wav_dry_run
cargo test -p reklawdbox dry_run_riff_only_excludes_unsupported_fields
```

Expected: both pass; legacy `changes` assertions for RIFF-only/default-both
retain current meaning.

### Step 4: Render layer-aware human CLI previews

Update `print_dry_run_human` so:

- `Some(changes_by_layer)` prints headings in stable target order (`ID3v2`,
  then `RIFF INFO`) and the canonical `ALL_FIELDS` order within each map;
- an empty layer prints an indented `No changes.` under its heading;
- the legacy `changes` map is not also printed for WAV;
- `WAV targets: ...` is still printed even when every layer is a no-op;
- `None` preserves current non-WAV output.

Extract the field-map rendering into a small function accepting an
`std::io::Write` sink so unit tests can assert headings, different old values,
field order, empty-layer text, and absence of duplicate rows without capturing
process stdout. Production passes a locked stdout or an equivalent writer.

**Verify**: `cargo test -p reklawdbox cli_wav_dry_run` → human-output tests
pass and each changed field appears exactly once per requested layer.

### Step 5: Verify MCP JSON and document compatibility

In `src/tools/tests.rs`, call `write_file_tags` with `dry_run: true` on the
synthetic dual-layer WAV and parse the returned tool JSON. Assert:

- both per-layer maps and distinct old values are present;
- legacy `changes` equals the ID3v2 map for default-both;
- `wav_targets` remains unchanged;
- the summary reports a preview and no file was mutated.

Re-read the audio layers after the call to prove dry-run remains side-effect
free. Reconcile these tests with reviewed Plan 019/003 additions rather than
replacing them.

Update `site/src/content/docs/mcp-tools/files-system.mdx` with a compact dry-run
response example and the compatibility rule. Update the `--dry-run` prose in
`site/src/content/docs/cli/index.mdx` to describe per-layer WAV headings. Run
`docs/workflows/doc-drift/README.md`; if it requires a public file outside
scope, stop and report before editing it.

**Verify**:

```bash
cargo test -p reklawdbox dry_run_response -- --nocapture
dprint check
(cd site && npm ci && npm run build)
```

Expected: MCP JSON assertions and docs build pass; docs explicitly say the
legacy field remains.

### Step 6: Run the full gate and inspect the diff

Run every command in the command table, then:

```bash
git diff --check
git diff -- \
  src/tags.rs src/cli/tags.rs src/tools/tests.rs \
  site/src/content/docs/cli/index.mdx \
  site/src/content/docs/mcp-tools/files-system.mdx
git status --short
```

Expected: only allowed files, this plan, and the permitted README status row
are changed; `changes` is still serialized and Plan 019 validation remains.

## Test plan

- Unit: per-layer diff helper semantics and deterministic map ordering.
- Synthetic WAV: divergent ID3v2/RIFF values, comment merge, unsupported
  fields, target subsets, no-op layers, and legacy compatibility map.
- CLI: stable per-layer headings/order, empty-layer message, no duplicate rows.
- MCP: additive JSON shape, legacy field, unchanged `wav_targets`, no mutation.
- Dependency: Plan 019 picture-type and Plan 003 keyed-lock tests.
- Repository/docs: fmt, clippy, full crate tests, dprint, doc drift, site build.

## Machine-checkable done criteria

- [ ] Plan 019 is reviewed `DONE`; Plan 019/003 regression tests pass.
- [ ] Every WAV preview serializes `changes_by_layer` with exactly its effective
      requested targets, including empty maps.
- [ ] Dual-layer previews report each layer's own old and comment-merged values.
- [ ] RIFF INFO omits unsupported fields; ID3v2 retains them.
- [ ] Legacy `changes` semantics are unchanged and covered for both/one target.
- [ ] Non-WAV output omits `changes_by_layer` and is behaviorally unchanged.
- [ ] Human CLI prints stable per-layer sections without duplicate legacy rows.
- [ ] MCP dry-run tests prove additive JSON and no audio-file mutation.
- [ ] Focused/full tests, dependency regressions, fmt, clippy, dprint, and docs
      build all exit 0.
- [ ] `git diff --check` is clean and the final diff stays within scope.

## STOP conditions

Stop and report if:

- Plan 019 is not reviewed `DONE`, or its picture-type/Plan 003 changes cannot
  be reconciled without altering their contracts.
- Existing consumers require the serialized `Preview` shape to reject unknown
  fields (the additive field then needs a versioned API decision).
- Preserving the legacy `changes` semantics conflicts with a published contract
  or an existing test indicates different current behavior.
- The installed `lofty` version cannot construct/read independent ID3v2 and
  RIFF INFO layers in a synthetic temporary WAV.
- The doc-drift workflow requires files outside scope.
- The change would modify real write order/atomicity, non-WAV behavior, input
  parameters, cover-art validation, or Rekordbox `master.db`.

## Maintenance notes

- Treat `changes_by_layer` as the authoritative WAV preview; keep `changes`
  only for compatibility until a separately versioned removal is approved.
- Add future WAV tag layers through the same diff helper, deterministic target
  ordering, docs, CLI renderer, and synthetic layer-divergence tests.
- Keep dry-run computation pure after reading tags; it must never acquire
  mutation locks or call a save path.
- Audio-file writes stay coordinated by existing write mechanics, Rekordbox
  `master.db` stays read-only, and user-visible Rekordbox metadata continues
  through `ChangeManager` and XML export.
