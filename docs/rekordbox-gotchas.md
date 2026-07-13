# Rekordbox Gotchas

Implementation-specific gotchas for reklawdbox development. For the broader list of Rekordbox platform gotchas (XML import, file paths, general), see [rekordbox-internals.md § Gotchas and Footguns](rekordbox-internals.md#9-gotchas-and-footguns).

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

## Reload Tag

- Rekordbox's Reload Tag re-reads file tags into the library DB — useful for bulk metadata updates without XML import
- **WAV files:** Only RIFF INFO is read. Supported fields: artist, title, album, genre, year, comment. That's it.
- **Label/publisher is not in RIFF INFO** — cannot be set via file tags on WAV. Use XML import or manual entry.
- After reload, any manually edited track info in Rekordbox is **replaced** by the file tag values
- FLAC/MP3/M4A/AIFF have richer tag support (including label via publisher field)

## XML

- XML rating values: `0/51/102/153/204/255` (not `0-5`) — use `stars_to_rating()` / `rating_to_stars()` in `src/domain/library/rating.rs`
- `COLLECTION Entries` count in XML must match actual track count or import fails
