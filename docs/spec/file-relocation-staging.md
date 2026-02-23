# Spec: Staged File Moves for Rekordbox

## Problem

Renaming or moving files breaks Rekordbox's internal path references. After a move, Rekordbox shows the track as "missing" until the user manually relocates it (right-click > Relocate). This makes the collection-audit SOP's rename/move operations destructive to Rekordbox state.

Currently the SOP has no awareness of whether a file is imported in Rekordbox. It renames freely, leaving the user to manually fix broken references afterward.

## Requirements

1. Before any rename/move, check if the file exists in Rekordbox's DB (by `Location` path)
2. If imported: warn the agent/user that the move will break Rekordbox references
3. Stage moves in memory (like `ChangeManager` does for metadata) — don't execute until approved
4. Export a relocation mapping (old path -> new path) that can be used to fix references
5. Support both renames (same directory) and moves (different directory)

## Proposed Approach

### Detection

New MCP tool or extension to `search_tracks`:

```
search_tracks(path="/exact/path/to/file.flac")
```

If a result is returned, the file is imported in Rekordbox. The `Location` field in the DB contains the file:// URI.

### Staging

Add a `RelocationManager` (parallel to `ChangeManager`):

```rust
struct RelocationManager {
    pending: Mutex<HashMap<String, PendingMove>>,  // track_id -> move
}

struct PendingMove {
    old_path: PathBuf,
    new_path: PathBuf,
    track_id: Option<String>,  // None if not in Rekordbox
}
```

MCP tools:
- `stage_move(old_path, new_path)` — stages a file move, auto-checks Rekordbox import status
- `preview_moves()` — shows all staged moves with Rekordbox impact
- `execute_moves()` — performs the file moves
- `export_relocation_map()` — writes old->new mapping for Rekordbox fixup

### Rekordbox path fixup

**Option A: XML with updated paths.** Write an XML where `Location` attributes use the new paths. If Rekordbox XML import can update existing tracks' paths (not just add new tracks), this would fix references automatically.

**Option B: Relocation CSV/JSON.** Export the mapping for manual use with Rekordbox's relocate feature, or a future automation tool.

## Open Questions

- **Critical:** Can Rekordbox XML import update file paths for existing tracks, or does it only add new entries? This determines whether Option A is viable. Needs testing.
- Should `execute_moves` also update the Rekordbox DB path in the export XML automatically?
- How to handle moves where the target path already exists (collision)?
- Should this integrate with `write_xml`, or be a separate export step?
