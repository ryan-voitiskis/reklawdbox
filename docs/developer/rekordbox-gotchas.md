# Rekordbox Gotchas

Implementation-specific gotchas for reklawdbox development. For the broader list of Rekordbox platform gotchas (XML import, file paths, general), see [rekordbox-internals.md § Gotchas and Footguns](../reference/rekordbox-internals.md#10-gotchas-and-footguns).

## Database

- DB column is `Commnt` not `Comment` (typo in schema)
- BPM stored as integer x100 (`12800` = `128.00 BPM`)
- `DJPlayCount` is integer in real DB but sometimes text — `row_to_track()` handles both
- SQLCipher key: `PRAGMA key = '<passphrase>'` (not `x'<hex>'`)
- All IDs are `VARCHAR(255)` strings, not integers

## Queries

- All queries must filter `rb_local_deleted = 0` and exclude sampler path prefix
- LIKE wildcards (`%`, `_`) in user input must be escaped — use `escape_like()`
- All queries use dynamic building with indexed positional parameters (`?1`, `?2`, ...) and `Vec<Box<dyn ToSql>>` for bind values
- The base `TRACK_SELECT` joins 7 tables via `LEFT JOIN`; every query filters deleted tracks and sampler samples

## XML

- XML rating values: `0/51/102/153/204/255` (not `0-5`) — use `stars_to_rating()` / `rating_to_stars()` in `src/types.rs`
- `COLLECTION Entries` count in XML must match actual track count or import fails
