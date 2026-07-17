---
title: Environment Variables
description: All environment variables for configuring reklawdbox.
sidebar:
  order: 2
---

In MCP server mode, reklawdbox is configured through environment variables set in your MCP host config. CLI subcommands also accept command-line flags — see [CLI Commands](/cli/) for details. CLI commands load environment variables from `.mcp.json` automatically.

## Required

| Variable            | Description                   | Default                                                |
| ------------------- | ----------------------------- | ------------------------------------------------------ |
| `REKORDBOX_DB_PATH` | Path to Rekordbox `master.db` | Auto-detected: `~/Library/Pioneer/rekordbox/master.db` |

The auto-detection path works for standard macOS Rekordbox installations. Only set this if your database is in a non-standard location. For server connections and backup-producing commands, a configured path must be an existing, direct regular file named `master.db`; symlinks are rejected before the server opens the database or starts a backup. The restore command is the recovery exception: it can target a missing `master.db` when that file's configured parent directory already exists.

The server resolves this effective path once and uses the same file for its read-only Rekordbox connection and automatic pre-export backups. Manual backup commands also follow `REKORDBOX_DB_PATH` when it is configured instead of falling back to the standard directory.

## Discogs enrichment

| Variable                          | Description                               | Default                                                |
| --------------------------------- | ----------------------------------------- | ------------------------------------------------------ |
| `REKLAWDBOX_DISCOGS_BROKER_URL`   | URL of the Discogs broker service         | Built-in production broker URL                         |
| `REKLAWDBOX_DISCOGS_BROKER_TOKEN` | Client token for a custom broker endpoint | Built-in public client token for the maintained broker |

The broker is a Cloudflare Workers service that handles Discogs OAuth and rate limiting. The maintained broker URL and a public client token are compiled in, so no configuration is needed for standard use. Set `REKLAWDBOX_DISCOGS_BROKER_TOKEN` only when you point `REKLAWDBOX_DISCOGS_BROKER_URL` at a self-hosted or custom broker that requires its own client token. Run `reklawdbox setup --broker` for a guided setup that writes a config file (`~/Library/Application Support/reklawdbox/config.toml`).

## Audio analysis

| Variable                    | Description                                                        | Default                                                     |
| --------------------------- | ------------------------------------------------------------------ | ----------------------------------------------------------- |
| `CRATE_DIG_ESSENTIA_PYTHON` | Expert override: path to a matching managed-manifest Python binary | Probes `~/.local/share/reklawdbox/essentia-venv/bin/python` |

Probe behavior:

- The server probes `CRATE_DIG_ESSENTIA_PYTHON` and the default venv path **lazily on first use** (not at startup). The first tool call that needs Essentia triggers the probe.
- A candidate must be CPython 3.14 with the exact Essentia 2.1b6.dev1438, NumPy 2.5.1, PyYAML 6.0.3, and six 1.17.0 manifest. If neither candidate matches, tools report Essentia as unavailable and continue with stratum-dsp only.
- The probe result is memoized for the process lifetime — restart the server after changing your Essentia config.
- The `setup_essentia` tool can install and activate Essentia without a restart.

stratum-dsp always runs regardless of Essentia availability. It returns BPM/key
with confidence, grid provenance and stability, decay, dub-stab and kick
evidence, and structural sections when available. Essentia adds optional
loudness, danceability, brightness, onset/rhythm, and other timbral features.
It does not return an `energy` field: scoring derives energy only when
danceability, integrated loudness, and onset rate are all present, otherwise it
uses a BPM proxy. See the [audio evidence and consumers](/mcp-tools/enrichment-analysis/#returned-and-cached-evidence)
reference for the complete grouped surface. Run `reklawdbox setup` to install
Essentia to the default venv path — no env var needed.

## Backup

| Variable                   | Description                                           | Default         |
| -------------------------- | ----------------------------------------------------- | --------------- |
| `REKLAWDBOX_BACKUP_SCRIPT` | Path to a custom backup script run before `write_xml` | Built-in script |

A backup of your Rekordbox database files runs automatically before every `write_xml` export using a script embedded in the binary. Set this variable only if you want to replace the built-in backup with your own script. The custom script receives `--pre-op` as its first argument and the exact effective database path in a child-only `REKORDBOX_DB_PATH` variable.

The built-in script reports success only after it creates the archive from that database directory and moves the archive into place. A custom script's zero exit is treated as the operator-supplied script's success attestation; reklawdbox cannot inspect what an arbitrary script archived. A missing custom script, launch error, or nonzero exit blocks the export and leaves staged changes available for retry. Close Rekordbox before exporting for the strongest backup consistency.

## Storage

| Variable               | Description                   | Default                                                     |
| ---------------------- | ----------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_STORE_PATH` | Path to internal state SQLite | `~/Library/Application Support/reklawdbox/internal.sqlite3` |

The internal state database contains both reconstructible and durable data:

- **Reconstructible caches:** enrichment results and audio-analysis output can
  be fetched or analyzed again.
- **Durable workflow state:** audit history, issue resolutions and notes,
  saved scoring presets, and calibration/profile statistics capture decisions
  that are not recreated by another enrichment run.
- **Broker session metadata:** session status and expiry metadata are stored
  here. The active broker credential is stored separately in macOS Keychain.

`CRATE_DIG_STORE_PATH` selects the database for all of these categories.
Moving or deleting it loses the durable decisions and settings even though
cache rows can be rebuilt. For authentication recovery, use the targeted
[`reklawdbox disconnect-broker` procedure](/troubleshooting/#discogs-broker-session-is-missing-or-expired)
instead of resetting the whole store.

## Advanced

| Variable                                 | Description                                           | Default |
| ---------------------------------------- | ----------------------------------------------------- | ------- |
| `REKLAWDBOX_BANDCAMP_MIN_INTERVAL_MS`    | Minimum interval between Bandcamp requests (ms)       | `1500`  |
| `REKLAWDBOX_MUSICBRAINZ_MIN_INTERVAL_MS` | Minimum interval between MusicBrainz requests (ms)    | `1100`  |
| `REKLAWDBOX_DISCOGS_MIN_INTERVAL_MS`     | Minimum interval between Discogs broker requests (ms) | `1100`  |

These are internal tuning knobs. The rate-limit intervals control minimum time
between requests to each service — lower values risk HTTP 429 errors.

## Example MCP config

A complete `mcp_servers` config block with all commonly used variables:

```json
{
  "mcpServers": {
    "reklawdbox": {
      "command": "/opt/homebrew/bin/reklawdbox"
    }
  }
}
```

If you built from source, use `./target/release/reklawdbox` as the command. After running `reklawdbox setup`, Essentia is auto-detected at the default venv path — no env vars needed for standard use.
