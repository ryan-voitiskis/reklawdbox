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

## MCP Development Loop

This project IS the MCP server. When modifying server code, `cargo build --release`
then ask the user to run `/mcp` to reconnect — the running server is the old binary
until restarted. Always build release (`.mcp.json` points to `target/release/`).
