# Batch Import Preparation — Agent SOP

Standard operating procedure for processing new music into an organized structure ready for Rekordbox import. Agents must follow this document step-by-step.

## Overview

This SOP takes a batch of newly acquired music (downloaded albums, loose tracks, zips) and prepares it for Rekordbox import by:
1. Organizing into a consistent directory structure
2. Writing complete metadata tags
3. Embedding cover art
4. Verifying everything conforms to conventions

It replaces manual workflows by using reklawdbox MCP tools for metadata lookups (Discogs, Beatport) and CLI tools for file operations.

**Goal:** After running this SOP, any processed track can be dragged into Rekordbox and have Artist, Title, Album, Year, Track Number, and Label display correctly without manual editing. Genre is handled separately via the genre classification SOP after import.

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

**WAV cover art limitation:** Rekordbox does not read embedded cover art from WAV files. RIFF INFO has no standard picture field, and Rekordbox ignores ID3v2 picture tags in WAVs. Cover art for WAV tracks must be added manually in Rekordbox after import. The SOP still embeds cover art into WAV tag 2 for compatibility with other applications, but tells the user which WAV tracks need manual cover art in Rekordbox.

**Critical:** WAV files **must** have RIFF INFO (tag 3) tags. Rekordbox ignores ID3v2 in WAV files. If only tag 2 is written, Rekordbox shows blank fields.

## Constraints

- **Tags are source of truth.** Metadata in tags drives filenames, not the reverse. Always write tags before renaming.
- **Never set genre.** Genre is curated manually in Rekordbox via the genre classification SOP. Leave genre tags empty.
- **Stop on ambiguity.** When metadata cannot be determined with confidence, stop and ask the user. Never guess artist names, album years, or label names.
- **Process album by album.** Complete one album fully before starting the next. Report progress after each.
- **WAV dual-tag rule.** WAV files must have tags in both tag 2 (ID3v2) and tag 3 (RIFF INFO). Always write both.
- **Verify before moving.** Confirm tags and filenames are correct before moving files to their final location.

## Prerequisites

### Required tools

| Tool | Purpose | Install |
|------|---------|---------|
| `kid3-cli` | Tag writing, file renaming, cover art embedding | `brew install kid3` |
| `exiftool` | Tag reading as JSON | `brew install exiftool` |
| `reklawdbox` MCP | Discogs/Beatport metadata lookups | This project |
| `unzip` | Extract zip archives | Pre-installed on macOS |

---

## Convention Reference

### Target structure

**Single artist albums:**
```
destination/
└── Artist Name/
    └── Album Name (Year)/
        ├── 01 Artist Name - Track Title.flac
        ├── 02 Artist Name - Track Title.flac
        └── cover.jpg
```

**Various Artists compilations:**
```
destination/
└── Various Artists/
    └── Label Name/
        └── Album Name (Year)/
            ├── 01 Artist A - Track Title.flac
            ├── 02 Artist B - Track Title.flac
            └── cover.jpg
```

**Loose tracks (play directories only):**
```
destination/
├── Artist Name - Track Title.wav    ← at root, no subdirectory
└── cover_Artist Name - Track Title.jpg
```

### File naming

- **Album tracks:** `NN Artist Name - Track Title.ext`
  - `NN` = zero-padded track number (01, 02, ...)
  - Space-hyphen-space separator between artist and title
- **Loose tracks:** `Artist Name - Track Title.ext` (no track number)

### Required tags

| Tag | Album tracks | Loose tracks |
|-----|-------------|-------------|
| Artist | Required | Required |
| Title | Required | Required |
| Track | Required | Not needed |
| Album | Required | Optional |
| Date/Year | Required | Optional |
| Publisher | Recommended (required for VA) | Optional |
| AlbumArtist | Required for VA | Not needed |

### Album directory naming

- Format: `Album Name (Year)/`
- No artist name in album directory
- Remove: `[FLAC]`, `[WAV]`, `24-96`, usernames, catalog numbers
- Preserve: `(Deluxe Edition)`, `Vol. 1`, `(Remastered)`
- Replace `/` with `-`, `:` with `-`

### Album type classification

| Pattern | Type | Directory structure |
|---------|------|-------------------|
| All tracks same artist | Single Artist | `Artist/Album (Year)/` |
| Different artists per track | VA Compilation | `Various Artists/Label/Album (Year)/` |
| Multiple named artists (A & B) | Collaboration | `Various Artists/Label/Album (Year)/` |

---

## Phase 1: Assessment

### Step 1: List the batch directory

```sh
ls -la "BATCH_PATH/"
```

Categorize contents:
- **Directories** → likely albums/EPs
- **Audio files at root** → loose tracks
- **Zip files** → need extraction first

### Step 2: Handle zip files

```sh
# List all zip files
ls "BATCH_PATH/"*.zip 2>/dev/null
```

For each zip:

1. Check if already extracted (matching directory exists):
   ```sh
   # If dir exists, delete the zip
   rm "BATCH_PATH/Artist - Album.zip"
   ```

2. Extract if not yet extracted:
   ```sh
   cd "BATCH_PATH"
   unzip -o "Artist - Album.zip"
   ```

3. Verify extraction — some zips extract flat instead of into a subdirectory. If files appeared at root, create a directory and move them in.

4. Delete zip after successful extraction.

**Quick cleanup** — delete all zips that have matching directories:
```sh
cd "BATCH_PATH"
for zip in *.zip; do
    dir="${zip%.zip}"
    [ -d "$dir" ] && rm "$zip" && echo "Deleted $zip (already extracted)"
done
```

### Step 3: Report to user

```
Found in [batch]:
- X album directories
- Y loose tracks
- Z zip files (extracted/pending)

Proceeding with album processing...
```

---

## Phase 2: Process Albums

For each album subdirectory, follow these steps in order.

### Step 1: Survey current state

```sh
# List contents
ls -la "BATCH_PATH/Album Directory/"

# Read existing tags
cd "BATCH_PATH/Album Directory"
exiftool -j -Artist -Title -Album -Year -TrackNumber -Publisher *.flac *.wav *.mp3 2>/dev/null
```

Note:
- Current filename pattern
- Which tags are already present
- Whether cover art exists (`cover.jpg`, `front.jpg`, etc.)

### Step 2: Parse directory name

Common incoming patterns:
- `Artist Name - Album Name`
- `Artist Name - Album Name (Year)`
- `Artist Name - Album Name [FLAC 24-96]`
- `Various Artists - Album Name`

Extract: artist, album name, year (if present).

### Step 3: Determine album type

Check if all tracks have the same artist:
- **Same artist** → Single Artist album
- **Different artists** → VA Compilation
- **"Various Artists" or multiple artists in dir name** → VA Compilation

For VA: a label name is **required**. Check tags (Publisher field), directory name, or look up.

### Step 4: Look up metadata with reklawdbox

Use reklawdbox MCP for metadata lookups instead of shell scripts.

**For album-level metadata (year, label):**

```
lookup_discogs(artist="Artist Name", title="First Track Title", album="Album Name")
```

If Discogs returns no result or low confidence, try Beatport:

```
lookup_beatport(artist="Artist Name", title="First Track Title")
```

**Use lookup results for:**
- Release year (required for directory name)
- Label name (required for VA albums)
- Verification of artist/album spelling

**When to stop and ask:**
- Multiple Discogs matches with different years → present options
- No results from either provider → ask user
- Ambiguous artist/album name → ask user

**Never use lookup results for:**
- Genre (leave blank)

### Step 5: Write tags

#### Album-wide tags first

**For FLAC/MP3:**
```sh
cd "BATCH_PATH/Album Directory"
kid3-cli -c "select all" -c "tag 2" \
         -c "set album 'Album Name'" \
         -c "set date YEAR" \
         -c "set publisher 'Label Name'" \
         -c "set album artist 'Artist Name'" \
         -c "save" .
```

**For WAV (both tag 2 AND tag 3):**
```sh
kid3-cli -c "select all" \
         -c "tag 2" \
         -c "set album 'Album Name'" \
         -c "set date YEAR" \
         -c "set publisher 'Label Name'" \
         -c "set album artist 'Artist Name'" \
         -c "tag 3" \
         -c "set album 'Album Name'" \
         -c "set date YEAR" \
         -c "set publisher 'Label Name'" \
         -c "set album artist 'Artist Name'" \
         -c "save" .
```

#### Per-track tags

Parse track info from filenames. Common incoming patterns:

| Pattern | Parse as |
|---------|----------|
| `Artist - Album - NN Title.wav` | Track N: Artist - Title |
| `NN Artist - Title.wav` | Track N: Artist - Title |
| `NN Title.wav` | Track N: [Album Artist] - Title |
| `NN. Title.wav` | Track N: [Album Artist] - Title |

For each track:

**FLAC/MP3:**
```sh
kid3-cli -c "select 'original-filename.flac'" -c "tag 2" \
         -c "set artist 'Track Artist'" \
         -c "set title 'Track Title'" \
         -c "set track number N" \
         -c "save" .
```

**WAV:**
```sh
kid3-cli -c "select 'original-filename.wav'" \
         -c "tag 2" \
         -c "set artist 'Track Artist'" \
         -c "set title 'Track Title'" \
         -c "set track number N" \
         -c "tag 3" \
         -c "set artist 'Track Artist'" \
         -c "set title 'Track Title'" \
         -c "set track number N" \
         -c "save" .
```

### Step 6: Verify tags

```sh
exiftool -j -Artist -Title -Album -Year -TrackNumber *.flac *.wav *.mp3 2>/dev/null
```

Confirm every file has: Artist, Title, Track, Album, Year.

### Step 7: Rename files from tags

```sh
kid3-cli -c "select all" \
         -c "fromtag '%{track} %{artist} - %{title}' 2" \
         -c "save" .
```

Verify:
```sh
ls -la
```

Expected: `01 Artist Name - Track Title.ext`

**If kid3-cli rename produces unexpected results** (wrong format, missing fields), stop and check tags. The rename depends entirely on tag correctness.

### Step 8: Embed cover art

```sh
# Check for cover art files
ls *.{jpg,jpeg,png,JPG,PNG} 2>/dev/null
```

**Single obvious cover file** (`cover.jpg`, `front.jpg`, `folder.jpg`):
```sh
kid3-cli -c "select all" \
         -c "set picture:'cover.jpg' 'Cover (front)' ''" \
         -c "save" .
```

**Multiple images or unclear:** Ask user which is the album cover.

**No images:** Try fetching from Discogs. `lookup_discogs` returns a `cover_image` URL when available:

```
lookup_discogs(artist="Artist Name", title="First Track", album="Album Name")
```

If the result includes a non-empty `cover_image`:

```sh
curl -s -o "ALBUM_PATH/cover.jpg" "COVER_IMAGE_URL"

# Embed into all tracks
cd "ALBUM_PATH"
kid3-cli -c "select all" \
         -c "set picture:'cover.jpg' 'Cover (front)' ''" \
         -c "save" .
```

If no cover art from Discogs either, note it in the report for the user to source manually.

### Step 9: Create target directory structure

**Determine clean directory name:**
- Strip tech specs: `[FLAC]`, `24-96`, usernames
- Add year if missing: `Album Name` → `Album Name (2022)`
- Clean special characters: `/` → `-`, `:` → `-`

**Single Artist:**
```sh
mkdir -p "DEST/Artist Name/Album Name (Year)"
```

**Various Artists:**
```sh
mkdir -p "DEST/Various Artists/Label Name/Album Name (Year)"
```

### Step 10: Move files

```sh
# Move all audio and cover art
mv "BATCH_PATH/Old Dir/"*.{wav,flac,mp3} "DEST/Artist Name/Album Name (Year)/" 2>/dev/null
mv "BATCH_PATH/Old Dir/"cover.* "DEST/Artist Name/Album Name (Year)/" 2>/dev/null

# Remove old empty directory
rmdir "BATCH_PATH/Old Dir"
```

### Step 11: Verify final state

```sh
ls -la "DEST/Artist Name/Album Name (Year)/"
kid3-cli -c "get" "DEST/Artist Name/Album Name (Year)/01"*
```

Confirm:
- [ ] Files in correct location
- [ ] Filename format: `NN Artist - Title.ext`
- [ ] Tags complete (Artist, Title, Track, Album, Year)
- [ ] VA has label subdirectory
- [ ] Cover art embedded (if available)

### Report

```
✓ Album: Artist Name - Album Name (Year)
  → destination/Artist Name/Album Name (Year)/
  Tracks: N files
  Cover: embedded / not available
  Source: Discogs (release #12345)
```

---

## Phase 3: Process Loose Tracks

For audio files at the root of the batch directory.

### Step 1: List loose tracks

```sh
ls "BATCH_PATH/"*.wav "BATCH_PATH/"*.flac "BATCH_PATH/"*.mp3 2>/dev/null
```

### Step 2: Clean filenames

#### Remove "(Original Mix)" suffix

```sh
cd "BATCH_PATH"
for f in *" (Original Mix)".*; do
    [ -e "$f" ] && mv "$f" "${f/ (Original Mix)/}"
done
```

Keep other parenthetical info: `(Remix)`, `(Edit)`, `(Artist Remix)`.

#### Verify format

Expected: `Artist Name - Track Title.ext`

If incorrectly named, parse what's available and rename. If unparseable, ask user.

### Step 3: Read/write tags

For each loose track:

```sh
# Read existing tags (both tag types for WAV)
kid3-cli -c "get" "BATCH_PATH/Artist - Title.wav"
kid3-cli -c "tag 3" -c "get" "BATCH_PATH/Artist - Title.wav"
```

**If Artist and Title are missing,** look up with reklawdbox:

```
lookup_discogs(artist="Artist Name", title="Track Title")
lookup_beatport(artist="Artist Name", title="Track Title")
```

**Write minimum tags:**

**FLAC/MP3:**
```sh
kid3-cli -c "select 'Artist - Title.flac'" -c "tag 2" \
         -c "set artist 'Artist Name'" \
         -c "set title 'Track Title'" \
         -c "set publisher 'Label Name'" \
         -c "save" "BATCH_PATH"
```

**WAV:**
```sh
kid3-cli -c "select 'Artist - Title.wav'" \
         -c "tag 2" \
         -c "set artist 'Artist Name'" \
         -c "set title 'Track Title'" \
         -c "set publisher 'Label Name'" \
         -c "tag 3" \
         -c "set artist 'Artist Name'" \
         -c "set title 'Track Title'" \
         -c "set publisher 'Label Name'" \
         -c "save" "BATCH_PATH"
```

### Step 4: Embed cover art (if available)

Check for existing cover files matching the loose track naming convention:

```sh
ls "BATCH_PATH/"cover_*.jpg 2>/dev/null
```

For tracks with a matching `cover_Artist Name - Track Title.jpg` file:

```sh
cd "BATCH_PATH"
kid3-cli -c "select 'Artist - Title.wav'" \
         -c "set picture:'cover_Artist Name - Track Title.jpg' 'Cover (front)' ''" \
         -c "save" .
```

**For tracks without a cover file,** try fetching from Discogs:

```
lookup_discogs(artist="Artist Name", title="Track Title")
```

If the result includes a non-empty `cover_image`:

```sh
curl -s -o "BATCH_PATH/cover_Artist Name - Track Title.jpg" "COVER_IMAGE_URL"

kid3-cli -c "select 'Artist - Title.wav'" \
         -c "set picture:'cover_Artist Name - Track Title.jpg' 'Cover (front)' ''" \
         -c "save" "BATCH_PATH"
```

If no Discogs match or empty `cover_image`, note it in the report.

### Step 5: Report

```
✓ Loose tracks processed: N files
  Named correctly: N
  Tags written: N
  Cover art: X embedded, Y not found
```

---

## Phase 4: Multi-Disc Albums

If an album has disc subdirectories (`CD1/`, `CD2/`, `Disc 1/`, etc.):

### Structure

```
Album Name (Year)/
├── CD1/
│   ├── 01 Artist - Track.wav
│   └── ...
├── CD2/
│   ├── 01 Artist - Track.wav
│   └── ...
└── cover.jpg
```

### Rules

- Track numbers restart at 01 per disc
- Cover art at album root (not in disc folders)
- Set Disc Number tag for each track:
  ```sh
  cd "Album/CD1"
  # For FLAC/MP3:
  kid3-cli -c "select all" -c "tag 2" -c "set disc number 1" -c "save" .

  # For WAV (both tags):
  kid3-cli -c "select all" \
           -c "tag 2" -c "set disc number 1" \
           -c "tag 3" -c "set disc number 1" \
           -c "save" .
  ```
- Album-wide tags (album, year, publisher) go on all tracks across all discs

---

## Phase 5: Final Verification

### Directory structure check

```sh
# List organized structure
find "DEST" -type d | head -30

# Count audio files
find "DEST" -type f \( -name "*.flac" -o -name "*.wav" -o -name "*.mp3" \) | wc -l
```

### Summary report

```
═══════════════════════════════════════════════
Batch Import Preparation Complete
═══════════════════════════════════════════════

Albums processed: X
├── Single Artist: N
└── Various Artists: M

Loose tracks processed: Y

Directory structure:
├── Artist A/
│   └── Album (Year)/ [N tracks]
├── Artist B/
│   └── Album (Year)/ [N tracks]
├── Various Artists/
│   └── Label/
│       └── Album (Year)/ [N tracks]
└── [Y loose tracks at root]

Issues/Notes:
- [any unresolved items]
- [any items needing user attention]

WAV cover art (manual Rekordbox step):
  N WAV files have embedded cover art in tag 2 but
  Rekordbox will not display it. After importing,
  set cover art for these tracks manually in Rekordbox:
  - Artist / Album (Year) — N WAV tracks
  - [etc.]

Next steps:
1. Import into Rekordbox (drag directory into collection)
2. Set cover art for listed WAV tracks in Rekordbox
3. Run collection audit SOP to verify import
4. Run genre classification SOP for genre tagging
═══════════════════════════════════════════════
```

---

## Decision Reference

### Proceed automatically when

- Artist clearly identified in tags, filename, or directory name
- Year present in tags, directory name, or single clear Discogs match
- Label present in tags or Discogs (for VA)
- Filename pattern is parseable
- Single artist with consistent tags across album

### Stop and ask when

- Multiple Discogs/Beatport matches with different years
- No results from either provider and year/label unknown
- VA album but label unknown
- Ambiguous: collaboration vs VA vs single artist
- Multiple images — which is album cover?
- Conflicting metadata between tags and filenames
- Unparseable filenames
- Large file size differences suggesting different versions/qualities

---

## Common Filename Patterns (Incoming)

### Album tracks

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

| Pattern | Status |
|---------|--------|
| `Artist - Title.wav` | Correct |
| `Artist - Title (Remix Info).wav` | Correct |
| `Artist, Artist B - Title.wav` | Correct |
| `Artist - Title (Original Mix).wav` | Remove "(Original Mix)" |
| `Title.wav` | Missing artist — ask user |

---

## Quality Checklist

### Albums
- [ ] Directory: `Artist/Album (Year)/` — no tech specs, year present
- [ ] Files: `NN Artist - Title.ext` — zero-padded track numbers
- [ ] Tags: Artist, Title, Track, Album, Year — all present
- [ ] WAV files: Tags in both tag 2 (ID3v2) AND tag 3 (RIFF INFO)
- [ ] Genre: NOT set
- [ ] VA albums: Label subdirectory, each track has correct per-track Artist
- [ ] Cover art: Embedded if available
- [ ] Old source directory removed

### Loose Tracks
- [ ] Named: `Artist Name - Track Title.ext`
- [ ] No "(Original Mix)" suffix
- [ ] Tags: Artist, Title minimum
- [ ] WAV files: Tags in both tag 2 AND tag 3
- [ ] Genre: NOT set
- [ ] Cover art: Embedded if available
- [ ] Location: Root of batch directory
