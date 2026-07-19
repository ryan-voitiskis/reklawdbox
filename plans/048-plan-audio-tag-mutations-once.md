# Plan 048: Plan audio-tag mutations once

> **Executor instructions**: Follow this plan step by step. Preserve every
> serialized field, CLI string, MCP schema, file-mutation lock, dry-run rule,
> and atomic WAV behavior. STOP rather than changing the public contract or
> weakening a safety regression. Update the tracker only after independent
> review and full verification.
>
> **Dependency and drift check (run first)**:
>
> 1. Confirm Plan 047 is reviewed `DONE` and start from its integrated head;
>    both plans touch the shared audio test/module surfaces.
> 2. Run:
>
> ```bash
> git diff --stat b2155e573d0a87be1eab98f09dca5afa3dfb7774..HEAD -- \
>   src/adapters/audio/tags.rs \
>   src/adapters/audio/mod.rs \
>   src/application/files/tags.rs \
>   src/application/audit/scan.rs \
>   src/application/metadata/backfill.rs \
>   src/cli/tags.rs \
>   src/mcp/files \
>   src/mcp/tests/files.rs \
>   site/src/content/docs/mcp-tools/files-system.mdx
> ```
>
> Reconcile reviewed Plan 047 test-support changes. STOP if the public tag
> fields, WAV layer defaults, comment separator, dry-run JSON, picture-type
> vocabulary, or file-locking semantics changed after the planning commit.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: 047
- **Category**: decomplexification / file safety / internal contracts
- **Planned at**: commit `b2155e5`, 2026-07-19

## Why this matters

`src/adapters/audio/tags.rs` is 1,958 lines, of which 1,284 are production
code. Size alone is not the finding. The mutation policy is implemented twice:

- `write_tag_layer` decides RIFF capability, deletion, split-key cleanup,
  comment merging, equality, and secondary-key writes; and
- `dry_run_layer_diff` independently decides RIFF capability, deletion,
  comment merging, and equality for preview.

That duplication is safety-sensitive: preview is supposed to describe the
write. A future rule can drift in one path while tests still cover the other.
The module also accepts raw `HashMap<String, Option<String>>` field names deep
into the adapter and gives adapter enums `Deserialize`/`JsonSchema` solely so
MCP can use them directly. This keeps a stringly contract and transport schema
concern inside the I/O adapter.

The module otherwise contains real, distinct responsibilities: public result
models, field policy, tag-file I/O, atomic dual-layer WAV replacement, and
cover art. This plan extracts those ownership boundaries only after replacing
the duplicated mutation rules with one canonical plan.

## Target design

### A validated internal patch

Introduce an adapter-internal `TagField` enum covering exactly the 14 existing
canonical strings and a validated patch representation with explicit
`Set(String)` and `Delete` operations. Conversion from the existing input map
must preserve all current validation text and semantics:

- unknown fields are rejected, including null/empty values;
- `year` remains four-digit `YYYY`;
- `track` and `disc` remain positive integers;
- `None` and `Some("")` both mean delete; and
- all other values remain unmodified.

Do not change MCP or CLI input JSON. Conversion happens once before file I/O.

### One layer mutation plan

Create one pure planner that accepts:

- the validated patch;
- a layer capability (`ID3v2`, `RIFF INFO`, or a non-WAV primary tag);
- the layer's existing field values; and
- the comment merge mode.

It returns a `LayerMutationPlan` containing the effective operations and the
existing `DryRunChange` values. Both dry-run rendering and actual tag mutation
must consume this same plan. Applying a plan may perform Lofty-specific
primary/secondary key writes, but it must not recalculate policy.

The planner must preserve:

- RIFF INFO's exact supported-field set;
- per-layer comment append/prepend using each layer's old comment;
- no-op omission;
- year/BPM secondary-key deletion;
- non-Vorbis secondary-key writes;
- deterministic `changes_by_layer` ordering; and
- the legacy top-level `changes` compatibility source.

### Edge-owned transport enums

Keep adapter concepts such as WAV layer and comment merge mode, but remove
`schemars::JsonSchema` and transport deserialization from the adapter. Define
MCP parameter enums in `src/mcp/files/transport.rs` with the exact current
spellings and aliases, then convert them explicitly in the handler. CLI keeps
its current string parser and maps to the adapter types.

Do not mirror output result structs at the edge: they are workflow data already
shared by CLI/MCP. The goal is to move schema-only input types, not create a
facade for every adapter value.

### Cohesive module ownership

After the canonical planner exists and tests pass, replace `tags.rs` with:

- `tags/mod.rs` — declarations and narrow re-exports only;
- `tags/model.rs` — adapter request/result/error models;
- `tags/fields.rs` — field mapping, validation, and comment policy;
- `tags/mutation.rs` — layer planning, application, and atomic WAV replace;
- `tags/read.rs` — tag reads and cover-art metadata reads;
- `tags/art.rs` — cover-art extract/embed I/O; and
- `tags/tests/` — policy, synthetic round-trip, and art capabilities with an
  explicit support module.

This split is the final step, not the implementation strategy. If policy is
still duplicated, smaller files do not satisfy the plan.

## Scope

**In scope**:

- `src/adapters/audio/tags.rs` (replaced by the directory below)
- `src/adapters/audio/tags/mod.rs`
- `src/adapters/audio/tags/model.rs`
- `src/adapters/audio/tags/fields.rs`
- `src/adapters/audio/tags/mutation.rs`
- `src/adapters/audio/tags/read.rs`
- `src/adapters/audio/tags/art.rs`
- `src/adapters/audio/tags/tests/**`
- `src/adapters/audio/mod.rs`
- `src/application/files/tags.rs`
- `src/application/audit/scan.rs` and
  `src/application/metadata/backfill.rs` only for import/path reconciliation
- `src/cli/tags.rs`
- `src/mcp/files/transport.rs`
- `src/mcp/files/handlers.rs`
- `src/mcp/tests/files.rs`
- `src/adapters/audio/tests.rs` only for the optional private-copy matrix
- `tests/source_boundaries.rs` only if a narrowly targeted edge-type rule is
  added without a parser rewrite
- `plans/README.md` status row only during execution

**Out of scope**:

- Adding, removing, renaming, or normalizing any public tag field.
- Changing serialized result shapes, status strings, error strings, MCP
  descriptions, CLI flags/output, picture-type aliases, or WAV defaults.
- Changing file concurrency, canonical-path locking, preview/dry-run behavior,
  confirmation expectations, or any existing backup handoff.
- Changing the dual-layer copy/write/rename order or making single-layer writes
  atomic as an incidental enhancement.
- Changing Lofty, cache versions, audio-analysis schemas, Rekordbox metadata,
  genre taxonomy, or cover-art policy.
- Writing to Rekordbox `master.db` or a source private audio file.

## Steps

### Step 1: Lock the public and file-safety behavior

Before refactoring, add or retain characterization tests for:

1. MCP input schemas for `wav_targets` and `comment_mode`, including exact
   enum spellings/defaults;
2. serialized read/write/dry-run/embed results, including omitted optional
   fields and legacy WAV `changes`;
3. CLI JSON and human output for a representative success, no-op, validation
   failure, and per-layer dry-run;
4. preview/write parity for set, delete, no-op, prepend, and append;
5. dual-layer failure cleanup: the original file survives and the temp copy is
   removed;
6. mutation locking for duplicate/canonicalized paths; and
7. cover art surviving an unrelated metadata write.

Use synthetic WAV and AIFF fixtures for mandatory tests. Add small synthetic
MP3/FLAC/M4A fixtures only if Lofty can create them reliably without checked-in
binary assets; otherwise cover those formats in the opt-in private-copy matrix.

Focused baseline:

```bash
cargo test -p reklawdbox tags_ -- --nocapture
cargo test -p reklawdbox wav_dry_run -- --nocapture
cargo test -p reklawdbox file_tag_workflow -- --nocapture
cargo test -p reklawdbox cover_art -- --nocapture
```

### Step 2: Validate raw field input once

Add `TagField`, `TagEdit`, and `ValidatedTagPatch` privately. Keep the raw map
on the edge-facing `WriteEntry` only if changing it would spread conversion
through transports; the first adapter operation must convert it once and all
deeper functions must accept the validated patch.

Replace `field_to_item_key(field: &str) -> Option<_>` in mutation code with an
exhaustive mapping from `TagField`. A string conversion may remain for reads
and serialized output. Add a compile-visible/exhaustive test that every member
maps to the corresponding existing `ALL_FIELDS` string in stable order.

### Step 3: Make preview and write share one plan

Extract existing layer values into a small read-only view, then build
`LayerMutationPlan`. Make dry-run serialize its changes. Make the write path
apply the exact operations from the plan and record written/deleted fields from
those operations.

Add a table-driven parity test that, for each supported field and each layer:

- computes preview;
- applies the write to a temporary file;
- re-reads the layer; and
- asserts the actual new value equals the preview's effective new value.

For RIFF-unsupported fields, assert both preview and write omit them. For
Vorbis year/BPM behavior, assert no duplicate secondary field is introduced.

### Step 4: Move MCP-only input schema types to the MCP edge

Define transport enums with the same serde/schemars contract, map them in
`handlers.rs`, and remove `JsonSchema`/`Deserialize` from adapter-only input
types. Compare generated tool schemas before and after as parsed JSON; the
`write_file_tags` schema must be equal, not merely similar.

Add a focused architecture assertion only if it can check a simple invariant
such as `src/adapters/audio/tags/**` not importing `schemars`. Do not turn the
source-boundary scanner into a dependency parser in this plan.

### Step 5: Extract responsibility modules and test capabilities

Move already-green code into the target module structure. Keep `mod.rs` free
of implementation. Split tests by policy/round-trip/art capability with one
small fixture support module. Re-run focused tests after each move so path or
visibility edits do not mask behavioral drift.

The combined production code should have one field-validation implementation,
one effective-value planner, and one plan applier. A source search should find
no second branch implementing comment/deletion/RIFF equality rules.

### Step 6: Run the opt-in real-format matrix

When Plan 047's private fixture and accessible audio are available, select
representative WAV, FLAC, MP3, and M4A/AAC files where present. Copy every
source file to a unique temporary directory. For each copy:

1. read tags and cover-art metadata;
2. preview a reversible comment change;
3. apply it;
4. re-read and compare preview with actual state;
5. restore/remove the temporary copy; and
6. prove the source hash is unchanged.

Run with an explicit ignored-test filter. Do not require every user's library
to contain every format; report per-format coverage and `NOT AVAILABLE` where
appropriate. No private path, hash, or metadata belongs in committed output.

### Step 7: Full gate and adversarial review

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

Review the complete diff specifically for schema drift, altered error text,
lost pictures, partial WAV replacement, temp-file leaks, and accidental writes
outside temporary fixtures.

Require independent file-safety and API/schema reviews. The safety reviewer
must trace preview, lock, backup/temp, dual-layer commit, and cleanup paths;
the API reviewer must compare generated MCP schema and serialized fixtures.
Remediate every concrete finding and send the amended diff back for re-review.

## Machine-checkable done criteria

- [ ] Write and dry-run consume the same `LayerMutationPlan`; comment,
      deletion, RIFF filtering, and equality policy are not duplicated.
- [ ] Raw field strings are converted once to an exhaustive validated patch.
- [ ] MCP-only input schema enums live in `mcp/files/transport.rs`; generated
      tool schemas are unchanged.
- [ ] All existing JSON, CLI, error, WAV-target, picture-type, and result
      contracts are unchanged.
- [ ] Dual-layer WAV atomic replacement, temp cleanup, cover-art preservation,
      and canonical-path locking remain covered.
- [ ] `tags/mod.rs` is navigation only and extracted modules have distinct
      ownership rather than wrapper-only forwarding.
- [ ] Mandatory synthetic parity tests pass; any private format matrix ran only
      on temporary copies and is reported separately.
- [ ] No cache/schema bump was made because analyzer output did not change.
- [ ] Full repository, MCP, docs-contract, site, architecture, and diff gates
      pass.

## STOP conditions

Stop and report if:

- preview/write parity requires changing a published result or current write
  behavior;
- Lofty cannot preserve an existing picture or unknown tag frame through the
  planned read-modify-write path;
- dual-layer failure cleanup or canonical-path locking becomes weaker;
- moving schema enums changes generated MCP JSON;
- a safe round-trip cannot be demonstrated without writing private source
  audio;
- any format change requires an audio cache/schema version bump; or
- the work expands into a new tagging backend or broad file-mutation framework.

## Complexity accounting

Success removes duplicated mutation policy and raw-string branching from deep
write code. Module extraction then localizes the remaining I/O responsibilities.
Reduced file size without one validated patch and one mutation planner is only
movement and must be rejected.

## Git workflow

- Branch: `codex/048-plan-tag-mutations-once`
- Preferred commit: `refactor(tags): plan file mutations once`
- Do not push, merge, release, or deploy.
