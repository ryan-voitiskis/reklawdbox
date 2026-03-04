# Batch Import Preparation — Agent SOP

Process newly acquired music (downloaded albums, loose tracks, zips) into an organized structure ready for Rekordbox import. Follow step-by-step.

**Goal:** After this SOP, any processed track can be imported into Rekordbox with Artist, Title, Album, Year, Track Number, and Label displaying correctly. Genre is handled separately via the genre classification SOP.

## Rekordbox Import Readiness

See [conventions.md § Rekordbox import readiness](../conventions.md#rekordbox-import-readiness) for the full format-specific tag mapping table, WAV dual-tag rule, and WAV cover art limitation.

## Constraints

- **Tags are source of truth.** Write tags before renaming — filenames are derived from tags.
- **Never set genre.** Leave genre tags empty; handled via genre classification SOP.
- **Stop on ambiguity.** Never guess artist names, album years, or label names — ask the user.
- **Process album by album.** Complete one fully before starting the next.
- **WAV dual-tag rule.** Always write both tag 2 and tag 3 for WAV files.
- **Verify before moving.** Confirm tags and filenames before moving to final location.

## Prerequisites

<!-- dprint-ignore -->
| Tool | Purpose | Install |
|------|---------|---------|
| `reklawdbox` MCP | Tag reading/writing (`read_file_tags`, `write_file_tags`), Discogs/Beatport lookups | This project |
| `kid3-cli` | File renaming from tags, cover art embedding | `brew install kid3` |
| `unzip` | Extract zip archives | Pre-installed on macOS |

`lookup_discogs`, `lookup_beatport`, `read_file_tags`, and `write_file_tags` are MCP tool calls, not shell commands.

**Shell note:** Claude Code does not persist shell state between tool calls. All shell snippets below use literal paths — substitute the actual path for each invocation.

## Convention Reference

See [conventions.md](../conventions.md) for directory structure, file naming, required tags, album directory naming, and album type classification.

---

## Phase 1: Assessment

### Step 1: List the batch directory

```sh
ls -la "/path/to/batch/"
```

Categorize: directories (albums/EPs), audio files at root (loose tracks), zip files (need extraction).

### Step 2: Handle zip files

Extract all root-level zips. Already-extracted zips (matching directory exists with files) are archived without re-extraction. Failed extractions go to `_failed_zips/`.

```sh
cd "/path/to/batch"
mkdir -p "_processed_zips" "_failed_zips"
find . -maxdepth 1 -type f -name "*.zip" -print0 | while IFS= read -r -d '' zip; do
    zip="${zip#./}"
    dir="${zip%.zip}"

    if [ -d "$dir" ] && find "$dir" -type f -print -quit | grep -q .; then
        mv "$zip" "_processed_zips/$zip"
        echo "Archived already-extracted: $zip"
        continue
    fi

    tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/batch-import-unzip.XXXXXX")"
    if unzip -o "$zip" -d "$tmp_dir"; then
        mkdir -p "$dir"
        find "$tmp_dir" -mindepth 1 -maxdepth 1 -exec mv -n {} "$dir"/ \;
        if find "$dir" -type f -print -quit | grep -q .; then
            mv "$zip" "_processed_zips/$zip"
            echo "Extracted: $zip"
        else
            mv "$zip" "_failed_zips/$zip"
            echo "No files extracted: $zip"
        fi
    else
        mv "$zip" "_failed_zips/$zip"
        echo "Extraction failed: $zip"
    fi
    rm -rf "$tmp_dir"
done
```

### Step 3: Report to user

Summarize what was found: album directories, loose tracks, zip results.

---

## Phase 2: Process Albums

For each album subdirectory, follow these steps in order.

### Step 1: Survey current state

```sh
ls -la "/path/to/batch/Album Directory/"
```

Read tags for all files in the directory:

```
read_file_tags(paths=["/path/to/batch/Album Directory/"])
```

Note: current filename pattern, which tags are present, whether cover art exists.

### Step 2: Parse directory name

Common incoming patterns:

- `Artist Name - Album Name`
- `Artist Name - Album Name (Year)`
- `Artist Name - Album Name [FLAC 24-96]`

Extract: artist, album name, year (if present).

### Step 3: Determine album type

- **Same artist on all tracks** → Single Artist
- **Different artists per track** → VA Compilation
- **"Various Artists" in dir name** → VA Compilation

For VA: label name is **required**. Check Publisher tag, directory name, or look up.

### Step 4: Look up metadata

```
lookup_discogs(artist="Artist Name", title="First Track Title", album="Album Name")
```

If no Discogs result, try Beatport:

```
lookup_beatport(artist="Artist Name", title="First Track Title")
```

Use results for: release year, label name, artist/album spelling verification.

**Stop and ask** on: multiple matches with different years, no results and year/label unknown, ambiguous artist.

Never use lookup results for genre.

### Step 5: Write tags

Use `write_file_tags` for all tag writes. It handles WAV dual-tagging automatically.

**Album-wide + per-track tags:**

```
write_file_tags(writes=[
  {path: "/path/to/album/01 original.flac", tags: {
    artist: "Track Artist", title: "Track Title", track: 1,
    album: "Album Name", year: "YEAR", publisher: "Label Name",
    album_artist: "Artist Name"
  }},
  ...
])
```

**Per-track tags** — parse from filenames. Common incoming patterns:

<!-- dprint-ignore -->
| Pattern | Parse as |
|---------|----------|
| `Artist - Album - NN Title.wav` | Track N: Artist - Title |
| `NN Artist - Title.wav` | Track N: Artist - Title |
| `NN Title.wav` | Track N: [AlbumArtist] - Title |
| `NN. Title.wav` | Track N: [AlbumArtist] - Title |

### Step 6: Verify tags

```
read_file_tags(paths=["/path/to/album/"])
```

Confirm every file has: Artist, Title, Track, Album, Year.

### Step 7: Rename files from tags

```sh
kid3-cli -c "select all" \
         -c "fromtag '%{track} %{artist} - %{title}' 2" \
         -c "save" .
```

Expected result: `01 Artist Name - Track Title.ext`

If rename produces unexpected results, stop and check tags — rename depends entirely on tag correctness.

### Step 8: Embed cover art

```sh
find . -maxdepth 1 -type f \( -iname "*.jpg" -o -iname "*.jpeg" -o -iname "*.png" \) -print
```

**Single obvious cover** (`cover.jpg`, `front.jpg`, `folder.jpg`):

```sh
kid3-cli -c "select all" \
         -c "set picture:'cover.jpg' 'Cover (front)' ''" \
         -c "save" .
```

**Multiple images:** Ask user which is the cover.

**No images:** Try `lookup_discogs(...)` — if result includes `cover_image`, download and embed:

```sh
curl -s -o "cover.jpg" "COVER_IMAGE_URL"
kid3-cli -c "select all" \
         -c "set picture:'cover.jpg' 'Cover (front)' ''" \
         -c "save" .
```

If no cover from Discogs either, note for user to source manually.

### Step 9: Create target directory and move files

Determine clean directory name (strip tech specs, add year, clean special chars).

```sh
# Single Artist
mkdir -p "/path/to/dest/Artist Name/Album Name (Year)"

# Or VA
mkdir -p "/path/to/dest/Various Artists/Label Name/Album Name (Year)"

# Move audio + cover art
find "/path/to/batch/Old Dir" -maxdepth 1 -type f \
     \( -iname "*.wav" -o -iname "*.flac" -o -iname "*.mp3" \) \
     -exec mv -n {} "/path/to/dest/Artist Name/Album Name (Year)/" \;
find "/path/to/batch/Old Dir" -maxdepth 1 -type f -iname "cover.*" \
     -exec mv -n {} "/path/to/dest/Artist Name/Album Name (Year)/" \;

# Remove old empty directory
rmdir "/path/to/batch/Old Dir"
```

### Step 10: Verify final state

```sh
ls -la "/path/to/dest/Artist Name/Album Name (Year)/"
```

```
read_file_tags(paths=["/path/to/dest/Artist Name/Album Name (Year)/01 Artist - Track.ext"])
```

Confirm: files in correct location, `NN Artist - Title.ext` format, all tags present, VA has label subdirectory, cover art embedded.

---

## Phase 3: Process Loose Tracks

For audio files at the root of the batch directory.

### Step 1: Clean filenames

Remove `(Original Mix)` suffix. Keep other parenthetical info (`(Remix)`, `(Edit)`, etc.):

```sh
cd "/path/to/batch"
find . -maxdepth 1 -type f -name "* (Original Mix).*" -print0 | while IFS= read -r -d '' f; do
    new_name="${f/ (Original Mix)/}"
    mv "$f" "$new_name"
done
```

Expected format: `Artist Name - Track Title.ext`. If unparseable, ask user.

### Step 2: Read/write tags

For each loose track, read existing tags:

```
read_file_tags(paths=["/path/to/batch/Artist - Title.wav"])
```

If tags are missing, look up with `lookup_discogs(...)` / `lookup_beatport(...)`.

Write minimum tags (WAV dual-tagging handled automatically):

```
write_file_tags(writes=[{
  path: "/path/to/batch/Artist - Title.wav",
  tags: {artist: "Artist Name", title: "Track Title", publisher: "Label Name"}
}])
```

### Step 3: Embed cover art (if available)

Check for `cover_Artist Name - Track Title.jpg` files. If found, embed. If not, try Discogs `cover_image`. Note any tracks without cover art in the report.

---

## Phase 4: Multi-Disc Albums

If an album has disc subdirectories (`CD1/`, `CD2/`, `Disc 1/`, etc.):

- Track numbers restart at 01 per disc
- Cover art at album root (not in disc folders)
- Set Disc Number tag on each track via `write_file_tags` (WAV dual-tagging handled automatically)
- Album-wide tags (album, year, publisher) go on all tracks across all discs

---

## Phase 5: Final Verification

Summarize: albums processed (single artist vs VA), loose tracks processed, any unresolved items, WAV tracks needing manual cover art in Rekordbox, and next steps (import, cover art, collection audit SOP, genre classification SOP).

---

## Decision Reference

### Proceed automatically when

- Artist clearly identified in tags, filename, or directory name
- Year present in tags, directory name, or single clear Discogs match
- Label present in tags or Discogs (for VA)
- Single artist with consistent tags across album

### Stop and ask when

- Multiple matches with different years
- No results and year/label unknown
- VA album but label unknown
- Ambiguous: collaboration vs VA vs single artist
- Multiple images — which is album cover?
- Conflicting metadata between tags and filenames
- Unparseable filenames

---

## Common Incoming Filename Patterns

### Album tracks

<!-- dprint-ignore -->
| Pattern | Parse as |
|---------|----------|
| `Artist - Album - NN Title.wav` | Track N: Artist - Title |
| `Artist - Album - NN. Title.wav` | Track N: Artist - Title |
| `NN Artist - Title.wav` | Track N: Artist - Title |
| `NN. Artist - Title.wav` | Track N: Artist - Title |
| `NN Title.wav` | Track N: [AlbumArtist] - Title |
| `NN. Title.wav` | Track N: [AlbumArtist] - Title |
| `Artist - Album - NN AX. Title.wav` | Track N: Artist - Title (vinyl) |

### Loose tracks

<!-- dprint-ignore -->
| Pattern | Status |
|---------|--------|
| `Artist - Title.wav` | Correct |
| `Artist - Title (Remix Info).wav` | Correct |
| `Artist, Artist B - Title.wav` | Correct |
| `Artist - Title (Original Mix).wav` | Remove "(Original Mix)" |
| `Title.wav` | Missing artist — ask user |
