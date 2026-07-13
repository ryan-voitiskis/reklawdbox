# Plan 028: Make bounded batch workflows convergent and resumable

> **Executor instructions**: Follow this plan in order, beginning with failing
> regression tests. Confirm each expected result. If a STOP condition occurs,
> report it rather than changing public semantics ad hoc. Update this plan's
> row in `plans/README.md` only if the orchestrator/reviewer does not own the
> index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- src/tools/mod.rs src/tools/resolve.rs src/tools/enrich_handlers.rs src/tools/audio_handlers.rs src/tools/label_handlers.rs src/tools/health_handlers.rs src/tools/params.rs src/db.rs src/store.rs src/tools/tests.rs site/src/data/workflows.mjs site/src/partials/sops site/src/content/docs/mcp-tools scripts/check-doc-contract.mjs scripts/check-doc-contract.test.mjs
> ```
>
> Plans 023 and 027 must be integrated. Reconfirm selector ordering, cache
> freshness semantics, public schemas, and the docs checker before coding.
> Removing or renaming public fields, using remote requests to choose a page,
> or writing to `master.db` is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: 023, 027
- **Category**: bug / API / docs
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

`enrich_tracks` and `analyze_audio_batch` currently apply `max_tracks` and
`offset` before checking `skip_cached`. A bounded repeated call can keep
examining an already cached first page and never reach pending tracks later in
the selector. Label conflicts and duplicate groups also truncate without a
complete continuation contract. SOPs tell agents to repeat work, but repetition
does not guarantee coverage and can repeat side effects.

The fix must make progress observable and deterministic while preserving the
existing one-shot API shape and the read-only Rekordbox database boundary.

## Current state

- `src/tools/enrich_handlers.rs:918-943` resolves a bounded page before cache
  checks later in the handler.
- `src/tools/audio_handlers.rs:337-358` does the same; per-track cache decisions
  occur inside analysis processing.
- `src/tools/resolve.rs` is the shared selector layer and already preserves ID,
  playlist, and search ordering after Plan 014.
- Search order currently uses only `c.Title`, and playlist order only
  `sp.TrackNo`; equal values need stable track-ID tie-breakers before offsets
  can be a deterministic public cursor.
- `src/store.rs:363-369` exposes batch enrichment-existence checks;
  `src/store.rs:652-727` exposes current audio-analysis existence/freshness
  checks. Reuse them; do not add N+1 selection queries.
- `src/tools/label_handlers.rs:13-23` defaults conflict output to 50 and claims
  `search_tracks` can page the rest, but that tool cannot filter conflicts.
- `src/tools/label_handlers.rs:348-373` truncates conflicts and reports only a
  boolean.
- `src/tools/params.rs:1033-1040` gives duplicate scanning `limit` but no
  `offset`.
- Metadata duplicates are limited in `src/db.rs:1144-1174`; the handler reports
  the returned page as the group count. Exact duplicates use hash-map iteration
  without a stable public page order.
- Embedded metadata/classification/health SOPs repeat bounded calls without a
  reliable cursor.
- Affected tools currently advertise input schemas only and return JSON as text
  in `CallToolResult`; Plan 027 therefore has no live `outputSchema` from which
  to verify new response fields.

## Canonical continuation contract

For enrichment and audio batch responses, add an additive `page` object:

```text
matched_tracks          total tracks in the logical selector at call time
start_offset            underlying selector index supplied by the caller
examined_tracks         candidates inspected from start_offset
selected_tracks         pending candidates admitted under max_tracks
fully_cached_skipped    candidates skipped as complete/current
next_offset             underlying index after the last examined candidate, or null
has_more                whether an unexamined underlying candidate remains
```

Rules:

1. `offset` remains an index into the stable underlying selector order, which
   preserves compatibility with existing callers.
2. Starting at `offset`, inspect cached/fresh state until `max_tracks` pending
   tracks are selected or the logical scope ends.
3. Do not spend a work slot on a fully completed candidate.
4. `next_offset` is the underlying position after the last inspected candidate.
5. A repeated call from offset zero normally advances because successes become
   cached; a failure remains pending, so the caller uses `next_offset` to finish
   the rest and retries failed IDs separately.
6. Existing summary fields remain and keep their current page-scoped meaning.
7. Offsets are only stable while the underlying library scope/order is
   unchanged; document restart-at-zero behavior after scope mutation.

`max_tracks=0` preserves the existing inspection-free no-op without creating a
cursor loop: `matched_tracks` still reports the logical scope,
`examined_tracks`, `selected_tracks`, and `fully_cached_skipped` are zero,
`next_offset` is null, and `has_more` is false. Documentation must tell callers
to use a positive cap for traversal.

The other pagination shapes are exact and always present on successful
responses:

```text
backfill_labels.conflict_page = {
  total, returned, offset, next_offset, has_more
}

scan_duplicates.page = {
  total, returned, offset, next_offset, has_more
}
```

For positive page sizes, `returned` is the emitted array length,
`has_more = offset + returned < total`, and `next_offset` is
`offset + returned` only when `has_more`, otherwise null. An offset beyond the
end returns zero with null/false. A zero page size follows the same terminal
no-op convention as `max_tracks=0`. Preserve `group_count` as the returned
duplicate-page count and `total_duplicate_tracks` as page-scoped.
`conflicts_truncated` remains the legacy optional alias: it is present and true
exactly when `conflict_page.has_more` is true, and absent otherwise.

These four affected tools (`enrich_tracks`, `analyze_audio_batch`,
`backfill_labels`, and `scan_duplicates`) must gain typed success payloads that
derive `Serialize` and `JsonSchema`. Their tool entrypoints must return
`rmcp::handler::server::wrapper::Json<T>` or an equivalent explicit
`outputSchema` plus matching `structuredContent`. Preserve the current JSON text
in `content` for compatibility. The live `tools/list.outputSchema` and call
result `structuredContent` become the response contract; do not create a
hand-maintained response-schema mirror.

## Commands you will need

| Purpose           | Command                                                                                              | Expected on success                      |
| ----------------- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| Page helper tests | `cargo test -p reklawdbox pending_batch_page -- --nocapture`                                         | exit 0; cached/pending/offset cases pass |
| Enrichment        | `cargo test -p reklawdbox enrich_tracks -- --nocapture`                                              | exit 0; no network-dependent tests       |
| Audio             | `cargo test -p reklawdbox analyze_audio_batch -- --nocapture`                                        | exit 0; synthetic/temp fixtures only     |
| Labels            | `cargo test -p reklawdbox backfill_labels -- --nocapture`                                            | exit 0                                   |
| Duplicates        | `cargo test -p reklawdbox scan_duplicates -- --nocapture`                                            | exit 0                                   |
| Contract          | `node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist`           | exit 0 after build                       |
| Full crate        | `cargo clippy -p reklawdbox --all-targets -- -D warnings && cargo test -p reklawdbox --no-fail-fast` | exit 0                                   |

## Scope

**In scope**:

- `src/tools/mod.rs`
- `src/tools/resolve.rs`
- `src/tools/enrich_handlers.rs`
- `src/tools/audio_handlers.rs`
- `src/tools/label_handlers.rs`
- `src/tools/health_handlers.rs`
- `src/tools/params.rs`
- `src/db.rs`
- `src/store.rs` only for batched read helpers needed by the selected contract
- `src/tools/tests.rs`
- `site/src/data/workflows.mjs`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `site/src/content/docs/mcp-tools/classification-staging.mdx`
- `site/src/content/docs/mcp-tools/files-system.mdx`
- `site/src/partials/sops/metadata-backfill.mdx`
- `site/src/partials/sops/genre-classification.mdx`
- `site/src/partials/sops/genre-audit.mdx`
- `site/src/partials/sops/library-health.mdx`
- `scripts/check-doc-contract.mjs`
- `scripts/check-doc-contract.test.mjs`
- `plans/README.md` for the status row only

**Out of scope**:

- CLI `hydrate`/analysis behavior; their discovery loops are separate.
- Changing provider/readiness definitions established by Plan 023.
- Persisted jobs, opaque server cursors, background queues, or failure caches.
- Calling provider networks merely to determine pending eligibility.
- Private audio or live-provider tests.
- Reworking classification review pagination that already has an explicit
  offset unless a regression proves it shares this defect.
- Direct writes to Rekordbox `master.db`.

## Git workflow

- Branch: `codex/028-make-batch-workflows-convergent`
- Preferred commit: `fix(batch): make bounded workflows resumable`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Characterize stable selector pagination

Add failing pure tests around the current selector order and a proposed pending
page helper. Cover:

- interleaved complete and pending candidates;
- an all-complete prefix larger than `max_tracks`;
- offset before, inside, and beyond the scope;
- zero limit behavior with the exact terminal no-op metadata above;
- caller ID order, playlist order, and search result order;
- equal-title search rows and equal-position playlist rows, with stable track-ID
  tie-breakers across repeated pages;
- `usize`/`u32` bounds and the existing 200-item cap;
- a failed first pending item plus a later pending item reachable through
  `next_offset`.

Tests must assert every page metadata field, not only selected IDs.

**Verify**:

```bash
if cargo test -p reklawdbox pending_batch_page -- --nocapture; then
  printf 'expected the new characterization tests to fail before implementation\n' >&2
  exit 1
fi
```

Expected at this step: at least one named `pending_batch_page` test runs and
fails on the cached-prefix/current pagination behavior. Zero tests or a compile
error is not an acceptable red result.

### Step 2: Add a reusable ordered pending-page selector

Separate unpaginated logical candidate resolution from work-page selection in
`src/tools/resolve.rs`. Add a generic helper that accepts ordered candidates
and a batched/local completion predicate, then returns selected IDs plus page
metadata. It must not sort internally.

Make DB-backed order deterministic before exposing it as a cursor: search
queries order by title then stable track ID, and playlist queries order by
playlist position then stable track ID. Preserve explicit caller-ID order.
Add equal-key DB regressions and prove adjacent pages neither overlap nor
reorder across identical calls.

Avoid loading unbounded rich track payloads when only IDs/cache keys are needed;
reuse the existing maximum logical scope or stream/page DB IDs if the selector
can be huge. If safe bounded resolution cannot be achieved without a new query
architecture, STOP and split the work rather than introducing an unbounded
memory regression.

Define a reusable typed page record and typed success payloads as each affected
handler lands. Derive `Serialize` and `JsonSchema`, return them through RMCP's
structured JSON wrapper, and keep the serialized JSON text compatibility
content. Add a router/tools-list regression proving all four tools advertise a
non-null output schema and a call-result regression proving success returns
matching structured content.

**Verify**:

```bash
cargo test -p reklawdbox pending_batch_page -- --nocapture
```

Expected: exit 0; all helper tests pass, preserve input order, and report exact
examined/selected/next-offset metadata.

### Step 3: Select enrichment work after batched cache checks

For `skip_cached=true` and `force_refresh=false`:

- resolve the logical candidate order;
- batch-read completion state for every requested provider using existing
  store APIs, adding an exact batched tuple helper if required;
- consider a track pending when at least one requested provider lacks a
  completed non-error search/cache entry;
- treat a cached no-match as a completed search, not a metadata match;
- keep error entries pending;
- admit pending tracks until the work cap, preserving order.

Preselection must use the exact cache key used by execution:
`(provider, normalized_artist, normalized_title, normalized_album)`. Discogs
uses the same normalized album key as `enrich_single_track`; Beatport and
Bandcamp use the empty album key. The current artist/title-only existence helper
is not sufficient. A cached `match_quality='none'` is complete, while
`match_quality='error'` remains pending. Add a same-artist/title,
different-album Discogs regression so one release cannot hide another.

When `skip_cached=false` or `force_refresh=true`, every candidate is pending.
For partially cached multi-provider tracks, process only the missing/refresh
provider work while retaining accurate per-provider counts.

Every retryable failure must retain `track_id` and the relevant `provider`, plus
a stable failure stage. Task joins and cache-writer open/write/ack failures must
not collapse to a batch-only count. Add bounded hermetic writer-failure tests
that prove the affected track/provider can be retried explicitly.

Add hermetic fake-store/handler tests for a cached prefix, partial providers,
no-match, error, force refresh, exact album keys, identity-bearing writer
failures, and returned continuation. No provider HTTP call may be necessary to
test selection.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "enrich_tracks_pending_page"
cargo test -p reklawdbox enrich_tracks_pending_page -- --nocapture
```

Expected: the list shows the new tests and the focused run exits 0 without a
provider network call; cached-prefix, partial-provider, no-match, error, refresh,
and failure-continuation cases all pass.

### Step 4: Select audio work after freshness checks

Use the existing current Stratum identity and batch audio freshness APIs.
Pending means:

- current Stratum is missing/stale; or
- Essentia is installed/requested and its current result is missing/stale.

When Essentia is unavailable, do not make its absence keep every track pending.
Schema, file-identity, and Rekordbox-grid staleness must continue to invalidate
Stratum as defined by the integrated cache logic.

Missing/unreadable files return explicit failures and remain retryable. They
must not make later tracks unreachable; tests use `next_offset` to bypass them.
Use synthetic/temp fixtures only.

Carry `track_id`, analyzer, and a stable failure stage through queued cache
writes. Writer open/write/join failures must report the affected identities,
not only a count or general writer error. Add a bounded hermetic writer-failure
regression alongside the unreadable-file continuation case.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "analyze_audio_batch_pending_page"
cargo test -p reklawdbox analyze_audio_batch_pending_page -- --nocapture
```

Expected: the named tests exist and pass for stale/current Stratum, optional
Essentia, unreadable-file continuation, and synthetic/temp inputs only.

### Step 5: Make label conflicts independently pageable

Add an optional `conflict_offset` beside `max_conflicts`. Sort conflicts by
normalized artist, normalized title, then stable track ID. Return total,
returned, offset, next offset, and has-more metadata in the exact always-present
`conflict_page` object above. Preserve the optional legacy
`conflicts_truncated` alias with the defined equality rule and terminal
zero-page semantics.

Because the backfill call can stage changes or auto-enrich, document a safe
continuation pattern: run the intended mutating pass once, then retrieve later
conflict pages with `dry_run=true` and `auto_enrich=false`. Add tests proving a
later page does not repeat staging/provider side effects.

Remove the false suggestion that `search_tracks` can retrieve the omitted
conflicts.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "backfill_labels_conflict_page"
cargo test -p reklawdbox backfill_labels_conflict_page -- --nocapture
```

Expected: the named tests exist and pass; pages are stable/non-overlapping and
later dry-run pages do not repeat staging or auto-enrichment.

### Step 6: Make metadata and exact duplicate groups pageable

Add `offset` to `ScanDuplicatesParams`.

- Metadata mode: use deterministic ordering such as duplicate count descending,
  normalized artist/title, then a stable tie-breaker; return total group count
  separately from the page; apply offset and limit in SQL.
- Exact mode: deterministically sort hash groups before slicing; return the
  same exact always-present `page` object. It is acceptable for each request to
  rehash the selected scope; a persisted hashing job is out of scope.

Keep existing `group_count` equal to `page.returned` for compatibility. Clearly
label `total_duplicate_tracks` as page-scoped and do not present it as a
full-scope total. Apply the exact positive/zero/beyond-end formulas above. Test
non-overlap, stable order, equal-key tie-breakers, zero/beyond-end offsets, and
both duplicate modes.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "scan_duplicates_page"
cargo test -p reklawdbox scan_duplicates_page -- --nocapture
```

Expected: the named tests exist and pass for metadata/exact modes, stable order,
total versus returned counts, non-overlap, and beyond-end offsets.

### Step 7: Rewrite SOP loops and public references

Replace “repeat the same bounded call until 100%” with an explicit loop:

1. call from the current offset;
2. record successes and failures;
3. continue with `next_offset` while `has_more`;
4. retry failed explicit track IDs after traversing the scope;
5. stop only when workflow-specific readiness from Plan 023 is satisfied or
   unresolved work is reported.

Document label-conflict and duplicate paging. Preserve searched-versus-matched
terminology. Do not describe these offsets as durable across library changes.

Update Plan 027's structural fixtures/markers so every new parameter and page
field is required in the reference. Add narrow `doc-contract:mcp-output`
markers for the four affected success payloads and extend the checker to compare
only those marked response surfaces to live DB-free `tools/list.outputSchema`.
It must fail when a live response property is omitted; it must not call these
DB-dependent tools to obtain a success result. The focused Rust
`batch_output_schema_contract` router tests exclusively verify that actual
successes carry matching `structuredContent` plus preserved JSON text. Do not
infer a schema from prose or duplicate the Rust success types in Node.

Update the affected canonical workflow records in
`site/src/data/workflows.mjs` so `resumability` describes the integrated
continuation fields, failed-ID retry, and offset invalidation after scope/order
changes. Preserve every unrelated Plan 026 fact and re-run its validator.

**Verify**:

```bash
rg -n -e "next_offset" -e "has_more" -e "conflict_offset" site/src/partials/sops site/src/content/docs/mcp-tools
cargo test -p reklawdbox batch_output_schema_contract -- --nocapture
node --test scripts/check-doc-contract.test.mjs
```

Expected: all affected SOP/reference surfaces name the continuation fields,
the four live output schemas/structured results pass, and the contract fixtures
pass, including a negative case for an omitted field.

### Step 8: Run doc drift and the full gate

Run focused tests first, then formatting, clippy, complete crate tests, release
build, MCP smoke, site build, and the live docs checker. Because SOP partials
are embedded, the release build must precede MCP help validation.

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
cd site && npm run build && cd ..
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: every command exits 0; embedded help exposes the implemented
continuation contract and the live documentation gate has no schema drift.

## Test plan

- Pure helper tests establish deterministic pagination independent of network
  and DB fixtures.
- Handler tests cover cached prefixes, partial providers, stale audio, failures,
  no-match, refresh, zero/beyond offsets, and page metadata.
- Label tests prove continuation retrieval cannot repeat mutations.
- Duplicate tests prove complete, stable, non-overlapping pages in both modes.
- Existing fields remain present; new fields are additive and contract-checked.
- Live output schemas/structured content are the response-field oracle; no Node
  response-schema snapshot is added.
- Every failure needed for explicit retry carries track/analyzer/provider
  identity, including cache-writer failures.
- Full tests use no private audio or external provider.

## Done criteria

- [ ] Fully cached first pages cannot hide later pending work.
- [ ] Every batch response exposes deterministic progress/continuation metadata.
- [ ] A failure can be bypassed via `next_offset` and retried by explicit ID.
- [ ] Multi-provider and optional-Essentia pending semantics are correct.
- [ ] Every label conflict and duplicate group is reachable with bounded pages.
- [ ] Existing response fields remain compatible and documented.
- [ ] The four affected tools advertise typed output schemas and return matching structured content.
- [ ] Stable selector tie-breakers and exact provider cache keys prevent false skips/reordered pages.
- [ ] Canonical workflow resumability records reflect the integrated continuation contract.
- [ ] Embedded SOPs traverse continuations and report unresolved failures.
- [ ] Focused tests, full Rust gates, release/MCP smoke, site build, docs checker, and format pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- Integrated selector order differs materially from the evidence above.
- Correct selection requires a remote request or private data.
- The fix would remove/rename existing fields or redefine offset silently.
- Logical candidate resolution creates an unsafe unbounded memory/query path.
- Label pagination cannot avoid repeating staging or auto-enrichment side effects.
- Exact duplicate pagination requires a persisted-job subsystem to be correct.
- Any approach writes to `master.db` or weakens current cache freshness rules.

## Maintenance notes

- New bounded `skip_cached` tools should reuse the ordered pending-page helper.
- Offset stability is scoped to an unchanged selector and library order.
- Keep failures retryable and traversal progress separate.
- Plan 027 must fail whenever a continuation parameter/field is omitted from
  public reference material.
