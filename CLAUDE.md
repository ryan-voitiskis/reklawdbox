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

This project IS the MCP server. When modifying server code:

1. `cargo build --release` (`.mcp.json` points to `target/release/`)
2. Ask the user to run `/mcp` to reconnect — the running server is the old binary until restarted.
3. After reconnection, smoke-test the changed functionality by calling the affected MCP tools with representative inputs. Include at least one happy-path call and one edge-case or error-path call per changed tool.

## Releasing

Tag-triggered. Pushing a `v*.*.*` tag runs `.github/workflows/release.yml` which:
builds on ARM64 macOS, runs tests, creates a GitHub Release with the binary tarball,
and auto-updates the Homebrew formula in `ryan-voitiskis/homebrew-reklawdbox`.

```
# 1. Bump version in Cargo.toml
# 2. Commit, tag, push
git tag v0.x.y && git push origin main v0.x.y
```

Requires `HOMEBREW_TAP_TOKEN` repo secret (PAT with `repo` scope for the tap repo).
