# CLAUDE.md

Reklawdbox is an MCP server for Rekordbox 7.x that gives an AI agent read-only
SQLCipher DB access and stages metadata edits as Rekordbox XML for reimport
while never writing directly to the DB; human approval is always required.

It provides tools for library search, audio analysis via stratum-dsp +
Essentia, Discogs/Beatport enrichment, genre classification, transition
scoring, and greedy set sequencing with energy-curve shaping.

- Runtime: Rust 2024 single binary (`cargo`), `rmcp`, `tokio`, `serde`/`serde_json`/`schemars`.
- Rekordbox access: `rusqlite` + bundled SQLCipher/OpenSSL; encrypted `master.db` is read-only.
- Write path: DB is never written; exports Rekordbox-compatible XML.
- Local persistence: separate SQLite (WAL) for enrichment cache, audio-analysis cache, broker session tokens.
- Enrichment I/O: `reqwest` + `rustls`; Discogs via broker API; Beatport via HTML/JSON extraction.
- Audio analysis: `symphonia` decode + `stratum-dsp`; optional Essentia via Python subprocess.
- Companion service: Discogs broker in TypeScript on Cloudflare Workers + D1.
- SOPs: `site/src/partials/sops/*.mdx` are `include_str!`'d into the binary via `help_handler.rs`. SOP changes require a release to take effect.

## MCP Development Loop

This project IS the MCP server. The user's MCP config (`.mcp.json` in the repo root) normally resolves `reklawdbox` from PATH, which points to the Homebrew-installed binary in the Cellar. Claude Code ignores config file edits for running servers — `/mcp` always re-resolves the command from PATH. To test local changes:

1. `cargo build --release`
2. Overwrite the Homebrew binary: `sudo cp target/release/reklawdbox /opt/homebrew/Cellar/reklawdbox/$(brew list --versions reklawdbox | awk '{print $2}')/bin/reklawdbox`
3. Ask the user to run `/mcp` to reconnect with the new binary.
4. Smoke-test the changed functionality by calling the affected MCP tools. Include at least one happy-path call and one edge-case or error-path call per changed tool.
5. The Cellar binary stays overwritten until the next `brew upgrade` or release, so no manual revert is needed.

## Releasing

`./scripts/release.sh <version>` bumps `Cargo.toml`, updates the site homepage version,
commits, tags, and pushes. The tag push triggers `.github/workflows/release.yml` which
builds on ARM64 macOS, runs tests, creates a GitHub Release with the binary tarball,
and auto-updates the Homebrew formula in `ryan-voitiskis/homebrew-reklawdbox`.

```
./scripts/release.sh 0.25.0
```

Requires `HOMEBREW_TAP_TOKEN` repo secret (PAT with `repo` scope for the tap repo).
