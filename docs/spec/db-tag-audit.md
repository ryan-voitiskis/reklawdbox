# Spec: DB-Driven Tag Auditing

## Problem

The collection-audit SOP reads every file with exiftool to audit tags. This is slow (~5 min for 1,300 WAV files), redundant with data already in Rekordbox's DB, and fragile due to shell quoting and alias issues.

Rekordbox already ingests all tag metadata on import. For the ~17,000 imported tracks, the DB is a faster and more reliable source than re-reading files.

## Key Insight

Two distinct populations exist:

- **Imported tracks:** Rekordbox DB is source of truth. Use `search_tracks` / `resolve_tracks_data` for bulk metadata queries. File-level reads only needed for issues the DB can't detect (WAV tag 2/3 distinction, file-tag drift).
- **Un-imported tracks:** Files are the only source of truth. File-level tools (exiftool/kid3-cli, or future native tag tools) required.

## Requirements

1. Bulk metadata audit via DB queries — no per-file I/O for imported tracks
2. Cross-reference DB state against conventions (missing artist, empty album, etc.)
3. Fall back to file reads only for:
   - WAV tag 3 detection (DB doesn't distinguish tag 2 vs tag 3)
   - File-tag vs DB-tag drift detection (optional deep audit)
   - Un-imported files
4. Report should clearly separate DB-audited vs file-audited results

## Proposed Approach

### Phase 1: DB audit (bulk, fast)

Use existing MCP tools:

```
search_tracks(has_genre=false)           # tracks missing genre
search_tracks(artist="")                 # empty artist — if supported
resolve_tracks_data(playlist_id="...")   # full metadata for a scope
```

Build convention checks in the agent's logic:
- Missing required fields (artist, title, track, album, year)
- Artist-in-title pattern
- Filename/tag mismatch (DB has both `Location` path and tag values)

### Phase 2: File-level audit (targeted)

Only for issues the DB can't detect:
- WAV files: check RIFF INFO (tag 3) presence via `exiftool -s3 -RIFF:Artist`
- Un-imported files: full exiftool read
- Optional: compare file tags to DB values to detect drift

### Phase 3: Unified report

Merge DB and file audit results into the standard three-category report (auto-fixable, needs approval, needs investigation).

## Open Questions

- Can `search_tracks` filter on empty/null fields? If not, `resolve_tracks_data` with post-processing may be needed.
- Should file-tag drift detection be opt-in (deep audit) or always-on?
- How to handle tracks in the DB that no longer exist on disk (relocated/deleted)?
