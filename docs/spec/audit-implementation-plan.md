# Audit Engine — Implementation Plan

Implementation plan for the [audit-idempotency spec](audit-idempotency.md). Follows existing codebase patterns from `tools.rs`, `store.rs`, `tags.rs`.

## New Files

| File | Purpose |
|------|---------|
| `src/audit.rs` | Convention checks, filename parsing, scan orchestration, auto-fix execution, report generation |

Modified files: `src/main.rs` (add `mod audit`), `src/store.rs` (schema v3 + audit CRUD), `src/tools.rs` (add `audit_state` tool).

## Phase 1: Schema & Persistence

**File:** `src/store.rs`

1. Increment `user_version` from 2 → 3
2. Add migration block for `audit_files` and `audit_issues` tables per spec schema
3. Add CRUD functions:
   - `upsert_audit_file(conn, path, mtime, size)` — INSERT or UPDATE on conflict
   - `get_audit_file(conn, path) -> Option<AuditFile>`
   - `get_audit_files_in_scope(conn, scope) -> Vec<AuditFile>` — `WHERE path LIKE scope || '%'`
   - `delete_audit_file(conn, path)` — CASCADE deletes issues
   - `update_audit_file_path(conn, old_path, new_path)` — for renames, CASCADE updates issues
   - `upsert_audit_issue(conn, path, issue_type, detail, status)` — INSERT or UPDATE on UNIQUE(path, issue_type)
   - `get_audit_issues(conn, scope, status, issue_type, limit, offset) -> Vec<AuditIssue>`
   - `get_audit_issue_by_id(conn, id) -> Option<AuditIssue>`
   - `resolve_audit_issues(conn, ids, resolution, note) -> usize`
   - `get_audit_summary(conn, scope) -> AuditSummary`
   - `delete_missing_audit_files(conn, scope, existing_paths) -> usize` — remove rows not in the set

4. Define types: `AuditFile`, `AuditIssue`, `AuditSummary`

**Test:** Unit test the migration (open DB, verify tables exist, insert/query round-trip).

## Phase 2: Convention Checks

**File:** `src/audit.rs`

Core logic — pure functions that take tag data and return issues. No I/O.

1. Define `IssueType` enum (15 variants, `Display` impl for DB string)
2. Define `SafetyTier` enum (`Safe`, `RenameSafe`, `Review`) with `IssueType::safety_tier()` method
3. Define `AuditContext` enum (`AlbumTrack`, `LooseTrack`)
4. Implement `classify_track_context(path: &Path) -> AuditContext`:
   - Normalize directory name (strip `[FLAC]`, `[WAV]`, etc. before matching)
   - Check parent dir for `(YYYY)` suffix pattern → `AlbumTrack`
   - Check if in `play/` dir → `LooseTrack`
   - Default → `LooseTrack`
5. Implement `check_tags(path: &Path, read_result: &FileReadResult, context: &AuditContext, skip: &HashSet<IssueType>) -> Vec<DetectedIssue>`:
   - `EMPTY_ARTIST` — artist field empty/null
   - `EMPTY_TITLE` — title field empty/null
   - `MISSING_TRACK_NUM` — album track, track field empty
   - `MISSING_ALBUM` — album track, album field empty
   - `MISSING_YEAR` — album track, year AND date both empty
   - `ARTIST_IN_TITLE` — title starts with `{artist} - ` (case-insensitive)
   - `WAV_TAG3_MISSING` — WAV with non-empty `tag3_missing`
   - `WAV_TAG_DRIFT` — WAV where id3v2 and riff_info values differ for same field
   - `GENRE_SET` — genre field non-empty
   - `NO_TAGS` — all 14 fields empty
6. Implement `check_filename(path: &Path, read_result: &FileReadResult, context: &AuditContext, skip: &HashSet<IssueType>) -> Vec<DetectedIssue>`:
   - `BAD_FILENAME` — doesn't match canonical or acceptable alternates
   - `ORIGINAL_MIX_SUFFIX` — filename contains `(Original Mix)`
   - `TECH_SPECS_IN_DIR` — directory contains `[FLAC]`, `[WAV]`, `24-96`, etc.
   - `MISSING_YEAR_IN_DIR` — album dir missing `(YYYY)` suffix
   - `FILENAME_TAG_DRIFT` — parsed artist/title disagrees with tag values
7. Implement `parse_filename(path: &Path, context: &AuditContext) -> ParsedFilename`:
   - Album tracks: `NN Artist - Title.ext` parsing per conventions.md
   - Loose tracks: `Artist - Title.ext` parsing
   - Returns `ParsedFilename { track_num, artist, title }` with all fields optional

**Test:** Unit tests for each check function with synthetic `FileReadResult` data. Test filename parsing edge cases (hyphens in titles, missing track numbers, acceptable alternates).

## Phase 3: Scan Operation

**File:** `src/audit.rs`

Orchestrates filesystem walk, tag reading, convention checks, and persistence.

1. Implement `scan(conn: &Connection, scope: &str, revalidate: bool, skip_issue_types: &HashSet<IssueType>) -> Result<ScanSummary>`:
   - Walk filesystem under scope (skip symlinks, filter audio extensions)
   - Load existing `audit_files` for scope from DB
   - Classify each file: New / Changed / Unchanged / Missing
   - For New + Changed (+ Unchanged if `revalidate`):
     - Read tags via `tags::read_file_tags` (synchronous, called from blocking context)
     - Run `check_tags` + `check_filename`
     - Upsert into `audit_files` and `audit_issues`
     - Preserve accepted/deferred status on re-detected issues
   - For Missing: delete from `audit_files` (CASCADE)
   - **Batch commits:** Wrap every ~500 files in a transaction
   - Build and return `ScanSummary`

2. File walking helper: `walk_audio_files(scope: &Path) -> Vec<PathBuf>`:
   - Use `std::fs::read_dir` recursively (or `walkdir` crate if available)
   - Filter: `.flac`, `.wav`, `.mp3`, `.m4a` extensions
   - Skip symlinks
   - Sort for deterministic ordering

3. Change detection: `file_changed(audit_file: &AuditFile, metadata: &std::fs::Metadata) -> bool`:
   - Compare mtime (as ISO 8601) and size

**Test:** Integration test with temp directory of real audio files (or mocked). Verify incremental scan skips unchanged files. Verify batch commits survive simulated mid-scan failure.

## Phase 4: Query, Resolve, Summary, Report

**File:** `src/audit.rs`

Thin wrappers over store CRUD — mostly formatting.

1. `query_issues(conn, scope, status, issue_type, limit, offset) -> Vec<AuditIssue>` — delegates to store
2. `resolve_issues(conn, ids, resolution, note) -> usize` — delegates to store
3. `get_summary(conn, scope) -> AuditSummary` — delegates to store
4. `export_report(conn, scope, output_path) -> Result<PathBuf>`:
   - Query all issues for scope
   - Generate markdown: header, summary table, open issues by type, accepted/deferred items, stats
   - Write to disk

**Test:** Unit test report generation against known issue data.

## Phase 5: Auto-Fix

**File:** `src/audit.rs`

The most sensitive phase — modifies files on disk.

1. Implement `auto_fix(conn: &Connection, db_conn: Option<&Connection>, issue_ids: &[i64], dry_run: bool) -> Result<AutoFixResult>`:
   - Load issues by ID, validate all are "open"
   - Refuse Review-tier issues → add to `refused` list
   - For Safe-tier tag fixes:
     - `WAV_TAG3_MISSING`: build `write_file_tags` call copying id3v2 → riff_info for missing fields
     - `WAV_TAG_DRIFT`: build `write_file_tags` call syncing riff_info to id3v2 values
     - `ARTIST_IN_TITLE`: build `write_file_tags` call stripping artist prefix from title
   - For Rename-safe fixes:
     - Check Rekordbox import status via `db::search_tracks` (requires Rekordbox DB connection)
     - If imported → add to `skipped_imported` list
     - If not imported:
       - `ORIGINAL_MIX_SUFFIX`: compute new filename, `fs::rename`, update `audit_files.path`
       - `TECH_SPECS_IN_DIR`: compute new dir name, `fs::rename` dir, update all `audit_files` rows with old prefix
   - If `dry_run=true`: return preview without executing
   - If `dry_run=false`: execute writes/renames, mark issues resolved

2. The Rekordbox DB connection is optional — if unavailable, treat all rename-safe issues as "skipped" (can't verify import status).

**Test:** Integration test with temp files. Verify WAV tag 3 copy works. Verify rename updates audit_files. Verify imported files are skipped. Verify dry_run changes nothing.

## Phase 6: MCP Tool Wiring

**File:** `src/tools.rs`

1. Define `AuditStateParams` struct with tagged operation enum:
   ```rust
   #[derive(Debug, Deserialize, JsonSchema)]
   #[serde(tag = "operation")]
   pub enum AuditStateParams {
       #[serde(rename = "scan")]
       Scan { scope: String, revalidate: Option<bool>, skip_issue_types: Option<Vec<String>> },
       #[serde(rename = "query_issues")]
       QueryIssues { scope: String, status: Option<String>, issue_type: Option<String>, limit: Option<u32>, offset: Option<u32> },
       #[serde(rename = "resolve_issues")]
       ResolveIssues { issue_ids: Vec<i64>, resolution: String, note: Option<String> },
       #[serde(rename = "auto_fix")]
       AutoFix { issue_ids: Vec<i64>, dry_run: Option<bool> },
       #[serde(rename = "get_summary")]
       GetSummary { scope: String },
       #[serde(rename = "export_report")]
       ExportReport { scope: String, output_path: Option<String> },
   }
   ```

2. Add `#[tool(description = "...")]` handler that dispatches to audit module functions
3. Handle the internal SQLite connection (same `store::open` used for enrichment cache) and optional Rekordbox DB connection (for import-status checks)
4. Wrap blocking audit calls in `spawn_blocking`

**File:** `src/main.rs`

5. Add `mod audit;`

**Test:** End-to-end test calling the tool handler with JSON params.

## Implementation Order

```
Phase 1 (schema)  ──→  Phase 2 (checks)  ──→  Phase 3 (scan)  ──→  Phase 6 (wiring)
                                                      ↓
                                               Phase 4 (query/report)
                                                      ↓
                                               Phase 5 (auto_fix)
```

Phases 1 and 2 are independent and can be built in parallel. Phase 3 depends on both. Phase 6 (wiring) can happen as soon as Phase 3 is done — query/resolve/report and auto_fix can be wired incrementally.

**Minimum viable tool:** Phases 1 + 2 + 3 + 6 (scan + query_issues + get_summary). This alone replaces the entire old exiftool/kid3-cli audit flow. Auto-fix and report export are additive.

## Dependencies

- No new crates required. `walkdir` would be nice for recursive directory traversal but `std::fs::read_dir` is sufficient.
- `tags::read_file_tags` is synchronous — the scan must run in a blocking context.
- Rekordbox DB connection is optional (only needed for import-status checks in auto_fix).

## Risk Areas

- **Filename parsing edge cases:** Titles with ` - ` in them, non-ASCII characters, tracks with no separator. Mitigate with unit tests from real collection examples.
- **Batch transaction boundaries:** If a transaction commits 500 files and the 501st fails, the first 500 are persisted. The summary must accurately reflect what was committed, not what was attempted.
- **WAV tag drift comparison:** Fields may have trailing whitespace or encoding differences. Normalize before comparing.
- **Directory renames affecting multiple audit_files rows:** Must update all rows atomically in a single transaction.
