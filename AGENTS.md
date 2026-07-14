# Repository guidance

Reklawdbox is a Rust MCP server and CLI for Rekordbox 7.x. It reads the
encrypted library, stages metadata in memory, and exports XML for manual
Rekordbox import.

## Boundaries

- Normal `master.db` access is SQLCipher read-only. Never add a SQL mutation
  path. The confirmed `backup --restore` CLI flow may replace files for
  disaster recovery; keep it isolated from MCP and normal library access.
- Route user-visible Rekordbox metadata through `ChangeManager` and
  `write_xml`. Reklawdbox-owned cache, analysis, audit, calibration, and broker
  SQLite writes are allowed.
- Audio tag and cover-art tools write directly to selected files. Preserve
  preview, dry-run, backup, and confirmation behavior.
- Preserve unrelated worktree changes and stage only files in scope. Use
  Conventional Commits.

## Ownership

- `domain/` owns pure rules; `application/` owns reusable workflows;
  `adapters/` owns I/O; `mcp/` and `cli/` own transport concerns. See
  `src/README.md`.
- MCP tool surfaces come from `#[tool(...)]` in `src/mcp/server.rs` and
  `schemars` types/descriptions in `src/mcp/*/transport.rs`.
- SOPs in `site/src/partials/sops/*.mdx` are embedded by `src/mcp/help.rs` with
  `include_str!`; rebuild and deploy/release before a host can see edits.
- `stratum-dsp/` owns DSP, `site/` owns Astro docs, and `broker/` owns the
  Cloudflare/D1 Discogs broker. `dprint` intentionally excludes `docs/**`.

## Verify

Run the standard workspace gate:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
```

- Do not add mandatory tests that require private audio or Rekordbox data;
  ignored tests are opt-in local or benchmark checks.
- For docs/public-contract, broker, or research-corpus changes, run the exact
  area gate in `CONTRIBUTING.md`. Public-surface changes also need the semantic
  review in `docs/workflows/doc-drift/README.md`.
- Audio output/schema changes usually require a cache-version bump in
  `src/adapters/audio/mod.rs`.

## MCP and releases

- Claude Code `.mcp.json` runs `reklawdbox mcp` from `PATH`, usually the
  Homebrew binary, and points Essentia at `.venvs/essentia/bin/python`. A repo
  build does not refresh that host binary.
- The ChatGPT desktop app, Codex CLI, and Codex IDE extension share Codex MCP
  configuration rather than `.mcp.json`. Restart the app/extension or start a
  new CLI task after changing the server or its tool schemas.
- To test the current checkout directly, build release and run
  `./scripts/mcp-smoke.mjs`; add `--skip-db` when the library is unavailable.
  Pass `skip_cached: false` when testing fresh audio analysis.
- For Claude Code host testing, run `./scripts/deploy-local.sh` (requires
  `sudo`), reconnect with `/mcp`, then test a happy and an edge/error path. If
  host deployment is unavailable, say so.
- `./scripts/release.sh <version>` requires clean `main`, runs preflight,
  commits/tags/pushes, and triggers release plus Homebrew formula publishing.
