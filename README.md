# reklawdbox

MCP server for Rekordbox 7.x library management. Reads directly from the encrypted master.db,
stages metadata changes in memory, and writes Rekordbox-compatible XML for safe reimport.

Built as a single static Rust binary with zero runtime dependencies. Operated through
an MCP host (Codex, Claude Code, etc.) — no web UI, no CLI flags, just MCP.

### Why MCP, not CLI?

A CLI binary called via shell would work fine from Claude Code (which has Bash access),
but Claude Desktop and other MCP hosts **cannot execute shell commands** — they can only
call MCP tools. MCP keeps reklawdbox usable from any compliant host.

## Build

```bash
cargo build --release
```

The binary is at `./target/release/reklawdbox` (~12 MB, arm64).

## Development

Common local development and validation commands:

```bash
cargo build --release
cargo test
cargo test -- --ignored
bash docs/rekordbox/validate-corpus.sh
python3 docs/rekordbox/verify-phase-b.py
```

Host-specific workflow notes:

- Codex: [`CODEX.md`](CODEX.md)
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

Essentia probe behavior:

- The server only probes `CRATE_DIG_ESSENTIA_PYTHON` and `~/.local/share/reklawdbox/essentia-venv/bin/python`.
- If neither imports Essentia, tools report Essentia as unavailable and continue with stratum-dsp only.
- Probe result is memoized for process lifetime, so restart the MCP server after changing Essentia install/config.

Deprecated Discogs fallback (not the default path):

- `REKLAWDBOX_DISCOGS_KEY`
- `REKLAWDBOX_DISCOGS_SECRET`
- `REKLAWDBOX_DISCOGS_TOKEN`
- `REKLAWDBOX_DISCOGS_TOKEN_SECRET`

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

1. Configure `REKLAWDBOX_DISCOGS_BROKER_URL` (and `REKLAWDBOX_DISCOGS_BROKER_TOKEN` if required by your broker).
2. Call `lookup_discogs` for any track.
3. If auth is missing, the tool returns an actionable message with an `auth_url`.
4. Open the `auth_url`, approve Discogs access, then run `lookup_discogs` again.
5. The broker session token is stored in local internal SQLite; Discogs OAuth secrets remain broker-side only.

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

- [reklawdbox.com](https://reklawdbox.com) — Astro Starlight docs site
- [`docs/README.md`](docs/README.md) — Documentation index by area
- [`docs/rekordbox/README.md`](docs/rekordbox/README.md) — Rekordbox corpus map and manifest-first usage
- [`docs/reference/rekordbox-internals.md`](docs/reference/rekordbox-internals.md) — Rekordbox file formats, database schema, XML structure, ecosystem tools
- [`docs/operations/runbooks/backup-and-restore.md`](docs/operations/runbooks/backup-and-restore.md) — Backup usage and restore procedures
- [`docs/integrations/discogs/auth.md`](docs/integrations/discogs/auth.md) — Discogs broker setup, first-run auth, and re-auth/reset guidance
- [`docs/integrations/discogs/auth-plan.md`](docs/integrations/discogs/auth-plan.md) — Discogs broker architecture decisions and phased implementation plan
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — Development workflow, testing expectations, and pull request checklist
- [`SECURITY.md`](SECURITY.md) — Supported versions and vulnerability reporting process
- [`CODEX.md`](CODEX.md) — Codex-specific operator/developer workflow notes
- [`CLAUDE.md`](CLAUDE.md) — Claude Code-specific operator/developer workflow notes
