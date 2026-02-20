---
title: Workflows
description: Repeatable procedures for common tagging and export tasks.
---

## Summary

Use these workflows to keep metadata updates consistent and auditable.

## Steps

1. Search candidate tracks (`search_tracks` or `get_playlist_tracks`).
2. Stage metadata updates (`update_tracks`).
3. Validate the staged delta (`preview_changes`).
4. Export XML (`write_xml`).
5. Import XML in Rekordbox and verify results.

## Examples

Genre normalization workflow:

- Run `suggest_normalizations` for a target playlist.
- Apply approved genre mappings with `update_tracks`.
- Re-check with `preview_changes` before writing XML.

## Related

- [Concepts](/concepts/)
- [Reference](/reference/)
- [Troubleshooting](/troubleshooting/)
