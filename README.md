# reklawdbox

MCP server for Rekordbox 7.x library management. Reads directly from the encrypted master.db,
stages metadata changes in memory, and writes Rekordbox-compatible XML for safe reimport.

Built as a single Rust binary. Primary operation is through an MCP host (Codex, Claude Code,
etc.), with CLI subcommands for local batch audio analysis and native tag read/write.

### Why MCP as the primary interface?

A shell-invoked CLI works from hosts with terminal access, but Claude Desktop and other MCP hosts
**cannot execute shell commands** and can only call MCP tools. MCP keeps reklawdbox usable from
any compliant host while still allowing local CLI workflows when needed.

## Build

```bash
cargo build --release
```

The binary is at `./target/release/reklawdbox`.

## Development

Common local development and validation commands:

```bash
cargo build --release
cargo test
cargo test -- --ignored
bash docs/rekordbox/validate-corpus.sh
python3 docs/rekordbox/verify-phase-b.py
```

Agent workflow notes:

- Generic/Codex agents: [`AGENTS.md`](AGENTS.md)
- Claude Code: [`CLAUDE.md`](CLAUDE.md)
- Repo docs index: [`docs/README.md`](docs/README.md)

## MCP Host Setup

- Configure your MCP host to run this server over stdio with command `./target/release/reklawdbox`.
- Use `mcp-config.example.json` as the baseline for local host configuration.
- Keep real credentials in local environment variables or untracked local config only.

The server auto-detects the Rekordbox database at `~/Library/Pioneer/rekordbox/master.db`.
To override, set the `REKORDBOX_DB_PATH` environment variable.

Optional enrichment and analysis environment variables:

- `REKLAWDBOX_DISCOGS_BROKER_URL`
- `REKLAWDBOX_DISCOGS_BROKER_TOKEN`
- `CRATE_DIG_ESSENTIA_PYTHON`
- `CRATE_DIG_STORE_PATH` (optional override for internal cache SQLite path)

Essentia probe behavior:

- The server only probes `CRATE_DIG_ESSENTIA_PYTHON` and `~/.local/share/reklawdbox/essentia-venv/bin/python`.
- If neither imports Essentia, tools report Essentia as unavailable and continue with stratum-dsp only.
- Probe result is memoized for process lifetime, so restart the MCP server after changing Essentia install/config (or run `setup_essentia`, which installs and activates Essentia immediately).

### Codex Quickstart

1. Build the binary:

```bash
cargo build --release
```

2. (Recommended) Set up a persistent local env file + launcher (one-time):

```bash
mkdir -p ~/.config/reklawdbox
cp mcp.env.example ~/.config/reklawdbox/mcp.env
# edit ~/.config/reklawdbox/mcp.env for your machine
chmod +x scripts/run-reklawdbox-mcp.sh
```

3. Register once with the launcher script:

```bash
codex mcp remove reklawdbox 2>/dev/null || true
codex mcp add reklawdbox -- ./scripts/run-reklawdbox-mcp.sh
```

After this, you only update `~/.config/reklawdbox/mcp.env` and restart MCP when env changes.

4. Alternative: create local MCP config from template:

```bash
cp mcp-config.example.json .mcp.json
```

5. Edit `.mcp.json` and set:

- `REKORDBOX_DB_PATH` (if you are not using the default Rekordbox path)
- optional broker Discogs / Essentia env vars

6. Register or load that config in your Codex MCP host so it starts:

- command: `./target/release/reklawdbox`
- transport: `stdio`

7. Verify wiring by running a simple tool call from Codex (for example `read_library`).

## Optional CLI Subcommands

The binary runs MCP server mode by default. Subcommands are available for local workflows
outside your MCP host.

### Batch Enrichment + Analysis

```bash
./target/release/reklawdbox hydrate --max-tracks 200
./target/release/reklawdbox hydrate --providers discogs,beatport,analysis --playlist <playlist_id> --cpu overnight -y
```

### Batch Audio Analysis

```bash
./target/release/reklawdbox analyze --max-tracks 200
./target/release/reklawdbox analyze --playlist <playlist_id> --genre Techno --bpm-min 126 --bpm-max 134
```

### Tag Read/Write

Read, write, and manage metadata tags directly on audio files (FLAC, MP3, WAV, M4A, AAC, AIFF).

```bash
# Read tags (human-readable or --json)
./target/release/reklawdbox read-tags track.flac
./target/release/reklawdbox read-tags /music/album/ --fields artist,title,bpm --json

# Write tags (with dry-run preview)
./target/release/reklawdbox write-tags track.mp3 --artist "New Artist" --year 2026
./target/release/reklawdbox write-tags track.wav --genre Techno --wav-targets id3v2,riff_info --dry-run

# Cover art
./target/release/reklawdbox extract-art track.flac --output cover.jpg
./target/release/reklawdbox embed-art cover.jpg track1.mp3 track2.flac
```

WAV files support dual-layer tagging (ID3v2 + RIFF INFO). Use `--wav-targets` to control
which layers are written. See [`site/src/content/docs/reference/tools.mdx`](site/src/content/docs/reference/tools.mdx)
for full tool parameter reference.

### Setup (Recommended)

Install Essentia and configure MCP hosts:

```bash
reklawdbox setup
```

This creates a managed Python venv at `~/.local/share/reklawdbox/essentia-venv` (auto-detected, no env vars needed), writes `~/Music/.mcp.json` for Claude Code (scoped to music sessions only), and configures Claude Desktop if installed.

Start Claude Code from `~/Music` to access reklawdbox tools:

```bash
cd ~/Music && claude
```

### Disconnect Broker

Clear the stored Discogs broker session (forces re-auth on next lookup):

```bash
reklawdbox disconnect-broker
```

## Discogs Auth Flow

1. Call `lookup_discogs` for any track. The built-in broker is preconfigured — no env vars needed.
2. On first use, the tool returns an actionable message with an `auth_url`.
3. Open the `auth_url`, approve Discogs access, then run `lookup_discogs` again.
4. The broker session token is stored in local internal SQLite; Discogs OAuth secrets remain broker-side only.

## Tools

<!-- dprint-ignore -->
| Tool | Description |
|------|-------------|
| **Library & Data** | |
| `read_library` | Get library summary: track count, genre distribution, stats |
| `search_tracks` | Search and filter tracks in the Rekordbox library |
| `get_track` | Get full details for a specific track by ID |
| `get_playlists` | List all playlists with track counts |
| `get_playlist_tracks` | List tracks in a specific playlist |
| `get_sessions` | List recent DJ sessions from Rekordbox play history |
| `get_session_tracks` | Get the ordered track list for a specific DJ session |
| `get_play_stats` | Get per-track play statistics scoped by search filters |
| `get_genre_taxonomy` | Get the configured genre taxonomy |
| `resolve_track_data` | Return all cached + staged data for one track (cache-only) |
| `resolve_tracks_data` | Batched `resolve_track_data` over IDs, playlist, or search scope |
| `cache_coverage` | Report enrichment/audio cache completeness for a selected track scope |
| **Enrichment & Analysis** | |
| `lookup_discogs` | Look up a track on Discogs for genre/style enrichment |
| `lookup_beatport` | Look up a track on Beatport for genre/BPM/key enrichment |
| `lookup_musicbrainz` | Look up a track on MusicBrainz for year/label data |
| `lookup_bandcamp` | Look up a track on Bandcamp for year/label/tags data |
| `enrich_tracks` | Batch enrich tracks via Discogs/Beatport/Bandcamp using IDs, playlist, or filters |
| `analyze_track_audio` | Analyze one track with stratum-dsp and optional Essentia (cached) |
| `analyze_audio_batch` | Batch audio analysis with stratum-dsp and optional Essentia (cached) |
| `setup_essentia` | Install/validate Essentia in a local venv and activate it for the running server |
| **Classification & Staging** | |
| `suggest_normalizations` | Analyze genres and suggest normalizations to canonical taxonomy |
| `classify_tracks` | Apply genre decision tree to ungenred tracks with confidence levels |
| `audit_genres` | Verify existing genre tags against enrichment and audio evidence |
| `backfill_labels` | Auto-fill empty labels from enrichment caches |
| `backfill_years` | Auto-fill missing years from file tags, folder paths, and enrichment cache |
| `backfill_albums` | Auto-fill empty album names from file tags, folder paths, and enrichment cache |
| `update_tracks` | Stage changes to track metadata (genre, comments, rating, color, label, year, album) |
| `preview_changes` | Preview all staged changes, showing what will differ from current state |
| `write_xml` | Write staged changes to a Rekordbox-compatible XML file |
| `clear_changes` | Clear staged changes for specific tracks or all |
| **Mixing & Sequencing** | |
| `score_transition` | Score a single transition between two tracks (key/BPM/energy/genre/rhythm) |
| `query_transition_candidates` | Rank pool tracks as transition candidates from a reference track |
| `build_set` | Generate candidate set orderings from a track pool using beam search |
| `score_pool_compatibility` | Score pairwise, one-vs-pool, or cohesion compatibility between tracks |
| `expand_pool` | Expand a track pool by finding compatible additions from the library |
| `describe_pool` | Analyze a pool's compatibility, coverage, energy/BPM/key stats |
| `discover_pools` | Discover natural track pools via compatibility graph clique enumeration |
| `save_weight_preset` | Save a custom weight preset for reuse across sessions |
| `list_weight_presets` | List available weight presets (built-in and custom) |
| `delete_weight_preset` | Delete a custom saved weight preset |
| **Files & System** | |
| `read_file_tags` | Read metadata tags from audio files (FLAC, MP3, WAV, M4A, AAC, AIFF) |
| `write_file_tags` | Write/delete metadata tags on audio files with optional dry-run preview |
| `extract_cover_art` | Extract embedded cover art from an audio file to disk |
| `embed_cover_art` | Embed cover art into one or more audio files |
| `scan_broken_links` | Scan for tracks with missing audio files on disk |
| `scan_orphan_files` | Find audio files on disk not imported into Rekordbox |
| `scan_playlist_coverage` | Find tracks not assigned to any playlist |
| `scan_duplicates` | Detect duplicate tracks by metadata or exact file hash |
| `audit_state` | Collection audit engine: scan, query, resolve issues, get summary |
| `clear_caches` | Clear all caches and staged changes |
| `help` | Get step-by-step workflow SOPs |

## Response Contract Notes

- `write_xml` returns a JSON payload on both write and no-change paths.
- The no-change path includes `"message": "No changes to write."` with `track_count`, `changes_applied`, and provenance fields.
- Legacy consumers that previously parsed plain text should read the `message` field from the JSON payload.

## Genre Taxonomy

Starter set for consistency (not a closed list — arbitrary genres are accepted):

2-Step Garage, Acid, Afro House, Ambient, Ambient Techno, Bassline, Breakbeat,
Broken Beat, Dancehall, Deep House, Deep Techno, Disco, Downtempo, Drone Techno,
Drum & Bass, Dub, Dub Reggae, Dub Techno, Dubstep, EBM, Electro, Experimental,
Footwork, Future Garage, Gabber, Garage, Gospel House, Grime, Happy Hardcore,
Hard Techno, Hard Trance, Hardcore, Hardstyle, Highlife, Hip Hop, House, IDM,
Italo Disco, Jazz, Jungle, Minimal, Pop, Progressive House, Psytrance, R&B,
Reggae, Rock, Speed Garage, Synth-pop, Tech House, Techno, Trance, Trip-Hop,
UK Funky

## Workflow

1. **Search** — use `search_tracks` or `get_playlist_tracks` to find tracks to tag
2. **Update** — use `update_tracks` to stage genre, comments, rating, or color changes
3. **Preview** — use `preview_changes` to review what will change vs. current state
4. **Write** — use `write_xml` to generate the XML file (runs backup automatically)
5. **Import in Rekordbox** — File > Import > Import Playlist/Collection, select the XML

For enrichment/audio/set workflows, the common sequence is:

1. **Scope tracks** — `search_tracks`, `get_playlist_tracks`, or `resolve_tracks_data`
2. **Populate cache** — `enrich_tracks` and/or `analyze_audio_batch`
3. **Inspect completeness** — `resolve_track_data`/`resolve_tracks_data` and `cache_coverage`
4. **Plan transitions/sets** — `score_transition` and `build_set`

## Documentation

- [reklawdbox.com](https://reklawdbox.com) — Astro Starlight docs site
- [`docs/README.md`](docs/README.md) — Documentation index by area
- [`docs/rekordbox/README.md`](docs/rekordbox/README.md) — Rekordbox corpus map and manifest-first usage
- [`docs/conventions.md`](docs/conventions.md) — Collection naming, directory structure, and tagging conventions
- [`docs/rekordbox-internals.md`](docs/rekordbox-internals.md) — Rekordbox file formats, database schema, XML structure, ecosystem tools
- [`docs/rekordbox-gotchas.md`](docs/rekordbox-gotchas.md) — Rekordbox schema/XML edge cases and invariants used by code paths
- [`docs/rekordbox-gotchas.md#queries`](docs/rekordbox-gotchas.md#queries) — Query-building and binding patterns used in DB access code
- [`docs/backup-and-restore.md`](docs/backup-and-restore.md) — Backup usage and restore procedures
- [`README.md#discogs-auth-flow`](README.md#discogs-auth-flow) — Discogs broker setup, first-run auth, and re-auth/reset guidance
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Development workflow, testing expectations, and pull request checklist
- [`SECURITY.md`](SECURITY.md) — Supported versions and vulnerability reporting process
- [`AGENTS.md`](AGENTS.md) — Agent/operator workflow notes for Codex and compatible hosts
- [`CLAUDE.md`](CLAUDE.md) — Claude Code-specific operator/developer workflow notes
