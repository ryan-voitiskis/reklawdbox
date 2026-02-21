# CODEX.md

This file provides guidance to Codex when working with this repository.

## Build & Test

```bash
cargo build --release
cargo test
cargo test -- --ignored
bash scripts/run-rekordbox-evals.sh
```

Corpus validation commands:

```bash
bash docs/rekordbox/validate-corpus.sh
python3 docs/rekordbox/verify-phase-b.py
```

## Runtime Model

- Single-crate Rust MCP server targeting Rekordbox 7.x.
- Stdio transport only (`src/main.rs`); no CLI flags.
- MCP tool routing is defined in `src/tools.rs`.

## Codex Integration Notes

- Register or launch `./target/release/reklawdbox` as a stdio MCP server in your Codex host.
- Use `mcp-config.example.json` as the safe template for local host config.
- Required local data access:
  - Rekordbox DB path (`REKORDBOX_DB_PATH`, optional if default path exists)
  - Rekordbox backup script/runtime shell access for `write_xml` pre-op backup (`backup.sh`)
- Optional external integrations:
  - Discogs broker env vars (`REKLAWDBOX_DISCOGS_BROKER_URL`, `REKLAWDBOX_DISCOGS_BROKER_TOKEN`)
  - Deprecated Discogs legacy env vars (`REKLAWDBOX_DISCOGS_*`)
  - Essentia Python override (`CRATE_DIG_ESSENTIA_PYTHON`)
  - Recommended Essentia bootstrap script: `bash scripts/setup-essentia.sh`

## Working Rules

- Follow repo-level instructions in `AGENTS.md` (including commit format).
- Never commit secrets or local MCP credentials.
