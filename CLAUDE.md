# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
cargo build --release          # Binary: ./target/release/crate-dig (~12MB arm64)
cargo test                     # Unit tests only (44 tests, in-memory SQLite)
cargo test -- --ignored        # Integration tests (21 tests, real encrypted DB)
cargo test test_name           # Run a single test by name
```

Integration tests require a Rekordbox DB backup tarball. Set `REKORDBOX_TEST_BACKUP` or it defaults to `~/Library/Pioneer/rekordbox-backups/db_20260215_233936.tar.gz`. Missing tarball = tests silently skipped.

Register with Claude Code: `claude mcp add crate-dig ./target/release/crate-dig`

## Commit Messages

Use Conventional Commits for all commits:

```text
type(scope): short summary
```

Recommended types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `ci`, `build`.

Examples:
- `docs(rekordbox): orchestrate phase b verification`
- `fix(xml): preserve rekordbox URI encoding`

## Architecture

Single-crate Rust MCP server targeting Rekordbox 7.x, using rmcp 0.15. No CLI flags — stdio transport only, operated through Claude Code.

```
main.rs           Entry point (19 lines), stdio transport setup
tools.rs          All 13 MCP tool definitions + CrateDigServer + ServerState
db.rs             SQLCipher connection, all SQL queries, sample exclusion
types.rs          Shared types: Track, Playlist, TrackChange, LibraryStats, rating conversion
genre.rs          Genre taxonomy (35 genres), alias map (37 entries), normalize_genre()
changes.rs        ChangeManager — in-memory staging (Mutex<HashMap<String, TrackChange>>)
xml.rs            Rekordbox XML writer (template strings, no XML library)
discogs.rs        Discogs OAuth 1.0a PLAINTEXT client, rate-limited
beatport.rs       Beatport __NEXT_DATA__ HTML scraper, rate-limited
```

**Data flow:** Read from encrypted SQLCipher master.db → stage changes in memory → write Rekordbox-compatible XML for reimport. Never writes directly to the DB.

**State:** `CrateDigServer` is `Clone` (rmcp requirement), holds `Arc<ServerState>`. DB connection lazy-initialized via `OnceLock`. Changes held in `ChangeManager` until `write_xml` exports them.

## rmcp 0.15 Patterns

- `#[tool_router]` on impl block containing tool methods
- `#[tool_handler]` on `impl ServerHandler` (auto-generates list/call routing)
- Tool params: `Parameters<ConcreteStruct>` — MUST use the literal struct name, type aliases break schema generation
- Errors: `ErrorData::internal_error(msg, None)` for runtime errors, `McpError::invalid_params(msg, None)` for validation
- `CrateDigServer` must remain `Clone` — shared state goes in `Arc<ServerState>`

## Rekordbox Gotchas

- DB column is `Commnt` not `Comment` (typo in schema)
- XML rating values: 0/51/102/153/204/255 (not 0-5) — use `stars_to_rating()`/`rating_to_stars()` in types.rs
- `COLLECTION Entries` count in XML must match actual track count or import fails
- `DJPlayCount` is integer in real DB but sometimes text — `row_to_track()` handles both
- SQLCipher key: `PRAGMA key = '<passphrase>'` (NOT `x'<hex>'`)
- BPM stored as integer x100 (12800 = 128.00 BPM)
- All queries must filter `rb_local_deleted = 0` and exclude sampler path prefix
- LIKE wildcards (`%`, `_`) in user input must be escaped — use `escape_like()`

## SQL Patterns

All queries use dynamic building with indexed positional parameters (`?1`, `?2`, ...) and `Vec<Box<dyn ToSql>>` for bind values. The base `TRACK_SELECT` joins 7 tables via LEFT JOIN. Every query filters deleted tracks and sampler samples.
