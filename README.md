# reklawdbox

MCP server for Rekordbox 7.x library management. Reads directly from the
encrypted `master.db`, stages metadata changes in memory, and writes
Rekordbox-compatible XML for safe reimport -- never writes to the database.

Built as a single Rust binary with 48 MCP tools covering library search, audio
analysis, Discogs/Beatport enrichment, genre classification, transition scoring,
and set sequencing.

**For installation, usage guides, tool reference, and workflows, see
[reklawdbox.com](https://reklawdbox.com).**

## Install

```bash
brew tap ryan-voitiskis/reklawdbox
brew install reklawdbox
```

Then run interactive setup:

```bash
reklawdbox setup
```

This installs Essentia (audio analysis), configures Claude Code and Claude
Desktop, and verifies the Rekordbox database connection. See the
[install guide](https://reklawdbox.com/getting-started/) for details.

## Build from source

Requires the [Rust toolchain](https://rustup.rs/) and Xcode Command Line Tools.

```bash
git clone https://github.com/ryan-voitiskis/reklawdbox.git
cd reklawdbox
cargo build --release
```

The binary is at `./target/release/reklawdbox`.

## CLI subcommands

The binary runs as an MCP server by default. Subcommands are available for local
workflows outside your MCP host:

| Subcommand          | Description                                            |
| ------------------- | ------------------------------------------------------ |
| `setup`             | Install Essentia and configure MCP hosts               |
| `hydrate`           | Batch enrichment + analysis (Discogs, Beatport, audio) |
| `analyze`           | Batch audio analysis (stratum-dsp + Essentia)          |
| `backup`            | Manage Rekordbox library backups                       |
| `read-tags`         | Read metadata tags from audio files                    |
| `write-tags`        | Write metadata tags to audio files                     |
| `extract-art`       | Extract embedded cover art from an audio file          |
| `embed-art`         | Embed cover art into audio files                       |
| `disconnect-broker` | Clear stored Discogs broker session                    |

Run `reklawdbox <subcommand> --help` for usage details.

## Development

```bash
cargo build --release
cargo test
cargo test -- --ignored        # integration tests
dprint fmt && cargo fmt        # format
dprint check && cargo fmt --check  # verify formatting
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for expectations and workflow.

Agent-specific notes: [CLAUDE.md](CLAUDE.md), [AGENTS.md](AGENTS.md).

## Releasing

```bash
./scripts/release.sh 0.25.0
```

Tags and pushes. CI builds the binary, creates a GitHub Release, and updates the
[Homebrew tap](https://github.com/ryan-voitiskis/homebrew-reklawdbox).

## License

[MIT](LICENSE)
