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
2. Write tags to the same formats (merge semantics — only specified fields touched)
3. Read, write, and embed cover art for all formats
4. Distinguish WAV ID3v2 (tag 2) from RIFF INFO (tag 3) for both read and write
5. Single binary — no Python, Perl, Qt, or other runtime dependencies
6. Expose as MCP tools and CLI subcommands from the same binary
7. Handle the full character set (apostrophes, unicode, CJK) without quoting issues
8. Batch operation support (multiple files per call)
9. Field filtering on reads to control response size
10. Dry-run mode for writes (show diff without touching files)

## Crate

<!-- dprint-ignore -->
| Crate       | Read | Write | WAV tag 2/3 | Formats           | Notes                              |
| ----------- | ---- | ----- | ----------- | ----------------- | ---------------------------------- |
| `lofty`     | Yes  | Yes   | Yes         | FLAC/WAV/MP3/M4A  | Distinguishes ID3v2 vs RIFF INFO   |
| `symphonia` | Yes  | No    | Partial     | FLAC/WAV/MP3/M4A  | Audio-focused, metadata is secondary |

`lofty` (0.23.x) is the clear choice. Key capabilities:

- Independent accessors per tag type: `WavFile::id3v2()` / `riff_info()`, `FlacFile::vorbis_comments()`, `MpegFile::id3v2()`, `Mp4File::ilst()`
- All standard fields mapped via `ItemKey` enum (108 variants including `TrackArtist`, `TrackTitle`, `AlbumTitle`, `Bpm`, `InitialKey`, `Label`, `Publisher`, etc.)
- Cover art via `Picture` type with format detection
- `WriteOptions::remove_others = false` (default) preserves coexisting tag types
- `ParseOptions::read_cover_art(false)` to skip art on metadata-only reads
- `ParsingMode::BestAttempt` recovers from corrupt tags gracefully
- Full UTF-8 throughout; non-UTF-8 RIFF INFO (Windows UTF-16 LE) silently discarded in default mode

**Gotcha:** `ItemKey::Unknown` was removed in 0.23.0. Custom/non-standard fields require format-specific types (`Id3v2Tag`, `VorbisComments`, `Ilst`). Not relevant for standard fields but matters if scope expands later.

## Architecture

```
src/tags.rs       Core read/write logic (lofty). Pure functions, no MCP dependency.
src/tools.rs      MCP tool wrappers call tags:: functions
src/main.rs       clap dispatch: no subcommand → MCP stdio, subcommand → CLI
```

Core logic is shared between MCP and CLI. The `tags` module owns all lofty interaction. MCP tools and CLI subcommands are thin wrappers with different I/O serialization.

## MCP Tools

### `read_file_tags`

Read metadata tags from audio files. Exactly one input selector is required — providing multiple selectors (e.g., both `paths` and `track_ids`) returns a validation error.

#### Parameters

```jsonc
{
  // --- Input selectors (exactly one required) ---
  "paths": ["/path/a.wav", "/path/b.flac"],  // explicit file paths
  "track_ids": ["123", "456"],                // resolve file_path from Rekordbox DB
  "directory": "/path/to/album/",             // scan for audio files
  "glob": "*.wav",                            // filter within directory (default: all audio)
  "recursive": false,                         // scan subdirs (default: false)

  // --- Options ---
  "fields": ["artist", "title", "year"],      // return only these fields (default: all)
  "include_cover_art": false,                 // include cover art metadata (default: false)
  "limit": 200                                // max files (default: 200, max: 2000)
}
```

Selector groups: `paths`, `track_ids`, or `directory` (optionally with `glob` and `recursive`). If zero or multiple top-level selectors are provided, the tool returns `invalid_params` with a message listing which selectors were found.

#### Response

```jsonc
{
  "summary": {
    "files_read": 12,
    "files_failed": 0,
    "formats": { "wav": 8, "flac": 3, "mp3": 1 }
  },
  "results": [
    {
      // FLAC — single tag layer
      "path": "/path/to/track.flac",
      "format": "flac",
      "tag_type": "vorbis_comment",
      "tags": {
        "artist": "Burial",
        "title": "Archangel",
        "album": "Untrue",
        "year": "2007",
        "track": "1",
        "genre": "",            // empty string = tag exists but blank
        "album_artist": "Burial",
        "publisher": "Hyperdub",
        "disc": null,           // null = tag absent
        "comment": ""
      },
      "cover_art": {            // only if include_cover_art: true
        "format": "jpeg",
        "size_bytes": 45230
      }
    },
    {
      // WAV — dual tag layers shown separately
      "path": "/path/to/track.wav",
      "format": "wav",
      "id3v2": {
        "artist": "Burial",
        "title": "Archangel",
        "album": "Untrue",
        "year": "2007",
        "track": "1"
      },
      "riff_info": {
        "artist": "Burial",
        "title": "Archangel",
        "album": null,
        "year": null,
        "track": null
      },
      "tag3_missing": ["album", "year", "track"],  // fields in id3v2 but absent in riff_info
      "cover_art": null
    },
    {
      // Error — inline, not in a separate array
      "path": "/path/to/corrupted.flac",
      "error": "Failed to parse FLAC metadata: invalid block header"
    }
  ]
}
```

Design notes:

- **Null vs empty:** `null` = tag absent (no frame/field). `""` = tag exists but empty. This distinction is preserved on reads for audit accuracy.
- **Field filter on WAV:** When `fields` is specified, both `id3v2` and `riff_info` objects are filtered to only the requested fields. `tag3_missing` is also scoped to the filtered fields only — it will not report unfiltered fields as missing.
- **`tag3_missing`:** Computed for WAV only — surfaces the most critical audit issue (Rekordbox reads only RIFF INFO) without agent post-processing. A field appears in `tag3_missing` if it has a non-null value in `id3v2` but is `null` in `riff_info`.
- **Errors inline:** Failed files appear inline with `error` rather than in a separate array. Keeps results ordered.
- **Cover art metadata only:** `cover_art` returns format and size — never binary data. Use `extract_cover_art` to save art to a file.

### `write_file_tags`

Write metadata tags to audio files. Merge semantics — only specified fields are touched, everything else preserved.

**Delete semantics:** Both `null` and `""` (empty string) delete the tag frame entirely. There is no way to set a field to a present-but-empty value — in practice this distinction is meaningless for all supported fields and Rekordbox import. On reads, the tool preserves the null/empty distinction for audit; on writes, they are equivalent.

#### Parameters

```jsonc
{
  "writes": [
    {
      "path": "/path/to/track.wav",
      "tags": {
        "artist": "Burial",       // set value
        "title": "Archangel",
        "track": "1",
        "genre": null              // delete this tag
      },
      "wav_targets": ["id3v2", "riff_info"]  // WAV only. Default: both.
    }
  ],
  "dry_run": false    // true = show old→new diff without writing (default: false)
}
```

#### Response (write mode)

```jsonc
{
  "summary": {
    "files_written": 3,
    "files_failed": 0,
    "fields_written": 12
  },
  "results": [
    {
      "path": "/path/to/track.wav",
      "status": "ok",
      "fields_written": ["artist", "title", "track"],
      "fields_deleted": ["genre"],
      "wav_targets": ["id3v2", "riff_info"]
    }
  ]
}
```

#### Response (dry-run mode)

```jsonc
{
  "summary": {
    "files_previewed": 3,
    "files_failed": 0
  },
  "results": [
    {
      "path": "/path/to/track.wav",
      "status": "preview",
      "changes": {
        "artist": { "old": "Bural", "new": "Burial" },
        "title": { "old": null, "new": "Archangel" },
        "genre": { "old": "Dubstep", "new": null }
      },
      "wav_targets": ["id3v2", "riff_info"]
    }
  ],
  "dry_run": true
}
```

#### Response (write error — inline, same as reads)

```jsonc
{
  "summary": {
    "files_written": 2,
    "files_failed": 1,
    "fields_written": 8
  },
  "results": [
    { "path": "/path/to/track.wav", "status": "ok", "fields_written": ["artist", "title"], "wav_targets": ["id3v2", "riff_info"] },
    { "path": "/path/to/readonly.flac", "status": "error", "error": "Permission denied" },
    { "path": "/path/to/other.flac", "status": "ok", "fields_written": ["artist", "title", "album"], "fields_deleted": [] }
  ]
}
```

Partial failures do not abort the batch — each entry is independent. The tool returns success at the MCP level; the agent checks `files_failed` and per-entry `status` to detect issues.

Design notes:

- Album-wide writes: the agent repeats the same tags object across entries. No special mode.
- Per-track writes: each entry has different tags. Natural.
- Mixed: some entries set artist+title, others set album+year. Each entry is independent.
- WAV dual-tag default (both layers) encodes the project convention. Override with `wav_targets: ["riff_info"]` for tag-3-only repair.

### `extract_cover_art`

Extract embedded cover art from an audio file and save it to disk. The inverse of `embed_cover_art`.

#### Parameters

```jsonc
{
  "path": "/path/to/track.flac",             // source audio file
  "output_path": "/path/to/cover.jpg",       // where to save (optional)
  "picture_type": "front_cover"              // which art to extract (default: front_cover)
}
```

If `output_path` is omitted, the tool writes to `{parent_dir}/cover.{ext}` where `ext` is inferred from the embedded image format (jpeg, png, etc.).

For WAV files, cover art is read from ID3v2 (tag 2) only — RIFF INFO does not support embedded images.

#### Response

```jsonc
{
  "path": "/path/to/track.flac",
  "output_path": "/path/to/cover.jpg",
  "image_format": "jpeg",
  "size_bytes": 45230,
  "picture_type": "front_cover"
}
```

Returns an error if the file has no embedded cover art of the requested type.

### `embed_cover_art`

Embed cover art into audio files. Separate from `write_file_tags` because it handles binary data and has different parameters.

#### Parameters

```jsonc
{
  "image_path": "/path/to/cover.jpg",        // source image
  "targets": ["/path/to/01.flac", "/path/to/02.flac"],  // files to embed into
  "picture_type": "front_cover"               // default: front_cover
}
```

WAV files always embed into ID3v2 only (Rekordbox can't read WAV cover art anyway). No `wav_target` override needed.

#### Response

```jsonc
{
  "summary": {
    "files_embedded": 2,
    "files_failed": 0,
    "image_format": "jpeg",
    "image_size_bytes": 45230
  },
  "results": [
    { "path": "/path/to/01.flac", "status": "ok" },
    { "path": "/path/to/02.flac", "status": "ok" }
  ]
}
```

## CLI Subcommands

Human-readable output by default. `--json` flag for structured output (JSONL for batch results, single JSON object for single-file results). Progress/errors to stderr, data to stdout.

### `read-tags`

```sh
# Single file (human-readable)
reklawdbox read-tags track.wav

# Batch with glob
reklawdbox read-tags /path/to/album/*.flac

# Filtered fields
reklawdbox read-tags --fields artist,title,year /path/to/album/

# Machine output (JSONL)
reklawdbox read-tags --json /path/to/album/*.wav

# Pipe to jq
reklawdbox read-tags --json track.wav | jq '.tags.artist'
```

Human default:

```
=== track.wav (WAV) ===
ID3v2:
  Artist       Bicep
  Title        Glue
  Album        Isles
  Year         2021

RIFF INFO:
  Artist       Bicep
  Title        Glue
  Album        (missing)
  Year         (missing)
```

### `write-tags`

```sh
# Set fields
reklawdbox write-tags track.wav --artist "Bicep" --title "Glue" --year "2021"

# Delete a field (empty string = delete, same as MCP null)
reklawdbox write-tags track.wav --genre ""

# Dry run
reklawdbox write-tags --dry-run track.wav --artist "Bicep"

# WAV target override
reklawdbox write-tags --wav-targets riff_info track.wav --artist "Bicep"

# JSON input for scripting (reads from stdin)
echo '{"artist":"Bicep","title":"Glue"}' | reklawdbox write-tags --json-input track.wav
```

### `extract-art`

```sh
# Extract to auto-named file (cover.jpg in same directory)
reklawdbox extract-art track.flac

# Extract to specific path
reklawdbox extract-art track.flac --output /path/to/cover.jpg

# Machine output
reklawdbox extract-art --json track.flac
```

### `embed-art`

```sh
reklawdbox embed-art cover.jpg track1.flac track2.flac track3.flac
```

## Validation

Minimal, targeted validation — only fields where format matters for Rekordbox import:

| Field | Rule | Error behavior |
| ----- | ---- | -------------- |
| `year` | Must be 4-digit integer (YYYY) or empty/null | Reject with clear error |
| `track` | Must be positive integer or empty/null | Reject with clear error |
| `disc` | Must be positive integer or empty/null | Reject with clear error |

All other fields: write raw values as-is. Convention enforcement (no artist-in-title, genre blank in files, etc.) lives in the agent/SOP layer, not the tool.

## Field Mapping

Canonical field names used in the API, mapped to format-specific tag keys. A dash means the field is unavailable for that format — writes silently skip it, reads return `null`.

<!-- dprint-ignore -->
| Field          | Vorbis Comment (FLAC) | ID3v2 frame (MP3/WAV) | RIFF INFO chunk (WAV) | MP4 atom (M4A)        | lofty ItemKey       |
| -------------- | --------------------- | --------------------- | --------------------- | --------------------- | ------------------- |
| `artist`       | `ARTIST`              | `TPE1`                | `IART`                | `\xa9ART`             | `TrackArtist`       |
| `title`        | `TITLE`               | `TIT2`                | `INAM`                | `\xa9nam`             | `TrackTitle`        |
| `album`        | `ALBUM`               | `TALB`                | `IPRD`                | `\xa9alb`             | `AlbumTitle`        |
| `album_artist` | `ALBUMARTIST`         | `TPE2`                | —                     | `aART`                | `AlbumArtist`       |
| `genre`        | `GENRE`               | `TCON`                | `IGNR`                | `\xa9gen`             | `Genre`             |
| `year`         | `DATE`                | `TDRC`                | `ICRD`                | `\xa9day`             | `Year`              |
| `track`        | `TRACKNUMBER`         | `TRCK`                | —                     | `trkn`                | `TrackNumber`       |
| `disc`         | `DISCNUMBER`          | `TPOS`                | —                     | `disk`                | `DiscNumber`        |
| `comment`      | `COMMENT`             | `COMM`                | `ICMT`                | `\xa9cmt`             | `Comment`           |
| `publisher`    | `LABEL`               | `TPUB`                | —                     | freeform `LABEL`      | `Label`             |
| `bpm`          | `BPM`                 | `TBPM`                | —                     | `tmpo`                | `Bpm`               |
| `key`          | `INITIALKEY`          | `TKEY`                | —                     | freeform `INITIALKEY` | `InitialKey`        |
| `composer`     | `COMPOSER`            | `TCOM`                | —                     | `\xa9wrt`             | `Composer`          |
| `remixer`      | `REMIXER`             | `TPE4`                | —                     | freeform `REMIXER`    | `Remixer`           |

Notes:
- `publisher` maps to `ItemKey::Label` (not `ItemKey::Publisher`) to match Rekordbox's `Label` field.
- `year`: FLAC stores as Vorbis Comment `DATE`. A file's year is populated if either `DATE` or `YEAR` is present; on write, always use `DATE`.
- `track`/`disc`: ID3v2 `TRCK`/`TPOS` may contain `N/M` (track/total) format. On read, return as-is. Validation rejects non-integer values on write.
- RIFF INFO has a limited vocabulary — fields marked `—` are unavailable. WAV dual-tag writes silently skip unavailable fields for the RIFF INFO layer.

## WAV Dual-Tag Convention

Rekordbox reads **only RIFF INFO (tag 3)** from WAV files. ID3v2 (tag 2) is ignored by Rekordbox but used by other apps and for cover art embedding.

| Operation | Default behavior |
| --------- | ---------------- |
| Read WAV | Return both `id3v2` and `riff_info` as separate objects |
| Write WAV | Write to both tag layers with the same values |
| Embed cover art into WAV | Write to ID3v2 only (Rekordbox can't read WAV cover art) |

Override write target per-entry with `wav_targets: ["riff_info"]` or `wav_targets: ["id3v2"]`.

## Performance

- File I/O via `tokio::task::spawn_blocking` with semaphore-bounded concurrency (8 concurrent reads)
- Writes are sequential (avoid corruption on shared filesystems)
- `ParseOptions::read_cover_art(false)` by default unless `include_cover_art: true`
- lofty uses padding to avoid full file rewrites when tag size fits existing padding (1024 bytes default)
- Target: 1,000 file reads in < 5 seconds (vs ~5 minutes with exiftool)

## Workflow Impact

| Workflow | Before | After |
| -------- | ------ | ----- |
| Batch import (per album) | ~12 kid3-cli/exiftool calls | 4 MCP calls (read, dry-run, write, verify) |
| Collection audit (1000 files) | exiftool -j, ~5 min | 1 MCP call, < 5 sec |
| WAV tag 3 fix | 2 exiftool reads + 1 kid3-cli write | 2 MCP calls (read + write) |
| Cover art embed | 1 kid3-cli call per file | 1 MCP call (batch) |
| Cover art extract | exiftool + manual save | 1 MCP call (extract_cover_art) |
| File rename from tags | kid3-cli fromtag | read_file_tags + agent constructs mv commands |

## Not In Scope

- **Genre writing to files.** Genre is managed exclusively in Rekordbox. The tool can write genre (wide API), but the convention is to leave it blank in files.
- **Rename-from-tags tool.** The agent reads tags via `read_file_tags`, constructs filenames, and uses shell `mv`. No dedicated tool needed.
- **Direct DB writes.** Tags are file-level metadata. Rekordbox DB changes go through the existing `update_tracks` / `write_xml` pipeline.
- **Tag type conversion.** No converting between ID3v2 and Vorbis Comments. Each format uses its native tag type.
