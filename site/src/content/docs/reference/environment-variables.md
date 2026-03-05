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

| Variable                          | Description                       | Default                                                         |
| --------------------------------- | --------------------------------- | --------------------------------------------------------------- |
| `REKLAWDBOX_DISCOGS_BROKER_URL`   | URL of the Discogs broker service | Built-in production broker URL                                  |
| `REKLAWDBOX_DISCOGS_BROKER_TOKEN` | Auth token for the broker         | Built-in token                                                  |

The broker is a separate Cloudflare Workers service that handles Discogs OAuth and rate limiting on your behalf. Both variables have compiled-in defaults pointing to the production broker — you only need to set them to override for local development.

## Audio analysis

| Variable                    | Description                                   | Default                                                     |
| --------------------------- | --------------------------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_ESSENTIA_PYTHON` | Path to Python binary with Essentia installed | Probes `~/.local/share/reklawdbox/essentia-venv/bin/python` |

Probe behavior:

- The server probes `CRATE_DIG_ESSENTIA_PYTHON` and the default venv path **lazily on first use** (not at startup). The first tool call that needs Essentia triggers the probe.
- If neither imports Essentia successfully, tools report Essentia as unavailable and continue with stratum-dsp only.
- The probe result is memoized for the process lifetime — restart the server after changing your Essentia config.
- The `setup_essentia` tool can install and activate Essentia without a restart.

stratum-dsp always runs regardless of Essentia availability. It provides BPM, key, and energy. Essentia adds danceability, brightness, and additional spectral features.

## Backup

| Variable                   | Description                                            | Default                                             |
| -------------------------- | ------------------------------------------------------ | --------------------------------------------------- |
| `REKLAWDBOX_BACKUP_SCRIPT` | Path to a backup script run before `write_xml` exports | Probes `scripts/backup.sh`, then `backup.sh` in cwd |

The backup script runs synchronously before any XML is written. If neither the env var nor the default probe paths point to an existing script, no backup runs and `write_xml` proceeds normally.

## Storage

| Variable               | Description                   | Default                                                     |
| ---------------------- | ----------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_STORE_PATH` | Path to internal cache SQLite | `~/Library/Application Support/reklawdbox/internal.sqlite3` |

The cache database stores Discogs/Beatport enrichment results, audio analysis output, and broker session tokens. It is safe to delete at any time — data will be re-fetched or re-analyzed on next use.

## Advanced

| Variable                              | Description                                        | Default    |
| ------------------------------------- | -------------------------------------------------- | ---------- |
| `REKLAWDBOX_BEATPORT_MIN_INTERVAL_MS` | Minimum interval between Beatport requests (ms)    | `1000`     |
| `REKLAWDBOX_CORPUS_PATH`              | Path to the Rekordbox knowledge corpus manifest    | `docs/rekordbox/manifest.yaml` |

These are internal tuning knobs. The Beatport interval controls rate limiting — lower values risk HTTP 429 errors. The corpus path points to the knowledge manifest used for contextual tool responses.

## Deprecated (legacy Discogs direct auth)

These variables still work as a fallback but are not the recommended path. The built-in broker handles auth and rate limiting automatically.

| Variable                          | Description                 |
| --------------------------------- | --------------------------- |
| `REKLAWDBOX_DISCOGS_KEY`          | Discogs consumer key        |
| `REKLAWDBOX_DISCOGS_SECRET`       | Discogs consumer secret     |
| `REKLAWDBOX_DISCOGS_TOKEN`        | Discogs OAuth token         |
| `REKLAWDBOX_DISCOGS_TOKEN_SECRET` | Discogs OAuth token secret  |
| `REKLAWDBOX_DISCOGS_API_BASE_URL` | Custom Discogs API base URL |

Direct auth requires you to manage your own Discogs API credentials and rate limiting. The broker handles both automatically.

## Example MCP config

A complete `mcp_servers` config block with all commonly used variables:

```json
{
  "mcpServers": {
    "reklawdbox": {
      "type": "stdio",
      "command": "/opt/homebrew/bin/reklawdbox",
      "env": {
        "CRATE_DIG_ESSENTIA_PYTHON": "/Users/<you>/.local/share/reklawdbox/essentia-venv/bin/python"
      }
    }
  }
}
```

Replace `<you>` with your macOS username. If you built from source, use `./target/release/reklawdbox` as the command. The Rekordbox database path and Discogs broker are auto-configured with sensible defaults.
