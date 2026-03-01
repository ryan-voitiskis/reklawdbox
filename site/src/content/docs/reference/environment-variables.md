---
title: Environment Variables
description: All environment variables for configuring reklawdbox.
sidebar:
  order: 2
---

reklawdbox is configured entirely through environment variables set in your MCP host config. No config files, no CLI flags.

## Required

| Variable            | Description                   | Default                                                |
| ------------------- | ----------------------------- | ------------------------------------------------------ |
| `REKORDBOX_DB_PATH` | Path to Rekordbox `master.db` | Auto-detected: `~/Library/Pioneer/rekordbox/master.db` |

The auto-detection path works for standard macOS Rekordbox installations. Only set this if your database is in a non-standard location.

## Discogs enrichment

| Variable                          | Description                       | Required                                   |
| --------------------------------- | --------------------------------- | ------------------------------------------ |
| `REKLAWDBOX_DISCOGS_BROKER_URL`   | URL of the Discogs broker service | Yes, for Discogs lookups                   |
| `REKLAWDBOX_DISCOGS_BROKER_TOKEN` | Auth token for the broker         | Yes (unless broker allows unauthenticated) |

The broker is a separate Cloudflare Workers service that handles Discogs OAuth and rate limiting on your behalf. Without these variables, `lookup_discogs` and `enrich_tracks` skip Discogs entirely.

## Audio analysis

| Variable                    | Description                                   | Default                                                     |
| --------------------------- | --------------------------------------------- | ----------------------------------------------------------- |
| `CRATE_DIG_ESSENTIA_PYTHON` | Path to Python binary with Essentia installed | Probes `~/.local/share/reklawdbox/essentia-venv/bin/python` |

Probe behavior:

- The server probes `CRATE_DIG_ESSENTIA_PYTHON` and the default venv path at startup.
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

## Deprecated (legacy Discogs direct auth)

These variables still work as a fallback but are not the recommended path. Use the broker variables above instead.

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
      "command": "./target/release/reklawdbox",
      "env": {
        "REKORDBOX_DB_PATH": "/Users/<you>/Library/Pioneer/rekordbox/master.db",
        "REKLAWDBOX_DISCOGS_BROKER_URL": "<broker-url>",
        "REKLAWDBOX_DISCOGS_BROKER_TOKEN": "<broker-token>",
        "CRATE_DIG_ESSENTIA_PYTHON": "/Users/<you>/.local/share/reklawdbox/essentia-venv/bin/python"
      }
    }
  }
}
```

Replace `<you>` with your macOS username. The `command` path assumes you built from source — adjust to wherever your reklawdbox binary lives.
