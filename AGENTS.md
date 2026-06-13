# AGENTS.md

Reklawdbox is an MCP server and CLI for Rekordbox 7.x. It reads the encrypted
Rekordbox library, stages metadata edits in memory, and exports XML for manual
reimport.

## Non-Negotiables

- Never add a direct write path to Rekordbox `master.db`. It is opened via
  SQLCipher with `SQLITE_OPEN_READ_ONLY`; preserve that boundary.
- User-visible metadata changes go through `ChangeManager` and `write_xml`.
  Local SQLite cache/store writes are fine for enrichment, analysis, audit,
  calibration, and broker-session state.
- SOPs in `site/src/partials/sops/*.mdx` are embedded by
  `src/tools/help_handler.rs` using `include_str!`; SOP changes need a rebuild
  and deploy/release before an MCP host sees them.
- Tool surfaces are defined by `#[tool(...)]` annotations and `schemars`
  descriptions in `src/tools/params.rs`.
- Use Conventional Commits.

## Repo Shape

- Root crate `reklawdbox` (Rust 2024): MCP server, CLI, DB access, enrichment,
  classification, staging, XML export.
- `stratum-dsp` (Rust 2021): DSP and audio analysis.
- `site/`: Astro Starlight docs.
- `broker/`: Cloudflare Workers + D1 Discogs broker.
- `docs/tmp/`: research notes. `dprint` excludes `docs/**`.

## Build And Test

Normal gate for server/CLI work:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
```

Docs-site changes need:

```bash
cd site
npm install
npm run build
```

Full workspace checks are currently stricter than CI:

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p stratum-dsp --no-fail-fast
```

Known state on 2026-06-13: `reklawdbox` builds/tests cleanly, but full
workspace clippy is red in `stratum-dsp` (`dub_stab_real_audio.rs`,
`dub_stab.rs`, `sections.rs`), and `stratum-dsp` integration tests fail because
`120bpm_4bar.wav`, `128bpm_4bar.wav`, `cmajor_scale.wav`, and
`mixed_silence.wav` are missing. Do not report a full workspace pass until
those are fixed.

The pre-commit hook runs full workspace clippy when staged Rust files exist. If
that known-red state still exists, either fix it first or avoid staging Rust in
unrelated docs commits.

## MCP Local Testing

The repo `.mcp.json` runs `reklawdbox mcp` from `PATH`, which usually resolves
to the Homebrew binary, not `target/release/reklawdbox`. Claude Code also
re-resolves from `PATH` on `/mcp`, so editing `.mcp.json` is not enough.

Local MCP test loop:

1. `./scripts/deploy-local.sh` builds release, overwrites the Homebrew Cellar
   binary, and re-signs it. Requires `sudo`.
2. Ask the user to run `/mcp`.
3. Smoke-test changed MCP tools with a happy path and an error/edge path.

If `sudo` is unavailable, run the release build and CLI smoke tests, then say
that MCP host testing was not performed.

Essentia uses `.venvs/essentia/bin/python` via `CRATE_DIG_ESSENTIA_PYTHON` in
`.mcp.json`. For analysis tests that must bypass cache, pass
`skip_cached: false`.

## Change-Specific Notes

- Audio output/schema changes usually need a cache schema bump in
  `src/audio.rs` (`STRATUM_SCHEMA_VERSION` or `ESSENTIA_SCHEMA_VERSION`).
- DSP tests should use synthetic fixtures where possible; do not make normal
  tests depend on private local audio files.
- Before release, run the doc-drift workflow in
  `docs/workflows/doc-drift/README.md` if tools, params, SOPs, CLI flags, or
  README claims changed.
- `./scripts/release.sh <version>` requires clean `main`, bumps version files,
  commits, tags, pushes, and lets GitHub Actions publish the release plus
  Homebrew formula update.
