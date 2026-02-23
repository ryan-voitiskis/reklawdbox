# Spec: Idempotent Audit Runs

## Problem

Re-running the collection-audit SOP re-scans everything and re-flags all issues, including ones already fixed or explicitly accepted. There's no memory of previous runs. This makes incremental auditing impractical on a 17,000+ file collection — every run is a full run.

## Requirements

1. Record audit state: files scanned, issues found, fixes applied, user decisions
2. Subsequent runs diff against prior state — only surface new files, changed files, or unresolved issues
3. Track user decisions ("accepted as-is", "deferred", "won't fix") so they don't resurface
4. Handle file renames/moves (same content, different path) without losing history
5. State must survive across sessions (persisted to disk)

## Proposed Approach

### State file

JSON file at a known location (e.g., `.reklawdbox/audit-state.json` or `docs/tmp/audit-state.json`):

```json
{
  "version": 1,
  "last_run": "2026-02-23T10:00:00Z",
  "scope": "/path/to/collection",
  "files": {
    "/path/to/file.flac": {
      "last_audited": "2026-02-23T10:00:00Z",
      "file_mtime": "2026-01-15T08:30:00Z",
      "file_size": 45678901,
      "issues": [
        {
          "type": "MISSING_YEAR",
          "status": "resolved",
          "resolution": "fixed",
          "resolved_at": "2026-02-23T10:05:00Z"
        },
        {
          "type": "GENRE_SET",
          "status": "accepted",
          "note": "User chose to keep Bandcamp genre",
          "resolved_at": "2026-02-23T10:06:00Z"
        }
      ]
    }
  }
}
```

### Diff logic

On each run:

1. Load prior state
2. Scan filesystem for current files in scope
3. Classify each file:
   - **New:** not in state file — full audit
   - **Changed:** mtime or size differs from state — re-audit
   - **Unchanged:** mtime and size match — skip unless `--force`
   - **Missing:** in state but not on disk — flag as removed
4. For re-audited files, compare new issues against prior issues — only surface genuinely new problems

### File identity

Use path as primary key. For renamed/moved files, an optional content hash (first 64KB) could match across paths, but this adds complexity. Start with path-only and accept that renames reset audit history for that file.

### Integration

The agent reads the state file at the start of the audit SOP and writes it at the end. No new MCP tools needed — the agent manages the file directly via Read/Write tools.

## Open Questions

- JSON vs SQLite for state storage? JSON is simpler and readable; SQLite scales better for 17,000+ entries.
- Should the state file live in the project (`.reklawdbox/`) or alongside the collection?
- How to handle scope changes (audited `/collection/ArtistA/` last time, now auditing all of `/collection/`)?
- Should audit state be exportable as a report (markdown summary of collection health)?
