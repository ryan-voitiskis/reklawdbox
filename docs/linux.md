# Running reklawdbox on Linux and WSL

This fork (`bender-projects/reklawdbox`, branch `linux-support`) runs the
MCP server on Linux against a library that belongs to a Rekordbox install on
another machine — typically Windows, reached from WSL today and from a
cluster later. Rekordbox itself does not run on Linux, so reklawdbox always
works on a **snapshot** of `master.db`, never on the live file.

Validated on Ubuntu 24.04 under WSL2 (x86_64), Rust 1.99, against a real
Rekordbox 7 library: release build, full test suite, `cargo clippy -D
warnings`, and MCP calls over stdio (`read_library`, `search_tracks`,
`get_track`, `scan_broken_links`, `read_file_tags`, `analyze_track_audio`,
`write_xml`).

## Build

```bash
sudo apt install build-essential perl pkg-config   # SQLCipher builds vendored OpenSSL
cargo build --release --locked                      # ~2 min on 32 cores
cargo install --path . --locked                     # optional: ~/.cargo/bin/reklawdbox
```

## Take a snapshot of the Windows library (WSL)

SQLite's WAL locking does not work across the Windows/WSL boundary, so
reading the live `master.db` from `/mnt/c` while Rekordbox writes can return a
torn database. Close Rekordbox and copy it:

```bash
scripts/wsl-snapshot.sh               # → ~/.local/share/reklawdbox/rekordbox/master.db
scripts/wsl-snapshot.sh --with-anlz   # also copy share/ so Rekordbox beat grids are used
```

The script refuses to run while `rekordbox.exe` is up (`--force` overrides).
Re-run it whenever you want reklawdbox to see library changes.

## Configuration

| Variable | Purpose |
| --- | --- |
| `REKORDBOX_DB_PATH` | **Required on Linux.** The snapshot's `master.db`. |
| `REKLAWDBOX_PATH_MAP` | Translate Rekordbox paths to local ones: `DB_PREFIX=LOCAL_PREFIX`, several rules separated by `;`. Example: `N:/media/music=/mnt/data/media/music`. |
| `REKORDBOX_ANLZ_ROOT` | Override where ANLZ beat grids live. Defaults to `share/` next to `REKORDBOX_DB_PATH` when it exists. |
| `REKLAWDBOX_SECRETS_DIR` | Where the Discogs broker session token is stored (default `~/.config/reklawdbox`). Use a persistent volume in a container. |

### Path translation

Rekordbox stores absolute paths from its own host (`N:/media/music/...`).
`REKLAWDBOX_PATH_MAP` is applied where reklawdbox reads `FolderPath`, so every
tool sees and returns local paths; it is reversed wherever a path goes back to
Rekordbox or is compared with `FolderPath`:

- XML export `Location` keeps the Rekordbox form, so importing the XML in
  Rekordbox on Windows still matches the existing tracks;
- `path` / `path_prefix` filters and orphan scans accept local paths;
- ANLZ beat-grid lookups query by the Rekordbox form.

DB prefixes match ASCII case-insensitively and accept `\`; rules match whole
path segments only, and the longest prefix wins. Paths with no matching rule
(streaming entries such as `/v4/catalog/...`) pass through unchanged.

## Use from Claude Code on Windows

The Windows client can launch the Linux binary through `wsl.exe`:

```json
{
  "mcpServers": {
    "reklawdbox": {
      "command": "wsl.exe",
      "args": [
        "-d", "Ubuntu-24.04", "--",
        "env",
        "REKORDBOX_DB_PATH=/home/<user>/.local/share/reklawdbox/rekordbox/master.db",
        "REKLAWDBOX_PATH_MAP=N:/media/music=/mnt/data/media/music",
        "/home/<user>/.cargo/bin/reklawdbox"
      ]
    }
  }
}
```

The audio files must be reachable inside WSL for analysis and tag tools, e.g.
mount the NAS share: `sudo mount -t drvfs '\\nas\data' /mnt/data`.

## Platform changes in this fork

- `adapters/rekordbox/path_map.rs`: `REKLAWDBOX_PATH_MAP` translation.
- `adapters/rekordbox/anlz.rs`: ANLZ root defaults to `share/` beside the DB.
- `adapters/platform/keychain.rs`: without the macOS Keychain the Discogs
  broker token is kept in an owner-only (0600) JSON file instead of failing.
- `adapters/platform/browser.rs`: opens the auth URL with `wslview`,
  `explorer.exe` or `xdg-open` instead of macOS `open`.
- `scripts/backup.sh`: detects Rekordbox running on the Windows host from WSL.
- `scripts/wsl-snapshot.sh`: consistent snapshot of the Windows library.

## Known gaps

- **Essentia** (`setup` / `setup_essentia`) has Linux code paths but is not
  validated here yet; without it classification stays *Degraded*.
- `write_xml` backs up the database it reads — on Linux that is the snapshot,
  written to `~/Music/rekordbox-backups/`. The Windows library is never
  modified; changes reach it only by importing the exported XML in Rekordbox.
- Rekordbox's Windows sampler folder may not match `SAMPLER_PATH_FRAGMENT`
  (`/rekordbox/Sampler/`), so sampler files can show up in library scans.
- MCP tools carry no `readOnlyHint` annotations; rely on the client's
  per-tool permission prompts.
- Some upstream tests write fixture XML into the real `~/reklawdbox-exports`.
- The server speaks stdio only; a cluster deployment needs an HTTP transport
  with authentication in front of it.
