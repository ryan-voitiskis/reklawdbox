# AGENTS.md

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

Use Conventional Commits.

## MCP Development Loop

This project IS the MCP server. `.mcp.json` resolves `reklawdbox` from PATH,
which points to the Homebrew-installed binary. To test local changes,
`cargo build --release` then overwrite the Cellar binary and ask the user to
run `/mcp` to reconnect. See CLAUDE.md for the full procedure.
