# Performance Audit

Audited ~42K lines across `src/` and `stratum-dsp/`. Findings grouped by severity.
All findings verified against source code; false positives removed.

## Critical — Blocking the Async Runtime

### C1: Essentia probe spin-polls on Tokio worker

`validate_essentia_python_with_timeout` spin-polls with `std::thread::sleep(25ms)`
for up to 5s on a Tokio worker thread. Called via `OnceLock::get_or_init` on first
call to `essentia_python_path()` from any async handler (e.g. `analyze_track_audio`,
`resolve_tracks_data`). Parks a Tokio worker for the entire probe duration. The
`setup_essentia` path correctly wraps its own validation in `spawn_blocking`, but the
`OnceLock` init path does not.

**Location:** `src/tools/essentia.rs:15-84`

### C2: Blocking file tag reads on async runtime

`year_from_file_tags` calls `tags::read_file_tags` (blocking lofty file I/O) directly
on the async runtime inside `scan_years` (sync fn), which is called from async
`handle_backfill_years` with no `spawn_blocking` wrapper. For hundreds of year-zero
tracks, this blocks the worker for the duration of all tag reads.

**Location:** `src/tools/year_handlers.rs:39-52`

## High — N+1 Query Patterns

All of these hit the cache store `Mutex<Connection>` once per iteration, executing
individual `prepare_cached()` + `execute()` calls per track with no batching. Batch
query functions exist in `store.rs` (`batch_enrichment_existence`,
`batch_enrichment_with_results`, etc.) and are used by `handle_cache_coverage`, but
not by these paths.

### H1: `scan_labels` — 4× N cache queries

4 `get_enrichment` calls per track (discogs, musicbrainz, bandcamp, beatport). No
short-circuit — even if discogs returns a label, all 4 providers are still queried.

**Location:** `src/tools/label_handlers.rs:84-155`

### H2: `scan_years` — up to 8× N cache queries

Short-circuit cascade of up to 4 enrichment reads (discogs, beatport, musicbrainz,
bandcamp). If all miss, 4 additional cache-gap checks follow. Worst-case: 8 queries
per year-zero track.

**Location:** `src/tools/year_handlers.rs:116-217`

### H3: `classify_batch` — 4× N cache queries

2 enrichment (discogs, beatport) + 2 analysis (stratum, essentia) cache reads per
track via `build_track_evidence`. Mutex acquired per iteration.

**Location:** `src/tools/classify_handler.rs:403-486`

### H4: `resolve_tracks_data` — 4× N cache queries

Same 4-query pattern as H3 (2 enrichment + 2 analysis). Mutex acquired per iteration.

**Location:** `src/tools/resolve_handlers.rs:100-158`

## High — Sequential Where Parallel

### H5: `backfill_labels` auto-enrich is sequential

Sequential `await` per Bandcamp lookup in a plain `for` loop. 200 tracks × 1.5s =
5 min wall time. No concurrency primitives.

**Location:** `src/tools/label_handlers.rs:190-208`

### H6: `backfill_years` auto-enrich is sequential

Two back-to-back sequential `for` loops: Bandcamp (258-276) and MusicBrainz (278-296),
each with individual `.await` calls.

**Location:** `src/tools/year_handlers.rs:258-296`

### H7: `backfill_albums` auto-enrich is sequential

Same sequential `for` loop pattern with Bandcamp lookups.

**Location:** `src/tools/album_handlers.rs:312-330`

### H8: `write_file_tags` and `embed_cover_art` are sequential

Process files one at a time via sequential `spawn_blocking(...).await` in a `for` loop.
`handle_read_file_tags` correctly uses semaphore (capacity 8) + spawned handles for
parallel reads.

**Location:** `src/tools/file_tag_handlers.rs:136-218, 240-300`

## Medium — Memory & Allocation

### M1: Beam search clones entire `BeamState` per expansion

`remaining: HashSet<String>` copies all track ID strings on every clone.
O(W × sum(N-t)) total clone work across all steps — still substantial for large pools.
For 100 tracks, 30 slots, beam width 8: thousands of HashSet clones. A bitset or
`Arc`-wrapped persistent set would eliminate this.

**Location:** `src/tools/scoring.rs:507-524`

### M2: `AxisScore.label` and `ScoreAdjustment.reason` are heap-allocated String

Mix of static literals (via `.to_string()`) and `format!()` calls. The formatted cases
genuinely need heap allocation; the literal cases do not. `Cow<'static, str>` would
help the literal cases while still supporting formatted strings. These get cloned
inside every `BeamState` clone.

**Location:** `src/tools/scoring.rs:59-70`

### M3: `normalize_for_matching` allocates 4 intermediate strings

NFC collect, lowercase, filter collect, trim to_string. The final `.to_string()` always
allocates even when no trimming occurs. Called ~600× per 200-track classify batch.

**Location:** `src/normalize.rs:9-19`

### M5: No upper bound on PCM accumulation in MCP tool path

CLI has a dual `cpu_sem` + `mem_sem` memory budget. MCP `handle_analyze_track_audio`
has only a concurrency semaphore with no per-track size guard. A 60-min mix could spike
to 600+ MB.

**Location:** `src/tools/audio_handlers.rs`

### M6: `row_to_track` does 10× `trim().to_string()` per row

`row.get::<_, String>()` allocates, `.trim()` borrows, `.to_string()` allocates again
unconditionally (even when no trimming occurs). 10 fields per row.

**Location:** `src/db.rs:88-107`

## Medium — Database & Caching

### M10: `batch_enrichment_with_label` forces `json_extract` on every row

`json_extract(response_json, '$.label')` predicate has no index support. Must be
evaluated row-by-row for every row passing the provider/artist filter.

**Location:** `src/store.rs:334-347`

### M11: Discogs broker session read from SQLite on every lookup

`lookup_discogs_remote` reads the persisted broker session from the cache store on
every call, acquiring the shared `Mutex<Connection>`. The session token should be
cached in-memory in `ServerState` after first validation.

**Location:** `src/tools/discogs_auth.rs:200-207`

## Medium — Network

### M13: MCP lookup handlers have no retry logic

Single transient 429 or 5xx fails the enrichment immediately — the agent must retry
manually. CLI hydrate correctly retries with Retry-After / backoff. This is an
intentional asymmetry (CLI is batch, MCP is interactive) but worth noting.

**Location:** `src/tools/enrich_handlers.rs` (all four `handle_lookup_*` handlers)

## Low — Minor Inefficiencies

### L1: `apply_search_filters` uses `Box<dyn ToSql>` per filter param

Heap-allocates integers. Could use an enum in a `SmallVec`. At most ~15 params; cost
is negligible in practice.

**Location:** `src/db.rs:186-306`

### L2: `get_tracks_by_ids` builds HashMap then drains to Vec

Intentional pattern to preserve caller-specified order and deduplicate across chunked
IN queries. Load-bearing — no simpler correct alternative when chunking is required.

**Location:** `src/db.rs:675-708`

### L4: `get_playlists` uses correlated subquery per row

TrackCount computed via correlated subquery per playlist. Tens of rows in a typical
library — negligible.

**Location:** `src/db.rs:414-443`

### L5: `get_sessions` duration query uses scalar subquery

Duration calculation uses an IN-list batch fetch with a scalar subquery correlated to
the GROUP BY. Bounded to max 100 sessions. Negligible.

**Location:** `src/db.rs:757-769`

### L6: Beatport full HTML page buffered + untyped JSON parse

Full page buffered via `resp.text()`, then `__NEXT_DATA__` parsed as
`serde_json::Value`. A typed struct would skip unused fields. Network/rate-limit
dominates timing.

**Location:** `src/beatport.rs:65-93, 144`

## What's Done Well

- **Audio batch pipeline** (`handle_analyze_audio_batch`) — bounded channels,
  `spawn_blocking` writer, read-only parallel connections, concurrency semaphore.
- **`enrich_tracks`** — semaphore-bounded concurrency with `tokio::spawn` per track,
  auth-failure watch channel, channel-backed cache writer.
- **Single `reqwest::Client`** with connection pooling and timeouts on both MCP server
  and CLI hydrate paths.
- **Audio analysis caching** with schema versioning — stale cache auto-evicts on
  analyzer version bump.
- **XML generation** with `String::with_capacity(tracks.len() * 512)`.
- **`spawn_blocking`** correctly used for symphonia decode, lofty tag reads (in
  file_tag_handlers read path), filesystem scans, SHA-256 hashing, and audit scans.
- **`prepare_cached`** used for all static SQL queries; dynamic queries correctly use
  `prepare`.
- **Decode buffer pre-allocated** using symphonia `n_frames` hint.
- **Audio analysis serialized once** — `to_value` then `to_string(&val)` for cache.
- **`resolve_audit_issues`** wrapped in transaction for atomic batch updates.

## Recommended Priority

1. **Batch cache reads** (H1-H4) — biggest throughput win for classify/resolve/backfill.
2. **Parallelize backfill auto-enrich** (H5-H7) — adopt the `enrich_tracks` pattern.
3. **Beam search data structures** (M1-M2) — bitset + `Cow<'static, str>`.
4. **Wrap blocking calls in `spawn_blocking`** (C1-C2).
