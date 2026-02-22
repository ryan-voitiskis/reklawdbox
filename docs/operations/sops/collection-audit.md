# Collection Audit — Agent SOP

Detect and fix naming/tagging convention violations in a music collection. Follow step-by-step.

**Goal:** After this SOP, any track file can be imported into Rekordbox with Artist, Title, Album, Year, Track Number, and Label displaying correctly. Genre is handled separately via the genre classification SOP.

## Rekordbox Import Readiness

What Rekordbox reads on import by format:

| Field | FLAC | WAV | MP3 |
|-------|------|-----|-----|
| Artist | Vorbis Comment | RIFF INFO (tag 3) | ID3v2 (tag 2) |
| Title | Vorbis Comment | RIFF INFO (tag 3) | ID3v2 (tag 2) |
| Album | Vorbis Comment | RIFF INFO (tag 3) | ID3v2 (tag 2) |
| Year | Vorbis Comment | RIFF INFO (tag 3) | ID3v2 (tag 2) |
| Track | Vorbis Comment | RIFF INFO (tag 3) | ID3v2 (tag 2) |
| Cover art | Embedded (auto) | **Not imported** | Embedded (auto) |

**WAV critical:** Rekordbox reads **only** RIFF INFO (tag 3) from WAV files. ID3v2 (tag 2) is ignored. Both must be written — tag 2 for general compatibility, tag 3 for Rekordbox.

**WAV cover art:** Rekordbox cannot import cover art from WAV files. Embed in tag 2 for other apps, but note WAV tracks need manual cover art in Rekordbox after import.

## Constraints

- **Read-only by default.** No files modified until user approves a fix plan.
- **Stop on ambiguity.** If the correct fix isn't clear, flag for user review — never guess.
- **Process in small batches.** One artist or album at a time. Report progress after each.
- **Verify after fixing.** Re-read tags/filenames after every fix.
- **Never delete audio files.** Renaming and tag editing only.
- **WAV dual-tag rule.** Always write both tag 2 and tag 3 for WAV files.

## Prerequisites

| Tool | Purpose | Install |
|------|---------|---------|
| `kid3-cli` | Tag reading/writing, file renaming | `brew install kid3` |
| `exiftool` | Batch tag reading as JSON | `brew install exiftool` |
| `reklawdbox` MCP | Discogs/Beatport metadata lookups | This project |

`lookup_discogs` and `lookup_beatport` are MCP tool calls, not shell commands. If MCP is unavailable, continue the audit without lookups and mark metadata gaps for manual review.

## Convention Reference

### Directory structure

```
collection/
├── Artist Name/
│   └── Album Name (Year)/
│       ├── 01 Artist Name - Track Title.flac
│       └── cover.jpg
└── Various Artists/
    └── Label Name/
        └── Album Name (Year)/
            ├── 01 Artist A - Track Title.flac
            └── cover.jpg
```

### File naming

- **Album tracks:** `NN Artist Name - Track Title.ext` (zero-padded track number)
- **Loose tracks:** `Artist Name - Track Title.ext` (no track number)
- **Multi-disc:** Track numbers restart at 01 per disc, in `CD1/`, `CD2/` subdirs

### Required tags

| Tag | Format | Example |
|-----|--------|---------|
| Artist | Track-level artist | `Kangding Ray` |
| Title | Track title only (no artist prefix) | `Amber Decay` |
| Track | Integer (not zero-padded) | `1` |
| Album | Album name | `Ultrachroma` |
| Date/Year | YYYY | `2022` |

### Recommended tags

| Tag | When required |
|-----|---------------|
| Publisher/Organization | Always for VA, recommended for all |
| AlbumArtist | Required for VA, recommended for all |
| Disc | Required for multi-disc albums |

### Must NOT be set

| Tag | Reason |
|-----|--------|
| Genre | Left blank for Rekordbox curation via reklawdbox |

---

## Step 0: Scope Selection

Ask the user what to audit:
1. Entire collection directory
2. Entire play directory
3. Specific artist directory
4. Specific album
5. Specific play subdirectory (e.g., `play/play29/`)
6. Custom path

Confirm conventions or ask for overrides. Then count audio files:

```sh
find "/path/to/scope" -type f \( -name "*.flac" -o -name "*.wav" -o -name "*.mp3" -o -name "*.m4a" \) | wc -l
```

If >500 files, recommend processing in artist-level batches.

**Shell note:** Claude Code does not persist shell state between tool calls. All shell snippets below use literal paths — substitute the actual path selected by the user.

---

## Step 1: Filename Scan

**Goal:** Detect files whose names don't match conventions. Read-only — don't fix anything yet.

### 1a: Album track filename scan

For each album directory, check audio files against `NN Artist - Title.ext`:

```sh
find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -print0 | while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [[ ! "$name" =~ ^[0-9][0-9]\ .+\ -\ .+\..+$ ]]; then
        echo "MISMATCH: $f"
    fi
done
```

### 1b: Directory naming scan

Check for tech specs in names and missing years:

```sh
find "/path/to/scope" -type d -print0 | while IFS= read -r -d '' dir; do
    name=$(basename "$dir")
    if [[ "$name" =~ \[FLAC\]|\[WAV\]|\[MP3\]|24-96|16-44|24bit|PBTHAL|vtwin88cube ]]; then
        echo "TECH_SPECS: $dir"
    fi
    first_audio=$(find "$dir" -maxdepth 1 -type f \
        \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
        -print -quit)
    if [ -n "$first_audio" ]; then
        if [[ ! "$name" =~ \([0-9]{4}\) ]]; then
            echo "NO_YEAR: $dir"
        fi
    fi
done
```

### 1c: Loose track naming scan

For play directories, check loose tracks match `Artist - Title.ext`:

```sh
find "/path/to/play" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -print0 | while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [[ "$name" == *"(Original Mix)"* ]]; then
        echo "ORIGINAL_MIX: $name"
    elif [[ ! "$name" =~ ^.+\ -\ .+\..+$ ]]; then
        echo "BAD_LOOSE: $name"
    fi
done
```

---

## Step 2: Tag Audit

**Goal:** Read tags and detect missing, empty, or inconsistent metadata.

### 2a: Batch tag read

For each album directory:

```sh
find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -exec exiftool -j -Artist -Title -Album -Year -TrackNumber -Publisher -AlbumArtist {} + 2>/dev/null
```

For large scopes, process one top-level directory at a time.

### 2b: Issue detection

Check each file against these patterns:

| Issue | Detection | Auto-fixable? |
|-------|-----------|---------------|
| **A: Empty artist** | Artist tag empty/missing | Yes — parse from filename |
| **B: Artist-in-title** | Title contains `Artist - Title` | Yes — strip artist prefix |
| **C: Missing required tags** | Any of Artist/Title/Track/Album/Year empty | Sometimes — filename, directory, or Discogs |
| **D: WAV missing tag 3** | WAV has tag 2 but empty tag 3 | Yes — copy from tag 2 |
| **E: Genre tag set** | Genre not empty | Flag for user review |
| **F: Track number format** | Zero-padded in tag or missing | Yes — parse from filename |
| **G: Filename/tag mismatch** | Filename and tags disagree | No — flag for user review |

To check WAV tag 3:
```sh
kid3-cli -c "tag 3" -c "get" "/path/to/file.wav"
```

### 2c: Metadata lookups for gaps

When tags are missing and can't be derived from filenames, use MCP lookups:

```
lookup_discogs(artist="Artist Name", title="Track Title")
lookup_beatport(artist="Artist Name", title="Track Title")
```

Use results to fill Album, Year, and Publisher. Never auto-set Genre from lookups.

---

## Step 3: Issue Report

Present findings to the user in three categories:

1. **Auto-fixable** — unambiguous fixes, present as batch for single approval
2. **Needs per-item approval** — multiple valid fixes exist, present options for each
3. **Needs investigation** — user must inspect and decide

Include a summary with total files scanned, pass rate, and counts per issue type.

---

## Step 4: Fix Execution

### 4a: Pre-fix safety checkpoint

Before any file modifications:

```sh
# Confirm tools are runnable
command -v exiftool >/dev/null || { echo "Missing exiftool"; exit 1; }
kid3-cli --help >/dev/null 2>&1 || { echo "kid3-cli not runnable"; exit 1; }

# Create rollback backup of scope
tar -cf "./collection-audit-backup-$(date +%Y%m%d-%H%M%S).tar" \
    -C "/path/to/parent" "scope-dir"
```

Proceed only after backup succeeds and user confirms.

### 4b: Auto-fixes (after batch approval)

Process one album at a time.

#### Fix empty artist / artist-in-title

Parse artist and title from filename, write to tags. Use `APPLY=0` for dry-run preview first, `APPLY=1` to write:

```sh
escape_kid3() {
    printf "%s" "$1" | sed "s/'/\\\\'/g"
}

find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.wav" -o -iname "*.flac" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -print0 | while IFS= read -r -d '' path; do
    filename=$(basename "$path")
    base="${filename%.*}"

    if [[ ! "$base" =~ ^[0-9][0-9]\ .+\ -\ .+$ ]]; then
        echo "SKIP: $filename"
        continue
    fi

    track_num="${base:0:2}"
    rest="${base:3}"
    artist="${rest%% - *}"
    title="${rest#* - }"
    track_int=$((10#$track_num))

    echo "File: $filename -> Artist: $artist | Title: $title | Track: $track_int"

    [ "$APPLY" -eq 1 ] || continue

    artist_esc=$(escape_kid3 "$artist")
    title_esc=$(escape_kid3 "$title")
    filename_esc=$(escape_kid3 "$filename")

    case "${filename##*.}" in
        [Ww][Aa][Vv])
            kid3-cli -c "select '$filename_esc'" \
                     -c "tag 2" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set track number $track_int" \
                     -c "tag 3" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set track number $track_int" \
                     -c "save" .
            ;;
        [Mm]4[Aa])
            exiftool -overwrite_original \
                -Artist="$artist" -Title="$title" -TrackNumber="$track_int" \
                "$path" >/dev/null
            ;;
        *)
            kid3-cli -c "select '$filename_esc'" -c "tag 2" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set track number $track_int" \
                     -c "save" .
            ;;
    esac

    # Verify write
    actual_artist=$(exiftool -s3 -Artist "$path")
    actual_title=$(exiftool -s3 -Title "$path")
    actual_track=$(exiftool -s3 -TrackNumber "$path" | sed 's#/.*##')

    if [ "$actual_artist" = "$artist" ] && [ "$actual_title" = "$title" ] && [ "$actual_track" = "$track_int" ]; then
        echo "OK: $filename"
    else
        echo "VERIFY_FAIL: $filename"
    fi
done
```

#### Fix WAV tag 3 missing

Read tag 2 values, write to tag 3:

```sh
artist=$(exiftool -s3 -Artist "/path/to/file.wav")
title=$(exiftool -s3 -Title "/path/to/file.wav")
album=$(exiftool -s3 -Album "/path/to/file.wav")
year=$(exiftool -s3 -Year "/path/to/file.wav")
track=$(exiftool -s3 -TrackNumber "/path/to/file.wav" | sed 's#/.*##')

artist_esc=$(printf "%s" "$artist" | sed "s/'/\\\\'/g")
title_esc=$(printf "%s" "$title" | sed "s/'/\\\\'/g")
album_esc=$(printf "%s" "$album" | sed "s/'/\\\\'/g")

kid3-cli -c "select 'file.wav'" \
         -c "tag 3" \
         -c "set artist '$artist_esc'" \
         -c "set title '$title_esc'" \
         -c "set album '$album_esc'" \
         -c "set date $year" \
         -c "set track number $track" \
         -c "save" .
```

#### Fix (Original Mix) in filenames

Strip `(Original Mix)` from loose track filenames:

```sh
find "/path/to/play" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -name "* (Original Mix)*" -print0 | while IFS= read -r -d '' f; do
    base=$(basename "$f")
    new_base=$(printf "%s" "$base" | sed 's/ (Original Mix)//')
    new_path="$(dirname "$f")/$new_base"

    if [ -e "$new_path" ]; then
        echo "SKIP (target exists): $new_path"
        continue
    fi

    mv -- "$f" "$new_path" && echo "RENAMED: $base -> $new_base" || echo "FAIL: $base"
done
```

#### Fill missing tags from lookups

When Artist/Title are known but Album/Year/Publisher are missing, use `lookup_discogs(artist=..., title=...)`. Write tags on clear match; flag for user review on ambiguity.

### 4c: Per-item fixes

Present each ambiguous issue to the user with options and wait for approval before applying.

### 4d: Investigation items

Present context (filename vs tag values) and ask the user to decide which source is correct.

---

## Step 5: Verification

After fixing an album or batch:

```sh
# Re-read tags
find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -exec exiftool -j -Artist -Title -Album -Year -TrackNumber {} + 2>/dev/null

# Verify WAV tag 3
find "/path/to/album" -maxdepth 1 -type f \( -iname "*.wav" \) -print0 | \
while IFS= read -r -d '' f; do
    echo "=== $(basename "$f") tag 3 ==="
    kid3-cli -c "tag 3" -c "get" "$f"
done

# Re-check filenames
find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -print0 | while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [[ ! "$name" =~ ^[0-9][0-9]\ .+\ -\ .+\..+$ ]]; then
        echo "MISMATCH: $f"
    fi
done
```

---

## Step 6: Final Report

Summarize: scope, files scanned, pass rate, fixes applied by type, remaining issues, and next steps (manual review items, Rekordbox import, WAV cover art, genre classification SOP).

---

## Appendix A: Issue Detection Quick Reference

| Issue | Detection | Auto-fixable? | Fix method |
|-------|-----------|---------------|------------|
| Empty artist tag | exiftool -Artist shows empty | Yes (if filename has artist) | Parse from filename |
| Artist-in-title | Title contains `Artist - Title` | Yes | Strip artist prefix |
| Missing Track number | TrackNumber empty | Yes (if filename has NN) | Parse from filename |
| Missing Album tag | Album empty | Sometimes (from directory) | Directory name or Discogs |
| Missing Year tag | Year/Date empty | Sometimes | Discogs lookup |
| WAV tag 3 missing | kid3-cli tag 3 shows empty | Yes | Copy from tag 2 |
| Genre set | Genre not empty | Flag only | User decides keep/clear |
| Filename mismatch | Tags don't match filename | No — ambiguous | User review |
| Tech specs in dir name | Regex match `\[FLAC\]` etc. | Yes | Strip from dir name |
| Missing year in dir name | No `(YYYY)` in album dir | Sometimes | Discogs lookup |
| (Original Mix) suffix | Filename contains it | Yes | Rename |
| VA without label subdir | VA album not under label/ | No | User must provide label |

## Appendix B: Filename Parsing Rules

### Album tracks

Pattern: `NN Artist Name - Track Title.ext`

1. Strip file extension
2. First 2 chars = track number (zero-padded)
3. Char 3 = space (skip)
4. Char 4 to first ` - ` = artist
5. After first ` - ` = title

**Edge case:** Title contains ` - ` (e.g., "Artist - Track - Subtitle"). Split on first occurrence only, unless artist is known from directory.

**Edge case:** VA compilations — filename artist matches per-track artist tag, not AlbumArtist.

### Loose tracks

Pattern: `Artist Name - Track Title.ext` (no track number prefix)

## Appendix C: kid3-cli Patterns

### WAV dual-tag write

```sh
kid3-cli -c "select 'file.wav'" \
         -c "tag 2" \
         -c "set artist 'Name'" \
         -c "set title 'Title'" \
         -c "tag 3" \
         -c "set artist 'Name'" \
         -c "set title 'Title'" \
         -c "save" .
```

### Batch album-wide tags

```sh
kid3-cli -c "select all" -c "tag 2" \
         -c "set album 'Album Name'" \
         -c "set date 2022" \
         -c "set publisher 'Label Name'" \
         -c "set album artist 'Artist Name'" \
         -c "save" .
```

### Rename from tags

```sh
kid3-cli -c "select all" \
         -c "fromtag '%{track} %{artist} - %{title}' 2" \
         -c "save" .
```

### Embed cover art

```sh
kid3-cli -c "select all" \
         -c "set picture:'cover.jpg' 'Cover (front)' ''" \
         -c "save" .
```
