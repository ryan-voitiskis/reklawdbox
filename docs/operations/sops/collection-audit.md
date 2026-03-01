# Collection Audit — Agent SOP

Detect and fix naming/tagging convention violations in a music collection. Follow step-by-step.

**Goal:** After this SOP, any track file can be imported into Rekordbox with Artist, Title, Album, Year, Track Number, and Label displaying correctly. Genre is handled separately via the [genre classification SOP](genre-classification.md).

**Conventions:** All directory structure, filename, and tag conventions are defined in [conventions.md](../conventions.md). This SOP references those conventions but does not redefine them.

## Constraints

- **Read-only by default.** No files modified until user approves a fix plan.
- **Stop on ambiguity.** If the correct fix isn't clear, flag for user review — never guess.
- **Verify after fixing.** Re-scan after every fix batch.
- **Never delete audio files.** Renaming and tag editing only.
- **WAV dual-tag rule.** `write_file_tags` handles this automatically (default `wav_targets: ["id3v2", "riff_info"]`). WAV tag layers are written sequentially, not atomically — if a write fails partway through, the file may have inconsistent tags across layers. If a WAV write reports an error, treat the file's tag state as unknown and re-verify with `read_file_tags` before retrying.
- **Rekordbox path awareness.** Before any rename, **manually check** whether the file is imported in Rekordbox (e.g., `search_tracks` by path). Imported files must **never be renamed** — Rekordbox cannot update paths via XML import (creates duplicates). The user must manually relocate via Rekordbox after any external rename. The audit engine does not enforce this automatically.

## Prerequisites

<!-- dprint-ignore -->
| Tool             | Purpose                                     | Required? |
| ---------------- | ------------------------------------------- | --------- |
| `reklawdbox` MCP | Audit engine, tag tools, DB queries, lookups | Yes       |

No external tools (exiftool, kid3-cli) are needed. All tag reads, writes, and convention checks are handled by the `audit_state` and `write_file_tags` MCP tools.

---

## Step 0: Scope & Issue Type Selection

### 0a: Choose scope

Ask the user what to audit:

1. Entire collection directory
2. Specific artist directory
3. Specific album
4. Specific play subdirectory
5. Custom path

### 0b: Choose issue types

Present the full issue type list and ask which to include. `GENRE_SET` in particular can produce thousands of flags — confirm before including:

> "Skip `GENRE_SET` for this pass, or include it? (Genre tags are convention violations but there may be thousands.)"

Record the user's choice as `skip_issue_types` for the scan.

### 0c: Assess scope size

```
audit_state(get_summary, scope="/path/to/scope/")
```

If this is the first scan, the summary will be empty. For large scopes (full collection), inform the user the first scan may take tens of seconds.

---

## Step 1: Scan

```
audit_state(scan, scope="/path/to/scope/", skip_issue_types=["GENRE_SET"])
```

Review the returned summary:

```json
{
  "files_in_scope": 1342,
  "scanned": 1342,
  "skipped_unchanged": 0,
  "missing_from_disk": 0,
  "skipped_issue_types": ["GENRE_SET"],
  "new_issues": { "EMPTY_ARTIST": 3, "WAV_TAG3_MISSING": 12 },
  "total_open": 45
}
```

If `total_open` is 0, skip to Step 6 (Final Report).

---

## Step 2: Review Issues

### 2a: Query by type

Query open issues grouped by type to understand the landscape:

```
audit_state(query_issues, scope="/path/to/scope/", status="open", issue_type="WAV_TAG3_MISSING")
audit_state(query_issues, scope="/path/to/scope/", status="open", issue_type="EMPTY_ARTIST")
```

### 2b: Triage by safety tier

Categorize the open issues:

<!-- dprint-ignore -->
| Category       | Safety tier  | Issue types                                                    | Action        |
| -------------- | ------------ | -------------------------------------------------------------- | ------------- |
| Auto-fixable   | Safe         | `WAV_TAG3_MISSING`, `WAV_TAG_DRIFT`, `ARTIST_IN_TITLE`        | → Step 3      |
| Rename-fixable | Rename-safe  | `ORIGINAL_MIX_SUFFIX`, `TECH_SPECS_IN_DIR`                    | → Step 3      |
| Needs review   | Review       | All others                                                     | → Step 4      |

Present the triage summary to the user:

> "Found 12 WAV tag 3 issues (auto-fixable), 2 filename suffixes (rename-safe, un-imported), 3 empty artist tags (needs review). Start with auto-fixes?"

---

## Step 3: Fix Safe Issues

### 3a: Apply tag fixes

For safe-tier issues (`WAV_TAG3_MISSING`, `WAV_TAG_DRIFT`, `ARTIST_IN_TITLE`), apply fixes directly via `write_file_tags`:

```
write_file_tags(writes=[{path: "/path/to/file.wav", tags: {artist: "Fixed Artist", title: "Fixed Title"}}])
```

Present the planned writes to the user for approval before executing.

### 3b: Verify fixes via re-scan

After applying fixes, re-scan the scope. The scan automatically detects changed files (new mtime) and marks resolved issues as `fixed`:

```
audit_state(scan, scope="/path/to/scope/")
```

Do **not** manually resolve with `resolution="fixed"` — that value is reserved for auto-resolution by the scan engine and will be rejected at runtime. To manually resolve issues the user wants to skip, use `accepted_as_is`, `wont_fix`, or `deferred`.

---

## Step 4: Review-Tier Issues

These require human judgment. Present each issue type in batches.

### 4a: Empty/missing tags

For `EMPTY_ARTIST`, `EMPTY_TITLE`, `MISSING_TRACK_NUM`, `MISSING_ALBUM`, `MISSING_YEAR`:

1. Query the issues:
   ```
   audit_state(query_issues, scope="/path/to/scope/", status="open", issue_type="EMPTY_ARTIST")
   ```
2. For each issue, check the `detail` field — it may be `null` for empty/missing-tag issues. When `detail` is absent, infer the fix yourself from the filename and parent directory structure. Present to user:
   > "File `01 Unknown - Track.wav` has empty artist. Filename suggests: `Unknown`. Accept?"
3. If user approves, apply via `write_file_tags`:
   ```
   write_file_tags(writes=[{path: "/path/to/file.wav", tags: {artist: "Unknown"}}])
   ```
4. If the fix needs external data (missing album/year), use lookups:
   ```
   lookup_discogs(track_id="...")
   lookup_beatport(track_id="...")
   ```
5. Record user decisions for items they want to skip:
   ```
   audit_state(resolve_issues, issue_ids=[...], resolution="accepted_as_is", note="Intentionally blank")
   audit_state(resolve_issues, issue_ids=[...], resolution="deferred", note="Need to research")
   ```

### 4b: Filename issues

For `BAD_FILENAME`, `FILENAME_TAG_DRIFT`, `MISSING_YEAR_IN_DIR`:

1. Present the mismatch details from the `detail` JSON
2. Ask user which is correct (filename vs tags, or neither)
3. For tag-based fixes, use `write_file_tags`
4. For rename-based fixes on un-imported files, rename manually or defer
5. For imported files that need renaming, record as deferred with a note:
   ```
   audit_state(resolve_issues, issue_ids=[...], resolution="deferred",
     note="Needs rename but file is imported — manual Rekordbox relocate required")
   ```

### 4c: Genre tags

For `GENRE_SET` (if not skipped):

1. Present files with existing genre tags
2. For each, ask user: keep as-is, clear, or migrate to comments
3. Apply:
   - Keep: `audit_state(resolve_issues, issue_ids=[...], resolution="accepted_as_is")`
   - Clear: `write_file_tags(writes=[{path: "...", tags: {genre: null}}])`
   - Migrate: First read the existing `comment` value via `read_file_tags`. If non-empty, **prepend** the genre to preserve it (e.g., `"Genre: Deep House | <existing comment>"`). Then write:
     `write_file_tags(writes=[{path: "...", tags: {comment: "Genre: Deep House | <existing>", genre: null}}])`
     If the existing comment is empty, write the genre string directly. **Never blindly overwrite** — `write_file_tags` replaces the field value, it does not append.

### 4d: No-tag files

For `NO_TAGS`:

1. Infer metadata from parent directory name, companion files, filename
2. Present inferred values to user for confirmation
3. Apply via `write_file_tags` after approval

---

## Step 5: Verification Scan

Re-scan the entire scope to confirm all fixes and capture the final state:

```
audit_state(scan, scope="/path/to/scope/")
```

Review the summary. If `total_open` > 0, return to Step 2 for remaining issues or confirm with user that remaining items are intentionally deferred.

---

## Step 6: Final Report

```
audit_state(get_summary, scope="/path/to/scope/")
```

Present the summary to the user: scope, files scanned, pass rate, fixes applied by type, remaining deferred/accepted items, and recommended next steps (Rekordbox import for new files, genre classification SOP for ungenred tracks).

---

## Appendix A: Issue Type Reference

<!-- dprint-ignore -->
| Issue type            | Context      | Safety tier  | Detection                                             | Fix method                    |
| --------------------- | ------------ | ------------ | ----------------------------------------------------- | ----------------------------- |
| `EMPTY_ARTIST`        | All files    | Review       | `artist` field empty/null                             | Parse from filename           |
| `EMPTY_TITLE`         | All files    | Review       | `title` field empty/null                              | Parse from filename           |
| `MISSING_TRACK_NUM`   | Album tracks | Review       | `track` field empty/null                              | Parse from filename           |
| `MISSING_ALBUM`       | Album tracks | Review       | `album` field empty/null                              | Directory name or Discogs     |
| `MISSING_YEAR`        | Album tracks | Review       | `year`/`date` both empty/null                         | Discogs lookup                |
| `ARTIST_IN_TITLE`     | All files    | Safe         | `title` starts with `{artist} - `                     | Strip prefix                  |
| `WAV_TAG3_MISSING`    | WAV only     | Safe         | WAV file with `tag3_missing` non-empty                | Copy from tag 2               |
| `WAV_TAG_DRIFT`       | WAV only     | Safe         | WAV `id3v2` and `riff_info` values differ             | Sync to tag 2                 |
| `GENRE_SET`           | All files    | Review       | `genre` field non-empty                               | User decides keep/clear/migrate |
| `NO_TAGS`             | All files    | Review       | All 14 tag fields empty/null                          | Infer from filename/dir       |
| `BAD_FILENAME`        | All files    | Review       | Filename doesn't match canonical or alternates        | User review                   |
| `ORIGINAL_MIX_SUFFIX` | All files    | Rename-safe  | Filename contains `(Original Mix)`                    | Strip suffix (if not imported)|
| `TECH_SPECS_IN_DIR`   | Directories  | Rename-safe  | Directory contains `[FLAC]`, `[WAV]`, `24-96`, etc.  | Strip from dir name           |
| `MISSING_YEAR_IN_DIR` | Album dirs   | Review       | Album directory missing `(YYYY)` suffix               | Discogs lookup                |
| `FILENAME_TAG_DRIFT`  | All files    | Review       | Filename artist/title disagrees with tag values       | User review                   |
