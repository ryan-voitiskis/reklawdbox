# Spec: Collection Audit Engine & Idempotent State

## Problem

The collection-audit SOP has two problems:

1. **Slow, fragile tooling.** It relies on exiftool (Perl, per-file invocation) and kid3-cli (Qt app per call) for tag reads and writes. On a 17,000+ file collection, a full exiftool scan takes ~5 minutes for 1,300 WAV files alone. Shell quoting bugs with apostrophes in filenames and alias conflicts (`find`/`grep` aliased to `fd`/`rg`) make the shell snippets fragile.

2. **No memory.** Re-running the SOP re-scans everything and re-flags all issues, including ones already fixed or explicitly accepted. Every run is a full run with no way to do incremental audits.

3. **No fix execution.** Renames (stripping `(Original Mix)`, removing `[FLAC]` from directories) are done via shell snippets with no awareness of Rekordbox import status. Renaming an imported file silently breaks Rekordbox references.

## Key Context

- **~17,000 files** in the full collection across all formats (FLAC, WAV, MP3, M4A)
- **~2,400 tracks** imported into Rekordbox — the rest are un-imported
- **Native tag tools** (`read_file_tags`, `write_file_tags`) now exist in reklawdbox, built on lofty (Rust). They handle all formats, distinguish WAV ID3v2 (tag 2) from RIFF INFO (tag 3), and target ~1,000 reads in <5 seconds with batched concurrent I/O.
- **Genre policy:** Genre tags are left blank in files and managed exclusively through Rekordbox (see [conventions.md](../operations/conventions.md)). Pre-existing genre tags are convention violations flagged for user review.

## Requirements

### Audit engine

1. Convention checks run **server-side** — the agent does not process raw tag data for 17k files
2. The server reads files using the same lofty infrastructure as `read_file_tags`, applies convention rules, and records results directly into SQLite
3. Rekordbox DB queries (`search_tracks`, `resolve_tracks_data`) remain supplementary:
   - Check import status before renames (to avoid breaking Rekordbox references)
   - Detect DB-vs-file drift (optional deep audit)
   - Access Rekordbox-only metadata (playlists, hot cues, genre assignments)
4. No dependency on exiftool or kid3-cli — native tag tools replace both
5. Tag fix execution uses `write_file_tags`
6. Rename execution uses `std::fs::rename` with Rekordbox import-status pre-check

### Idempotent state

7. Record audit state: files scanned, issues found, fixes applied, user decisions
8. Subsequent runs diff against prior state — only surface new files, changed files, or unresolved issues
9. Track user decisions ("accepted", "deferred", "won't fix") so they don't resurface
10. State persisted in the existing reklawdbox local SQLite cache DB
11. New MCP tool (`audit_state`) exposes scanning, fixing, and state management to the agent

## Audit Engine

### Why server-side

At 17,000 files, agent-side convention checking is impractical. A `read_file_tags` batch of 2,000 files returns ~2,000 × 14 fields of tag data. Across 9 batches to cover the full collection, that's an enormous amount of data flowing through the agent's context window — just to apply mechanical string-matching rules. The agent would then need to make hundreds of `record_issue` calls per batch.

Moving convention checks server-side means: the server reads tags, applies rules, writes results to SQLite, and returns a summary. The agent sees counts, not raw data. It then queries specific issues to review and make decisions.

### Convention checks

The `scan` operation applies these checks using the same lofty tag-reading code as `read_file_tags`. Convention rules reference [conventions.md](../operations/conventions.md).

#### Track context inference

The scan classifies each file as **album track** or **loose track** based on directory structure. Classification runs after normalizing the directory name (stripping tech specs like `[FLAC]` and `[WAV]` before pattern matching):

- File in a directory matching `*/ (* (*))/` (parent has a year-suffix pattern like `Album Name (2024)/`) → album track
- File in a `play/` directory or similar flat structure → loose track
- Ambiguous → treated as loose track (fewer required fields)

This affects which tags are required (album tracks need track number, album, year; loose tracks only need artist and title).

#### Issue types

Tag-level issues:

<!-- dprint-ignore -->
| Issue type          | Detection                                              | Context      | Safety tier |
| ------------------- | ------------------------------------------------------ | ------------ | ----------- |
| `EMPTY_ARTIST`      | `artist` field empty/null                              | All files    | Review      |
| `EMPTY_TITLE`       | `title` field empty/null                               | All files    | Review      |
| `MISSING_TRACK_NUM` | `track` field empty/null                               | Album tracks | Review      |
| `MISSING_ALBUM`     | `album` field empty/null                               | Album tracks | Review      |
| `MISSING_YEAR`      | `year`/`date` both empty/null                          | Album tracks | Review      |
| `ARTIST_IN_TITLE`   | `title` starts with `{artist} - ` (case-insensitive)   | All files    | Safe        |
| `WAV_TAG3_MISSING`  | WAV file with `tag3_missing` non-empty                 | WAV only     | Safe        |
| `WAV_TAG_DRIFT`     | WAV `id3v2` and `riff_info` values differ for a field  | WAV only     | Safe        |
| `GENRE_SET`         | `genre` field non-empty                                | All files    | Review      |
| `NO_TAGS`           | All 14 tag fields empty/null                           | All files    | Review      |

Filename and directory issues:

<!-- dprint-ignore -->
| Issue type            | Detection                                                          | Safety tier                  |
| --------------------- | ------------------------------------------------------------------ | ---------------------------- |
| `BAD_FILENAME`        | Filename doesn't match canonical or any acceptable alternate       | Review                       |
| `ORIGINAL_MIX_SUFFIX` | Filename contains `(Original Mix)`                                 | Rename-safe (if not imported)|
| `TECH_SPECS_IN_DIR`   | Directory contains `[FLAC]`, `[WAV]`, `24-96`, etc.               | Rename-safe (if not imported)|
| `MISSING_YEAR_IN_DIR` | Album directory missing `(YYYY)` suffix                            | Review                       |
| `FILENAME_TAG_DRIFT`  | Artist/title parsed from filename disagrees with tag values        | Review                       |

The `detail` field on each issue carries specifics as JSON (e.g., which fields are missing in `tag3_missing`, what the filename vs tag values are for drift, the suggested fix value for auto-fixable issues).

#### Safety tiers

Issue types are classified into safety tiers that determine whether `auto_fix` can apply them:

- **Safe** — Mechanical, deterministic, lossless. `auto_fix` applies these without further confirmation. Examples: copying tag 2 → tag 3, stripping a known prefix from the title field, syncing drifted WAV tags to the authoritative tag 2 values.

- **Rename-safe** — Well-defined string transformation, but requires a Rekordbox import-status check first. `auto_fix` applies these only for files NOT imported into Rekordbox. Imported files are skipped with a warning in the result. Examples: stripping `(Original Mix)` from a filename, removing `[FLAC]` from a directory name.

- **Review** — Requires human judgment or external data (filename parsing can be wrong, album metadata needs Discogs lookup, drift direction is ambiguous). `auto_fix` refuses these; the agent must present them to the user.

#### Filename parsing

Filenames are parsed into artist/title using the rules from [conventions.md](../operations/conventions.md):

- **Album tracks:** First 2 chars = zero-padded track number, skip space, split remainder on first ` - ` → artist, title
- **Loose tracks:** Split on first ` - ` → artist, title
- **Acceptable alternates:** `NN. Title.ext` and `NN - Title.ext` (single-artist album, artist from directory context)

Edge cases: titles containing ` - ` split on first occurrence only. VA compilations use per-track artist from the filename, not AlbumArtist from the directory.

### DB cross-referencing (supplementary)

Not part of the `scan` operation. Used by `auto_fix` for import-status checks and by the agent for manual investigation:

- **Before renames:** `search_tracks(path="...")` to check Rekordbox import status. If the file is imported, block the rename and flag for manual handling.
- **Drift detection:** Compare `resolve_tracks_data` values against file tags (optional)
- **Metadata gap-filling:** `lookup_discogs` / `lookup_beatport` for missing album/year/label

### Fix execution

**Tag fixes** use `write_file_tags`. WAV files automatically get both layers written (default `wav_targets: ["id3v2", "riff_info"]`). Dry-run mode previews changes before writing.

**Renames** use `std::fs::rename`. Before any rename:

1. Check Rekordbox import status via `search_tracks(path="<exact_path>")`
2. If imported → skip the rename, return a warning (the file needs manual relocation or a future XML-path-update mechanism)
3. If not imported → execute `fs::rename`, then update `audit_files.path` in the same SQLite transaction
4. For directory renames (`TECH_SPECS_IN_DIR`): update all `audit_files` rows whose path starts with the old directory prefix

After fixes, the agent can re-scan the affected scope to verify — the scan will detect that files changed (new mtime for tag writes) or have new paths (for renames) and re-check them, automatically resolving issues that are now fixed.

#### Imported file renames: not supported

Rekordbox XML import cannot update file paths for existing tracks — it always creates a duplicate entry (tested 2026-02-25, Rekordbox 7.2.10; see [test results](../operations/test-plans/rekordbox-xml-path-relocation.md)). Rekordbox matches tracks by `Location` path, not by `TrackID` or metadata.

This means renames of imported files are **blocked by the audit engine**. When `auto_fix` encounters a rename-safe issue on an imported file, it skips it with a warning. The user must handle these manually using Rekordbox's built-in relocate feature (right-click → Relocate) after renaming outside the system.

A future enhancement could export a relocation mapping (old→new paths) to assist with manual relocation, but there is no automatic path.

## Idempotent State

### Storage

SQLite table(s) in the existing reklawdbox local cache DB. Leverages the existing WAL-mode SQLite infrastructure used for enrichment and audio analysis caches.

#### Schema

```sql
CREATE TABLE audit_files (
    path         TEXT PRIMARY KEY,
    last_audited TEXT NOT NULL,      -- ISO 8601
    file_mtime   TEXT NOT NULL,      -- ISO 8601
    file_size    INTEGER NOT NULL
);

CREATE TABLE audit_issues (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    path        TEXT NOT NULL REFERENCES audit_files(path) ON DELETE CASCADE ON UPDATE CASCADE,
    issue_type  TEXT NOT NULL,       -- enum values from issue types above
    detail      TEXT,                -- JSON with specifics and suggested fix
    status      TEXT NOT NULL DEFAULT 'open',  -- open | resolved | accepted | deferred
    resolution  TEXT,                -- fixed | accepted_as_is | wont_fix | deferred
    note        TEXT,                -- user comment
    created_at  TEXT NOT NULL,       -- ISO 8601
    resolved_at TEXT,                -- ISO 8601
    UNIQUE(path, issue_type)
);

CREATE INDEX idx_audit_issues_status ON audit_issues(status);
CREATE INDEX idx_audit_issues_path ON audit_issues(path);
```

The `UNIQUE(path, issue_type)` constraint means each file has at most one issue per type. On re-scan, existing issues are updated (not duplicated). This allows independent resolution — user can fix the year but accept a missing album.

`ON DELETE CASCADE` ensures that if a file is removed from `audit_files` (e.g., deleted from disk), its issues are cleaned up. `ON UPDATE CASCADE` ensures that when a file is renamed (path updated in `audit_files`), its issues follow automatically.

### Diff logic

Handled inside the `scan` operation:

1. List all files in the target scope from the filesystem (symlinks are skipped)
2. Look up each file in `audit_files` by path
3. Classify:
   - **New:** not in `audit_files` — read tags, run checks, insert results
   - **Changed:** mtime or size differs — read tags, run checks, update results. Preserve prior user decisions: if an issue was "accepted"/"deferred" and the same issue still exists, keep the user's status. If the issue is gone, mark it "resolved" with resolution "fixed".
   - **Unchanged:** mtime and size match — skip tag reading. Existing issues remain as-is.
   - **Missing:** in `audit_files` but not on disk — delete from `audit_files`, CASCADE deletes issues
4. Commit results to SQLite in batches (see [Partial progress](#partial-progress))
5. Return summary (not per-file details)

**`revalidate=true` semantics:** When `revalidate=true` is passed to `scan`, unchanged files (by mtime/size) are re-read and re-checked as if they were "Changed" files. This allows the user to force re-validation of the entire scope if they suspect mtime/size checks missed an edit (e.g., in-place file modifications, clock drift). Re-reading under `revalidate=true` does **not** reset user decisions — accepted/deferred issues are preserved if they still exist, following the "Changed" logic above. Deliberate user choices survive re-validation.

### Partial progress

The `scan` operation commits results to SQLite in batches (e.g., every 500 files). If a scan fails or times out mid-way:

- Files already committed appear as "unchanged" on the next incremental scan
- The next `scan` call picks up where the previous one left off automatically
- No special resume logic is needed — it falls out of the existing diff model

Each batch is independently correct. Partial state is valid state.

### File identity

Path is the primary key. Renames performed through `auto_fix` update `audit_files.path` in the same transaction as the filesystem rename, preserving audit history. External renames (outside the system) reset audit history for that file. Content hashing is not worth the complexity.

### Scope handling

Scopes are **directory paths that always end with `/`**. The server enforces a trailing `/` (appends one if missing). Scope filtering uses prefix matching: a file is in scope if its path starts with the scope string.

This ensures `/collection/Art/` matches `/collection/Art/track.flac` but not `/collection/ArtPunk/track.flac` or `/backup/collection/Art/track.flac`.

Auditing a narrow scope first (e.g., `/collection/ArtistA/`) and then a broader scope (e.g., `/collection/`) is efficient — previously audited files are skipped if unchanged.

## MCP Tool: `audit_state`

Single tool with an `operation` enum (tagged enum in Rust). Keeps state management out of the agent's context window.

### `scan`

Scan files in a scope, apply convention checks, record results.

**Params:**

- `scope` (string, required) — directory path; trailing `/` enforced
- `revalidate` (bool, default false) — re-read all files including unchanged; preserves user decisions
- `skip_issue_types` (array of strings, optional) — issue types to exclude from detection (e.g., `["GENRE_SET"]` to skip genre flagging). The SOP should instruct the agent to confirm issue type selection with the user before scanning.

**Behavior:** Reads tags for new/changed files using lofty, applies convention checks (excluding `skip_issue_types`), records results in `audit_files` and `audit_issues` via diff logic above. Processes files in internal batches with concurrent I/O. Commits to SQLite every ~500 files for partial-progress resilience.

**Returns:**

```json
{
  "files_in_scope": 1342,
  "scanned": 58,
  "skipped_unchanged": 1280,
  "missing_from_disk": 4,
  "skipped_issue_types": ["GENRE_SET"],
  "new_issues": { "EMPTY_ARTIST": 3, "WAV_TAG3_MISSING": 12 },
  "auto_resolved": { "MISSING_YEAR": 2 },
  "total_open": 45,
  "total_resolved": 312,
  "total_accepted": 28,
  "total_deferred": 5
}
```

Large scopes (17k files) may take tens of seconds on first run; subsequent incremental runs are fast since unchanged files are skipped.

### `query_issues`

List issues matching filters.

**Params:** `scope` (directory path prefix), `status` (optional: "open" | "resolved" | "accepted" | "deferred"), `issue_type` (optional filter), `limit` (default 100), `offset` (default 0)

**Returns:** Array of issues with `id`, `path`, `issue_type`, `detail`, `status`, `resolution`, `note`, `created_at`, `resolved_at`.

### `resolve_issues`

Batch-resolve issues (record user decisions without executing fixes).

**Params:** `issue_ids` (array of integer IDs), `resolution` ("accepted_as_is" | "wont_fix" | "deferred"), `note` (optional user comment)

**Returns:** Count of issues updated.

### `auto_fix`

Apply known-safe fixes for a set of issues. Dry-run by default.

**Params:**

- `issue_ids` (array of integer IDs) — issues to fix. Must be "open" status. Issues with safety tier "Review" are refused.
- `dry_run` (bool, default true) — preview fixes without applying

**Behavior:**

1. Validate all issues exist and are "open"
2. Refuse any issues with safety tier "Review" — return them as `refused` with reason
3. For **Safe** issues: apply tag fix via `write_file_tags`
4. For **Rename-safe** issues: check Rekordbox import status; if imported, skip with warning; if not imported, apply `fs::rename` and update `audit_files.path`
5. Mark applied issues as "resolved" with resolution "fixed"

**Returns:**

```json
{
  "dry_run": true,
  "fixes": [
    { "id": 12, "path": "/collection/track.wav", "issue_type": "WAV_TAG3_MISSING", "action": "copy_tag2_to_tag3", "fields": ["artist", "title"] },
    { "id": 15, "path": "/collection/track.flac", "issue_type": "ARTIST_IN_TITLE", "action": "strip_artist_prefix", "old_title": "Artist - Title", "new_title": "Title" }
  ],
  "skipped_imported": [
    { "id": 18, "path": "/collection/Artist/01 Track (Original Mix).wav", "issue_type": "ORIGINAL_MIX_SUFFIX", "reason": "File is imported in Rekordbox — rename would break references" }
  ],
  "refused": [
    { "id": 20, "issue_type": "EMPTY_ARTIST", "reason": "Review-tier issue — requires human judgment" }
  ]
}
```

When `dry_run=false`, the same structure is returned with actual results instead of previews. The agent should present the `dry_run=true` result to the user, get approval, then call again with `dry_run=false`.

### `get_summary`

Aggregate counts for a scope.

**Params:** `scope` (directory path prefix)

**Returns:** Counts by issue type × status. Same shape as the summary portion of `scan` output but queryable at any time.

### `export_report`

Write a markdown collection health report to disk. Generated server-side so the agent doesn't need to load all issues into context.

**Params:** `scope` (directory path prefix), `output_path` (optional, default `docs/reports/audit-{scope-slug}-{date}.md`)

**Contents:**

- **Header:** scope, timestamp, total files audited
- **Summary table:** counts by issue type and status (open / resolved / accepted / deferred)
- **Open issues:** grouped by issue type, each with file path and detail
- **Accepted/deferred items:** grouped separately so user decisions are visible
- **Stats:** pass rate, files with no issues, files never audited

**Returns:** Path to the written file.

## Agent Workflow

The agent's role is **orchestrator**: initiate scans, review issues, present choices to the user, record decisions, execute approved fixes.

```
1. Confirm issue type selection with user:
   "Skip GENRE_SET for this pass, or include it?"
   → user: skip it

2. audit_state(scan, scope="/collection/ArtistA/", skip_issue_types=["GENRE_SET"])
   → summary: 14 new issues found

3. audit_state(query_issues, scope="/collection/ArtistA/", status="open")
   → 14 issues: 12× WAV_TAG3_MISSING, 2× ORIGINAL_MIX_SUFFIX

4. Present auto-fixable issues to user:
   "12 WAV files missing tag 3, 2 files with (Original Mix) suffix — auto-fix all?"
   → user: yes

5. audit_state(auto_fix, issue_ids=[1..14], dry_run=true)
   → preview: 12 tag copies, 2 renames (both un-imported)

6. Present preview to user → user approves

7. audit_state(auto_fix, issue_ids=[1..14], dry_run=false)
   → 14 fixes applied

8. audit_state(scan, scope="/collection/ArtistA/")
   → re-scans changed files, confirms all issues auto-resolved

9. audit_state(export_report, scope="/collection/")
   → writes docs/reports/audit-collection-2026-02-25.md
```

## SOP Changes Required

The collection-audit SOP needs updating after this spec is implemented:

1. **Prerequisites:** Remove exiftool and kid3-cli. Only `reklawdbox` MCP is required.
2. **Issue type selection:** Agent confirms with user which issue types to include (especially whether to include `GENRE_SET`, which can produce thousands of flags). Added to SOP as an explicit step.
3. **Steps 1+2 (DB + File Audit):** Replace with `audit_state(scan)`. One step, not two.
4. **Step 3 (Filename Scan):** Absorbed into `scan` — server handles filename convention checks.
5. **Step 4 (Issue Report):** Use `audit_state(query_issues)` + `get_summary` instead of agent-compiled report.
6. **Step 5 (Fix Execution):** Use `audit_state(auto_fix)` for safe/rename-safe issues. Present review-tier issues to user for manual decision. Replace kid3-cli/exiftool/shell-rename patterns entirely.
7. **Step 6 (Verification):** Re-run `audit_state(scan)` on the fixed scope — changed files are re-checked automatically.
8. **Step 7 (Final Report):** Use `audit_state(export_report)`.

## Rekordbox Relocation Test Results

Tested 2026-02-25 on Rekordbox 7.2.10. XML import with a modified `Location` creates a duplicate track — it does not update the existing entry's path. Full details in [test plan](../operations/test-plans/rekordbox-xml-path-relocation.md).

This confirms that the audit engine must block renames of imported files. Only un-imported files can be safely renamed via `auto_fix`.

## Decisions

- **Server-side convention checks:** The `scan` operation reads files and applies rules internally. The agent never processes raw tag data for auditing — it sees summaries and issues. This is necessary at 17k-file scale to avoid context window exhaustion.
- **Single tool:** `audit_state` with `operation` param, not multiple tools. Tagged enum in Rust.
- **Concrete issue types:** 15 defined types covering tag, filename, and directory conventions. Each is a string enum value in the `issue_type` column.
- **Safety tiers:** Issues classified as Safe, Rename-safe, or Review. `auto_fix` only applies Safe and Rename-safe issues. Review-tier issues require human judgment.
- **UNIQUE(path, issue_type):** One issue per type per file. Re-scans upsert, not duplicate. User decisions survive re-scans of unchanged files.
- **ON UPDATE CASCADE:** Renames update `audit_files.path`; issues follow via cascade. Audit history is preserved for system-performed renames.
- **Scope as directory prefix:** Scopes always end with `/`, filtering uses `starts_with`. No substring matching.
- **Partial progress:** Scan commits to SQLite in batches. Failures are recoverable — the next scan picks up automatically.
- **`revalidate` not `force`:** The parameter name communicates intent (re-check correctness, not reset decisions).
- **`skip_issue_types`:** Allows the agent to exclude noisy issue types (e.g., `GENRE_SET`) after confirming with the user.
- **Rename execution included:** Simple renames (suffix stripping, tech-spec removal) handled server-side with import-status pre-check. Imported-file renames deferred until Rekordbox XML path update is validated.
- **File-relocation-staging removed:** The standalone file-relocation-staging spec was deleted. Its critical open question (can XML import update paths?) was answered: no. Rename execution for un-imported files is handled by this spec's `auto_fix` operation. Imported file renames are blocked.
- **Report export:** Server-side markdown generation via `export_report`. Keeps large issue lists out of the agent's context.
- **SOP refactor:** After this spec is implemented and validated on a real audit run.
- **Symlinks:** Skipped during filesystem traversal. Audio files should not be symlinks.
