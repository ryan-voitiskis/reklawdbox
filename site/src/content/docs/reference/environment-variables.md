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

The auto-detection path works for standard macOS Rekordbox installations. Only set this if your database is in a non-standard location.

## Discogs enrichment

| Variable                          | Description                               | Default                                                |
| --------------------------------- | ----------------------------------------- | ------------------------------------------------------ |
| `REKLAWDBOX_DISCOGS_BROKER_URL`   | URL of the Discogs broker service         | Built-in production broker URL                         |
| `REKLAWDBOX_DISCOGS_BROKER_TOKEN` | Client token for a custom broker endpoint | Built-in public client token for the maintained broker |

The broker is a Cloudflare Workers service that handles Discogs OAuth and rate limiting. The maintained broker URL and a public client token are compiled in, so no configuration is needed for standard use. Set `REKLAWDBOX_DISCOGS_BROKER_TOKEN` only when you point `REKLAWDBOX_DISCOGS_BROKER_URL` at a self-hosted or custom broker that requires its own client token. Run `reklawdbox setup --broker` for a guided setup that writes a config file (`~/Library/Application Support/reklawdbox/config.toml`).

## Audio analysis

| Variable                    | Description                                   | Default                                                     |
| --------------------------- | --------------------------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_ESSENTIA_PYTHON` | Path to Python binary with Essentia installed | Probes `~/.local/share/reklawdbox/essentia-venv/bin/python` |

Probe behavior:

- The server probes `CRATE_DIG_ESSENTIA_PYTHON` and the default venv path **lazily on first use** (not at startup). The first tool call that needs Essentia triggers the probe.
- If neither imports Essentia successfully, tools report Essentia as unavailable and continue with stratum-dsp only.
- The probe result is memoized for the process lifetime — restart the server after changing your Essentia config.
- The `setup_essentia` tool can install and activate Essentia without a restart.

stratum-dsp always runs regardless of Essentia availability. It provides BPM and key. Without Essentia, energy falls back to a BPM proxy. Essentia adds energy, danceability, brightness, and additional spectral features. Run `reklawdbox setup` to install Essentia to the default venv path — no env var needed.

## Backup

| Variable                   | Description                                           | Default         |
| -------------------------- | ----------------------------------------------------- | --------------- |
| `REKLAWDBOX_BACKUP_SCRIPT` | Path to a custom backup script run before `write_xml` | Built-in script |

A backup of your Rekordbox database files runs automatically before every `write_xml` export using a script embedded in the binary. Set this variable only if you want to replace the built-in backup with your own script. The script receives `--pre-op` as its first argument.

## Storage

| Variable               | Description                   | Default                                                     |
| ---------------------- | ----------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_STORE_PATH` | Path to internal cache SQLite | `~/Library/Application Support/reklawdbox/internal.sqlite3` |

The cache database stores Discogs/Beatport enrichment results, audio analysis output, and broker session tokens. It is safe to delete at any time — data will be re-fetched or re-analyzed on next use.

## Advanced

| Variable                                 | Description                                           | Default                        |
| ---------------------------------------- | ----------------------------------------------------- | ------------------------------ |
| `REKLAWDBOX_BEATPORT_MIN_INTERVAL_MS`    | Minimum interval between Beatport requests (ms)       | `1000`                         |
| `REKLAWDBOX_BANDCAMP_MIN_INTERVAL_MS`    | Minimum interval between Bandcamp requests (ms)       | `1500`                         |
| `REKLAWDBOX_MUSICBRAINZ_MIN_INTERVAL_MS` | Minimum interval between MusicBrainz requests (ms)    | `1100`                         |
| `REKLAWDBOX_DISCOGS_MIN_INTERVAL_MS`     | Minimum interval between Discogs broker requests (ms) | `1100`                         |
| `REKLAWDBOX_CORPUS_PATH`                 | Path to the Rekordbox knowledge corpus manifest       | `docs/rekordbox/manifest.yaml` |

These are internal tuning knobs. The rate-limit intervals control minimum time between requests to each service — lower values risk HTTP 429 errors. The corpus path points to the knowledge manifest used for contextual tool responses.

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
