---
title: Concepts
description: Core operating model behind reklawdbox.
---

## Summary

reklawdbox reads Rekordbox data from the encrypted SQLite database, stages edits in memory, and writes Rekordbox-compatible XML for import.

## Steps

1. Read current library state with `read_library`, `search_tracks`, or playlist tools.
2. Stage edits with `update_tracks`.
3. Inspect pending edits with `preview_changes`.
4. Export XML with `write_xml`.
5. Import the XML back into Rekordbox.

## Examples

Safety model example:

- Source of truth remains Rekordbox `master.db`.
- Pending edits remain in memory until export.
- Output is a reversible XML handoff rather than direct DB mutation.

## Related

- [Workflows](/workflows/)
- [Reference](/reference/)
