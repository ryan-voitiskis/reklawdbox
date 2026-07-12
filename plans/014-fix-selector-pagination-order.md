# Plan 014: Apply selector pagination after all logical filters

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat e6eb382..HEAD -- src/db.rs src/tools/resolve.rs src/tools/library_handlers.rs src/tools/tests.rs
> ```
>
> If any file changed, compare playlist query signatures and the local
> unknown-genre filter/pagination excerpts below with live code. An existing
> shared post-filter pagination helper or changed ordering contract is a STOP
> condition.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Selector pagination is not applied to one consistent logical result set.
Playlist selection ignores `offset`; unknown-genre selection sends `offset` to
SQL before applying the Rust-only unknown-genre predicate; explicit `track_ids`
also ignore offset. Pages can repeat, skip eligible tracks, or contain fewer
items than requested even when later matching rows exist. The contract must be:
resolve the chosen selector, apply every logical post-filter, then apply offset
and limit exactly once—while retaining SQL pagination for common paths that
have no Rust post-filter.

## Current state

- `src/db.rs` owns ordered search/playlist queries and bounded/unbounded safety
  policies. Rekordbox connections opened by this crate are read-only; preserve
  that invariant.
- `src/tools/resolve.rs` is shared by `enrich_tracks`, audio analysis,
  classification, resolved-data, pool, and cache-coverage tools. Selector
  priority is `track_ids > playlist_id > search filters`.
- `src/tools/library_handlers.rs` separately implements `search_tracks` and
  repeats unknown-genre post-filtering.
- `src/tools/tests.rs` contains selector/tool parameter and handler regressions;
  `src/db.rs` contains query-level tests.

Current `src/tools/resolve.rs:57-60` recognizes only limit—not offset—as unsafe
before the unknown-genre post-filter:

```rust
// Fetch without a limit so the post-filter has the full candidate set to work
// with, then truncate to effective_max afterward.
let skip_db_limit = has_unknown_genre == Some(true) && track_ids.is_none();
```

Current `src/tools/resolve.rs:62-78` ignores `offset` for both explicit IDs and
playlists:

```rust
let tracks = if let Some(ids) = track_ids {
    db::get_tracks_by_ids(conn, ids)?
} else if let Some(pid) = playlist_id {
    if skip_db_limit {
        db::get_playlist_tracks_unbounded(conn, pid, None)?
    } else if bounded {
        db::get_playlist_tracks(conn, pid, db_limit)?
    } else {
        db::get_playlist_tracks_unbounded(conn, pid, db_limit)?
    }
```

Current `src/tools/resolve.rs:80-84` applies the raw SQL offset before the
unknown-genre predicate:

```rust
if skip_db_limit {
    let search = filters.into_search_params(true, None, offset)?;
    db::search_tracks_unbounded(conn, &search)?
}
```

The predicate is only applied later (`src/tools/resolve.rs:107-117`), followed
by truncation but no local offset:

```rust
if has_unknown_genre == Some(true) {
    tracks.retain(|t| {
        !t.genre.is_empty()
            && !genre::is_known_genre(&t.genre)
            && genre::canonical_genre_from_alias(&t.genre).is_none()
    });
}
if let Some(max) = effective_max {
    tracks.truncate(max);
}
```

Current `src/tools/library_handlers.rs:23-42` repeats the same bug:

```rust
let mut search = filters.into_search_params(
    !params.include_samples.unwrap_or(false),
    if has_unknown_genre { None } else { limit },
    params.offset,
)?;
// ... retain unknown genres ...
tracks.truncate(limit.unwrap_or(50).min(200) as usize);
```

Current `src/db.rs:419-448` supports playlist limit but has no offset:

```rust
fn get_playlist_tracks_with_limit_policy(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
    default_limit: Option<u32>,
    max_limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    // ... ORDER BY sp.TrackNo ...
    if let Some(limit) = resolved_limit {
        write!(sql, " LIMIT {limit}").unwrap();
    }
}
```

Applicable conventions:

- `db::get_tracks_by_ids` preserves caller order and deduplicates IDs; local
  pagination must happen after that behavior.
- SQL search order is `c.Title`; playlist order is `sp.TrackNo`. Do not change
  either ordering.
- Bounded ordinary tools cap at 200; unbounded variants exist for diagnostics
  and calibration. Preserve each caller's safety policy.
- SQLite requires `LIMIT` before `OFFSET`; existing search code emits
  `LIMIT -1` when offset exists without a limit. Match it for playlists.

## Commands you will need

| Purpose              | Command                                                        | Expected on success                 |
| -------------------- | -------------------------------------------------------------- | ----------------------------------- |
| DB playlist tests    | `cargo test -p reklawdbox db::tests::test_get_playlist_tracks` | exit 0; matching tests pass         |
| Resolver tests       | `cargo test -p reklawdbox selector_pagination`                 | exit 0; all new selector tests pass |
| Search handler tests | `cargo test -p reklawdbox search_tracks_unknown_genre`         | exit 0; matching tests pass         |
| Format               | `cargo fmt --check`                                            | exit 0, no diff                     |
| Docs/config format   | `dprint check`                                                 | exit 0                              |
| Lint                 | `cargo clippy -p reklawdbox --all-targets -- -D warnings`      | exit 0, no warnings                 |
| Full crate tests     | `cargo test -p reklawdbox --no-fail-fast`                      | exit 0; all tests pass              |

## Scope

**In scope** (the only source files you may modify):

- `src/db.rs`
- `src/tools/resolve.rs`
- `src/tools/library_handlers.rs`
- `src/tools/tests.rs`
- `plans/README.md` for the status row only

**Out of scope**:

- Changing selector priority (`track_ids > playlist_id > filters`).
- Cursor pagination, total-count fields, response-envelope changes, or new MCP
  parameters.
- Changing sort order, unknown-genre taxonomy semantics, or sampler path
  detection.
- Raising/removing bounded tool caps or making common searches unbounded.
- Adding offset to the standalone `get_playlist_tracks` MCP tool, whose params
  currently expose only `limit`; this plan fixes shared selectors that already
  advertise offset.
- Writes to Rekordbox or the internal cache; this is read-only selection logic.

## Git workflow

- Branch: `codex/014-fix-selector-pagination-order`
- Use Conventional Commits; preferred final message:
  `fix(selectors): paginate filtered results`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Add query- and tool-level regression fixtures

Before production changes, add tests that use deterministic in-memory SQLite
fixtures with at least four ordered candidates and interleaved canonical,
unknown, and empty genres.

Use only existing resolver/handler entry points in this red phase; do not call
the not-yet-created paged DB variants. Cover:

1. Through `resolve_tracks`, playlist `limit=1, offset=1` returns only the
   second `sp.TrackNo` item and preserves its original `position`.
2. Shared resolver playlist pages do not repeat page one.
3. Explicit IDs `[t3, t1, t2]` with offset 1/max 1 return `t1` (caller order
   after deduplication).
4. Unknown-genre search with canonical rows interleaved, offset 1/max 1,
   returns the second _unknown_ track, not the second raw SQL row.
5. Unknown-genre playlist selection has the same post-filter pagination.
6. `handle_search_tracks` applies the same semantics.
7. Offset beyond the filtered set returns an empty vector; limit zero returns
   empty; default/cap behavior remains unchanged.

Give new tests names containing `selector_pagination` (and the search handler
name requested in the Commands table) so focused commands are stable.

**Verify**: `cargo test -p reklawdbox selector_pagination -- --nocapture` → the
playlist/ID/unknown-offset regressions fail against current code for the
intended reasons.

### Step 2: Add offset-aware playlist query variants

Extend the private `get_playlist_tracks_with_limit_policy` in `src/db.rs` with
`offset: Option<u32>`. After resolving the bounded/unbounded limit:

- emit `LIMIT <n>` when present;
- if offset exists without a limit, emit `LIMIT -1`;
- then emit `OFFSET <n>`;
- preserve `ORDER BY sp.TrackNo` and row `position`.

Keep existing public wrappers source-compatible by passing `offset=None`.
Add explicit paged variants for shared resolver use, for example:

- `get_playlist_tracks_page(conn, id, limit, offset)` with default/cap 200;
- `get_playlist_tracks_unbounded_page(conn, id, limit, offset)` with no
  default/cap.

Do not simulate playlist offset by fetching the first `offset + limit` rows on
the common no-post-filter path.

After these signatures exist, add the direct DB-level tests for bounded and
unbounded limit+offset, offset without a limit, zero offset, beyond-end offset,
stable position/order, and source compatibility of the old wrappers. Do not
try to add these direct calls in Step 1, where they would only compile-fail.

**Verify**:
`cargo test -p reklawdbox db::tests::test_get_playlist_tracks` → existing order
tests plus bounded/unbounded offset, offset-without-limit, and beyond-end cases
pass.

### Step 3: Centralize local post-filter pagination

In `src/tools/resolve.rs`, add two small helpers visible to sibling tool
modules:

1. the exact unknown-genre predicate currently duplicated in resolver and
   library handler;
2. `apply_offset_limit(Vec<Track>, offset, limit)` (or iterator equivalent)
   that skips first, then takes/truncates, preserving order and handling
   `None`, zero, and beyond-end values.

Keep numeric conversion safe on 32/64-bit targets; use checked/`try_from` or
the fact that `u32` always fits the repository's supported `usize` target with
an explicit conversion. Do not sort inside this helper.

**Verify**: focused pure helper tests cover all offset/limit combinations and
unknown canonical/alias/empty/raw cases:
`cargo test -p reklawdbox selector_pagination_helpers` → all pass.

### Step 4: Apply offset exactly once in the shared resolver

Refactor `resolve_tracks` around an explicit boolean such as
`pagination_applied_in_db`. Required behavior by selector:

- **track_ids**: fetch via `get_tracks_by_ids`, apply sampler/unknown
  post-filters, then local offset/max exactly once.
- **playlist, no Rust post-filter**: call the appropriate bounded/unbounded
  paged DB function with effective max and offset; do not paginate locally.
- **playlist with unknown-genre or sampler post-filter**: fetch the full
  unbounded ordered playlist without offset/limit, apply post-filters, then
  local offset/max.
- **search, no Rust post-filter**: keep SQL limit/offset fast path.
- **search with unknown-genre post-filter**: pass both SQL limit and SQL offset
  as `None`, fetch unbounded candidates, filter, then local offset/max.

Search already expresses sample exclusion in SQL through `SearchParams`; do
not unnecessarily convert ordinary searches into full-library reads. Preserve
the `default_max_tracks` and `max_tracks_cap` calculations exactly.

Add an assertion/test that every branch applies pagination either in SQL or
locally, never both.

**Verify**: `cargo test -p reklawdbox selector_pagination` → all selector,
post-filter, priority, default, and cap regressions pass.

### Step 5: Fix standalone search unknown-genre ordering with the same helpers

In `handle_search_tracks`, when `has_unknown_genre` is true:

- pass `limit=None` and `offset=None` into `into_search_params`;
- keep playlist and sample filters in SQL;
- fetch unbounded ordered candidates;
- apply the shared unknown-genre predicate;
- call shared local offset-then-limit with default 50 and cap 200.

When `has_unknown_genre` is false, retain the current bounded SQL path. Remove
the duplicated predicate and direct `truncate`.

**Verify**:
`cargo test -p reklawdbox search_tracks_unknown_genre` → first, second, and
beyond-end unknown-genre pages return the expected IDs without gaps/repeats.

### Step 6: Run callers and the full crate gate

Because `resolve_tracks` is shared broadly, run all root tests rather than only
library tests. Review the diff to confirm common non-post-filter search remains
bounded at SQL.

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

Expected: all commands exit 0; only in-scope files changed.

## Test plan

- `src/db.rs` tests:
  - bounded playlist limit+offset;
  - unbounded offset with `LIMIT -1` behavior;
  - offset zero and beyond end;
  - stable `sp.TrackNo` order/position.
- `src/tools/tests.rs` resolver tests:
  - explicit-ID order/dedup then pagination;
  - normal playlist pages;
  - unknown-genre playlist pages;
  - unknown-genre search pages with interleaved nonmatches;
  - sampler post-filter before pagination;
  - selector priority, zero/beyond-end, defaults, and cap.
- Search-handler tests:
  - same unknown-genre page sequence as shared resolver;
  - normal search still uses expected results/default cap.
- All tests use in-memory/synthetic SQLite and perform no writes through the
  production Rekordbox connection path.

## Done criteria

- [ ] Playlist selectors honor offset in `sp.TrackNo` order.
- [ ] Explicit track IDs honor offset after caller-order preservation and
      deduplication.
- [ ] Unknown-genre offset is applied after unknown-genre filtering in both
      shared resolver and `search_tracks` handler.
- [ ] Every branch applies offset/limit exactly once.
- [ ] Common no-post-filter search and playlist paths retain SQL pagination;
      only Rust post-filter paths fetch unbounded candidates.
- [ ] Defaults and 200-item caps remain unchanged.
- [ ] Offset beyond filtered results and limit zero return empty results.
- [ ] Targeted tests, format, dprint, clippy, full crate tests, release build,
      `--version`, and `--help` exit 0.
- [ ] `git diff --name-only` lists only `src/db.rs`,
      `src/tools/resolve.rs`, `src/tools/library_handlers.rs`,
      `src/tools/tests.rs`, and optionally `plans/README.md`.
- [ ] No write path to Rekordbox or internal SQLite was added.

## STOP conditions

Stop and report back if:

- A current caller relies on playlist offset being ignored or track-ID offset
  having different documented semantics; surface the contract conflict.
- A new post-filter beyond unknown genre/sampler exclusion is discovered in a
  caller. Add it explicitly to the local-pagination decision and tests rather
  than assuming DB pagination is safe.
- The common search path would need to fetch the full library to implement the
  fix; preserve SQL pagination when no Rust post-filter exists.
- Tests reveal unstable/non-unique ordering for search or playlist pages. A
  stable tie-breaker would be a separate public behavior decision; report it
  rather than silently changing order.
- The fix requires a new MCP parameter or response shape.
- Any path would write to Rekordbox.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- Any future selector post-filter must be declared in the
  `pagination_applied_in_db` decision. Applying SQL offset before a Rust filter
  recreates this bug.
- Keep selector priority, sort order, and pagination order documented together
  in `resolve_tracks` comments.
- Reviewers should trace each branch and mark exactly where offset and limit
  are applied.
- Cursor pagination and stable tie-breakers may be worthwhile for very large
  mutable libraries, but are intentionally deferred because this plan repairs
  the existing offset contract without changing the API.
