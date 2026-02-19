# reklawdbox

MCP server for Rekordbox 7.x library management. Reads directly from the encrypted master.db,
stages metadata changes in memory, and writes Rekordbox-compatible XML for safe reimport.

Built as a single static Rust binary with zero runtime dependencies. Operated through
Claude Code — no web UI, no CLI flags, just MCP.

## Build

```bash
cargo build --release
```

The binary is at `./target/release/reklawdbox` (~12 MB, arm64).

## Register with Claude Code

```bash
claude mcp add reklawdbox ./target/release/reklawdbox
```

The server auto-detects the Rekordbox database at `~/Library/Pioneer/rekordbox/master.db`.
To override, set the `REKORDBOX_DB_PATH` environment variable.

## Tools

| Tool | Description |
|------|-------------|
| `read_library` | Get library summary: track count, genre distribution, stats |
| `search_tracks` | Search and filter tracks in the Rekordbox library |
| `get_track` | Get full details for a specific track by ID |
| `get_playlists` | List all playlists with track counts |
| `get_playlist_tracks` | List tracks in a specific playlist |
| `get_genre_taxonomy` | Get the configured genre taxonomy |
| `update_tracks` | Stage changes to track metadata (genre, comments, rating, color) |
| `preview_changes` | Preview all staged changes, showing what will differ from current state |
| `write_xml` | Write staged changes to a Rekordbox-compatible XML file |
| `clear_changes` | Clear staged changes for specific tracks or all |
| `suggest_normalizations` | Analyze genres and suggest normalizations to canonical taxonomy |
| `lookup_discogs` | Look up a track on Discogs for genre/style enrichment |
| `lookup_beatport` | Look up a track on Beatport for genre/BPM/key enrichment |

## Response Contract Notes

- `write_xml` returns a JSON payload on both write and no-change paths.
- The no-change path includes `"message": "No changes to write."` with `track_count`, `changes_applied`, and provenance fields.
- Legacy consumers that previously parsed plain text should read the `message` field from the JSON payload.

## Genre Taxonomy

Starter set for consistency (not a closed list — arbitrary genres are accepted):

Acid, Afro House, Ambient, Ambient Techno, Bassline, Breakbeat, Broken Beat,
Dancehall, Deep House, Deep Techno, Disco, Downtempo, Drum & Bass, Dub,
Dub Techno, Dubstep, Electro, Experimental, Garage, Grime, Hard Techno,
Hip Hop, House, IDM, Jungle, Minimal, Psytrance, R&B, Reggae, Speed Garage,
Synth-pop, Tech House, Techno, Trance, UK Bass

## Workflow

1. **Search** — use `search_tracks` or `get_playlist_tracks` to find tracks to tag
2. **Update** — use `update_tracks` to stage genre, comments, rating, or color changes
3. **Preview** — use `preview_changes` to review what will change vs. current state
4. **Write** — use `write_xml` to generate the XML file (runs backup automatically)
5. **Import in Rekordbox** — File > Import > Import Playlist/Collection, select the XML

## Documentation

- [`docs/rekordbox-internals.md`](docs/rekordbox-internals.md) — Rekordbox file formats, database schema, XML structure, ecosystem tools
- [`docs/backup-and-restore.md`](docs/backup-and-restore.md) — Backup usage and restore procedures
