# Spec: Rust-Native Tag Reading/Writing

## Problem

kid3-cli and exiftool are external dependencies with real operational issues:

- **Quoting bugs:** kid3-cli's `select 'filename'` breaks on apostrophes. Workarounds exist but are fragile.
- **Alias conflicts:** `find`/`grep` aliased to `fd`/`rg` silently break shell snippets. Full paths (`/usr/bin/find`) required as mitigation.
- **Slow per-file invocation:** kid3-cli spawns a Qt app per call. exiftool spawns Perl. ~5 min for 1,300 files.
- **No tag 2/3 distinction API:** exiftool can read RIFF INFO separately, but kid3-cli's tag switching (`-c "tag 3"`) is awkward and error-prone for batch operations.
- **Two tools for one job:** exiftool for reading, kid3-cli for writing. Different quoting rules, different field names, different output formats.

## Requirements

1. Read tags from FLAC, WAV, MP3, M4A files
2. Write tags to the same formats
3. Distinguish WAV ID3v2 (tag 2) from RIFF INFO (tag 3) for both read and write
4. Single binary — no Python, Perl, Qt, or other runtime dependencies
5. Expose as MCP tools (`read_file_tags`, `write_file_tags`) callable by the agent
6. Handle the full character set (apostrophes, unicode, CJK) without quoting issues
7. Batch operation support (multiple files per call)

## Proposed Approach

Add two MCP tools to reklawdbox:

### `read_file_tags`

```
read_file_tags(path="/path/to/file.wav")
read_file_tags(paths=["/path/to/dir/*.flac"], recursive=false)
```

Returns structured JSON with all tags, clearly separated by tag type for WAV (ID3v2 vs RIFF INFO).

### `write_file_tags`

```
write_file_tags(path="/path/to/file.wav", tags={artist: "Name", title: "Title"}, tag_types=["id3v2", "riff_info"])
```

For WAV files, `tag_types` controls which tag layers to write. Default: both (matching the dual-tag convention).

### Crate candidates

| Crate       | Read | Write | WAV tag 2/3 | Formats           | Notes                              |
| ----------- | ---- | ----- | ----------- | ----------------- | ---------------------------------- |
| `lofty`     | Yes  | Yes   | Yes         | FLAC/WAV/MP3/M4A  | Distinguishes ID3v2 vs RIFF INFO   |
| `symphonia` | Yes  | No    | Partial     | FLAC/WAV/MP3/M4A  | Audio-focused, metadata is secondary |

`lofty` is the clear choice — it handles both read and write, supports all target formats, and distinguishes WAV tag types natively.

## Open Questions

- Should cover art read/write be included in v1, or deferred?
- Should the tool validate tag values against conventions (e.g., reject zero-padded track numbers), or just read/write raw values?
- Performance target: what's acceptable latency for batch reads of 1,000+ files?
