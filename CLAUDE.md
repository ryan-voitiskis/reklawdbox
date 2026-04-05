# CLAUDE.md

Reklawdbox is an MCP server for Rekordbox 7.x that gives an AI agent read-only
SQLCipher DB access and stages metadata edits as Rekordbox XML for reimport
while never writing directly to the DB; human approval is always required.

It provides tools for library search, audio analysis via stratum-dsp +
Essentia, multi-provider enrichment (Discogs, Beatport, Bandcamp, MusicBrainz),
genre classification, transition scoring, and greedy set sequencing with
energy-curve shaping.

- Workspace: Rust 2024, two crates — `reklawdbox` (MCP server + CLI) and `stratum-dsp` (audio DSP).
- Key deps: `rmcp`, `tokio`, `rusqlite` + bundled SQLCipher/OpenSSL, `reqwest` + `rustls`, `symphonia`, `serde`/`schemars`.
- Rekordbox access: encrypted `master.db` opened read-only via SQLCipher. No write path exists.
- Write path: in-memory staged changes exported as Rekordbox-compatible XML for manual reimport.
- Local persistence: SQLite (WAL) for enrichment cache and audio-analysis cache. Broker session tokens are in macOS Keychain.
- Enrichment: Discogs via broker API (`broker/` — Cloudflare Workers + D1); Beatport, Bandcamp, MusicBrainz via direct HTTP.
- Audio analysis: `symphonia` decode → `stratum-dsp` (BPM, key); optional Essentia via Python subprocess (energy, timbre, rhythm).
- SOPs: `site/src/partials/sops/*.mdx` are `include_str!`'d into the binary via `help_handler.rs`. SOP changes require a release to take effect.
- Pre-commit hook: `cargo fmt --check`, `clippy -D warnings`, `dprint check`. Run `cargo fmt && dprint fmt` before committing.

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
