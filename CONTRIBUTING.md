# Contributing

Reklawdbox is a Rust MCP server and CLI for working with Rekordbox 7.x
libraries. The workspace also contains the `stratum-dsp` audio-analysis crate,
an Astro documentation site, and a Cloudflare Discogs broker.

Read [README.md](README.md) for the product workflow, [src/README.md](src/README.md)
for the architecture, and [AGENTS.md](AGENTS.md) for repository-specific safety
boundaries.

## Set up and verify

Install the Rust toolchain, Node.js 22, `dprint`, and the project dependencies,
then run the standard gate:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
./scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db
```

Ignored Rust tests are opt-in checks that may require private local audio,
Rekordbox data, or benchmark fixtures. They are not part of the normal test
suite. New required tests must use synthetic or checked-in fixtures.

Run the private Rekordbox fixture gate only with an explicit backup archive:

```bash
REKORDBOX_TEST_BACKUP=/absolute/path/to/backup.tar.gz \
  cargo test -p reklawdbox private_rekordbox_ -- --ignored --nocapture
```

The gate extracts only `master.db` and optional WAL/SHM sidecars into a unique
temporary directory, opens the extracted database read-only, and copies any
selected audio into a separate temporary directory before a test may mutate
it. Never point a private test at the live database or mutate source audio.

## Area-specific checks

For documentation-site or public tool/CLI contract changes:

```bash
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs \
  --bin ./target/release/reklawdbox \
  --dist ./site/dist
```

Then complete the semantic review in
[`docs/workflows/doc-drift/README.md`](docs/workflows/doc-drift/README.md).

For broker changes:

```bash
(cd broker && SHARP_IGNORE_GLOBAL_LIBVIPS=1 npm ci && npm run typecheck && npm run build && npm test)
```

After changing the Rekordbox research corpus, also run:

```bash
bash docs/rekordbox/validate-corpus.sh
python3 docs/rekordbox/verify-phase-b.py
```

After changing the genre reference candidate corpus, also run:

```bash
python3 -m unittest scripts/test_validate_genre_reference_corpus.py
python3 -m json.tool docs/genre-classification/genre-reference-candidates.json >/dev/null
python3 scripts/validate_genre_reference_corpus.py docs/genre-classification/genre-reference-candidates.json
```

During an explicitly incomplete family-wave checkpoint, the final command may
temporarily use `--allow-incomplete`. That mode still fully validates every
populated genre and never permits taxonomy drift or `Experimental` candidates.

## Safety boundaries

- Normal Rekordbox database access is SQLCipher read-only. Never add SQL that
  mutates `master.db`. The confirmed `backup --restore` CLI workflow is a
  separate recovery boundary.
- Stage user-visible Rekordbox metadata through `ChangeManager` and export it
  with `write_xml` for manual import.
- Local Reklawdbox cache, audit, calibration, and broker-session writes are
  allowed. Keep them separate from the Rekordbox library.
- Audio tag and cover-art writes modify files directly. Preserve dry-run,
  preview, backup, and confirmation safeguards.
- Define MCP surfaces in `src/mcp/server.rs` and the matching
  `src/mcp/*/transport.rs` types. Reusable behavior belongs in `application/`,
  not in a transport handler.
- SOPs under `site/src/partials/sops/` are compiled into the binary through
  `src/mcp/help.rs`; changing one requires a rebuild and release/deploy.

## Change expectations

- Keep changes scoped and preserve unrelated worktree edits.
- Add or update tests for behavior changes, or explain why a test is not
  practical.
- Update user-facing documentation when behavior, tools, parameters, or CLI
  flags change.
- Run `cargo fmt` and `dprint fmt` before the verification commands. Note that
  `dprint` intentionally excludes `docs/**`.
- Use Conventional Commits, such as `fix(mcp): handle missing playlist`.
- Never commit credentials, private library/audio data, caches, or local-only
  configuration such as `.mcp.json`.

## Security issues

Do not open a public issue for a vulnerability. Follow [SECURITY.md](SECURITY.md)
to report it privately.
