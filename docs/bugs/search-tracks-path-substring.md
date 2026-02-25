# Bug: `search_tracks` path filter uses substring matching

## Summary

`search_tracks(path=...)` uses `LIKE '%path%'` (substring match) when directory-scoping use cases need prefix matching (`LIKE 'path%'`).

## Location

`src/db.rs:210-213` in `apply_search_filters()`:

```rust
if let Some(ref path) = params.path {
    let idx = bind_values.len() + 1;
    sql.push_str(&format!(" AND c.FolderPath LIKE ?{idx} ESCAPE '\\'"));
    bind_values.push(Box::new(format!("%{}%", escape_like(path))));
}
```

## Impact

- **Low severity for typical use** — single-root libraries produce identical results with either matching strategy.
- **Medium severity for batch operations** — `enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`, `cache_coverage` all pass `path` through to `search_tracks`. Scoping to a directory could pull in tracks from unintended paths if the DB contains multiple roots (confirmed possible via Windows migration paths per `docs/reference/rekordbox-internals.md`).
- **No test coverage** for the path filter.

## Not Affected

- `exclude_samples` uses `SAMPLER_PATH_FRAGMENT` (`/rekordbox/Sampler/`) with substring matching — this is correct because the fragment is globally unique and must match across different home directories.
- Audio analysis cache in `store.rs` uses exact path matching (`file_path = ?`).

## Related

- Minor inconsistency: `cache_coverage` in `tools.rs:3101` uses the legacy `SAMPLER_PATH_PREFIX` (hardcoded `/Users/vz/Music/rekordbox/Sampler/`) with prefix matching, while `apply_search_filters` uses the portable `SAMPLER_PATH_FRAGMENT` with substring matching. The `apply_search_filters` version is more correct.

## Recommended Fix

Non-breaking additive change:

1. Add `path_prefix: Option<String>` to `SearchTracksParams` described as "Filter to tracks whose file path starts with this prefix (directory scoping)."
2. Implement as `LIKE 'prefix%'` (no leading `%`).
3. Keep existing `path` as substring match for backward compatibility — its description already says "partial match."
4. Audit engine and batch tools should prefer `path_prefix` when scoping to a directory.
5. Add tests for both `path` and `path_prefix` filters.
