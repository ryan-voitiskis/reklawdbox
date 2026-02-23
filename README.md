# reklawdbox

MCP server for Rekordbox 7.x library management. Reads directly from the encrypted master.db,
stages metadata changes in memory, and writes Rekordbox-compatible XML for safe reimport.

Built as a single Rust binary. Primary operation is through an MCP host (Codex, Claude Code,
etc.), with an optional `analyze` CLI subcommand for local batch audio analysis.

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

Deprecated Discogs fallback (not the default path):

- `REKLAWDBOX_DISCOGS_KEY`
- `REKLAWDBOX_DISCOGS_SECRET`
- `REKLAWDBOX_DISCOGS_TOKEN`
- `REKLAWDBOX_DISCOGS_TOKEN_SECRET`
- `REKLAWDBOX_DISCOGS_API_BASE_URL` (optional custom Discogs API base URL)

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

## Optional CLI: Batch Audio Analysis

The binary runs MCP server mode by default. Use the `analyze` subcommand for local batch analysis
and cache priming outside your MCP host:

```bash
./target/release/reklawdbox analyze --max-tracks 200
```

Example with filters:

```bash
./target/release/reklawdbox analyze --playlist <playlist_id> --genre Techno --bpm-min 126 --bpm-max 134
```

### Essentia Setup (Recommended)

Use the repo script to install Essentia into the default probe location:

```bash
bash scripts/setup-essentia.sh
```

Then set `CRATE_DIG_ESSENTIA_PYTHON` in `.mcp.json` to:

```text
/Users/<you>/.local/share/reklawdbox/essentia-venv/bin/python
```

Restart the MCP host/server after updating config.

## Discogs Auth Flow

1. Configure `REKLAWDBOX_DISCOGS_BROKER_URL` and `REKLAWDBOX_DISCOGS_BROKER_TOKEN` (default broker mode requires token auth; only omit if broker explicitly enables unauthenticated local-dev mode).
2. Call `lookup_discogs` for any track.
3. If auth is missing, the tool returns an actionable message with an `auth_url`.
4. Open the `auth_url`, approve Discogs access, then run `lookup_discogs` again.
5. The broker session token is stored in local internal SQLite; Discogs OAuth secrets remain broker-side only.

## Tools

<!-- dprint-ignore -->
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
| `enrich_tracks` | Batch enrich tracks via Discogs/Beatport using IDs, playlist, or filters |
| `analyze_track_audio` | Analyze one track with stratum-dsp and optional Essentia (cached) |
| `analyze_audio_batch` | Batch audio analysis with stratum-dsp and optional Essentia (cached) |
| `setup_essentia` | Install/validate Essentia in a local venv and activate it for the running server |
| `score_transition` | Score a single transition between two tracks (key/BPM/energy/genre/rhythm) |
| `build_set` | Generate 2-3 candidate set orderings from a track pool |
| `resolve_track_data` | Return all cached + staged data for one track without external calls |
| `resolve_tracks_data` | Batched `resolve_track_data` over IDs, playlist, or search scope |
| `cache_coverage` | Report enrichment/audio cache completeness for a selected track scope |

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

For enrichment/audio/set workflows, the common sequence is:

1. **Scope tracks** — `search_tracks`, `get_playlist_tracks`, or `resolve_tracks_data`
2. **Populate cache** — `enrich_tracks` and/or `analyze_audio_batch`
3. **Inspect completeness** — `resolve_track_data`/`resolve_tracks_data` and `cache_coverage`
4. **Plan transitions/sets** — `score_transition` and `build_set`

## Documentation

- [reklawdbox.com](https://reklawdbox.com) — Astro Starlight docs site
- [`docs/README.md`](docs/README.md) — Documentation index by area
- [`docs/rekordbox/README.md`](docs/rekordbox/README.md) — Rekordbox corpus map and manifest-first usage
- [`docs/reference/rekordbox-internals.md`](docs/reference/rekordbox-internals.md) — Rekordbox file formats, database schema, XML structure, ecosystem tools
- [`docs/developer/rekordbox-gotchas.md`](docs/developer/rekordbox-gotchas.md) — Rekordbox schema/XML edge cases and invariants used by code paths
- [`docs/developer/sql-patterns.md`](docs/developer/sql-patterns.md) — Query-building and binding patterns used in DB access code
- [`docs/operations/runbooks/backup-and-restore.md`](docs/operations/runbooks/backup-and-restore.md) — Backup usage and restore procedures
- [`docs/integrations/discogs/auth.md`](docs/integrations/discogs/auth.md) — Discogs broker setup, first-run auth, and re-auth/reset guidance
- [`docs/integrations/discogs/auth-plan.md`](docs/integrations/discogs/auth-plan.md) — Discogs broker architecture decisions and phased implementation plan
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Development workflow, testing expectations, and pull request checklist
- [`SECURITY.md`](SECURITY.md) — Supported versions and vulnerability reporting process
- [`AGENTS.md`](AGENTS.md) — Agent/operator workflow notes for Codex and compatible hosts
- [`CLAUDE.md`](CLAUDE.md) — Claude Code-specific operator/developer workflow notes
