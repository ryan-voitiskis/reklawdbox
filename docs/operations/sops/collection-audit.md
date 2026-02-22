# Collection Audit — Agent SOP

Standard operating procedure for detecting and fixing convention violations in an organized music collection. Agents must follow this document step-by-step.

## Overview

This SOP systematically audits audio files in a music collection against naming and tagging conventions, then fixes violations with appropriate safety levels. It uses CLI tools for file operations and reklawdbox MCP for metadata lookups.

**Primary use case:** A user has an organized collection (or partially organized) and wants to verify everything conforms to their conventions before importing into Rekordbox.

**Goal:** After running this SOP, any track file can be dragged into Rekordbox and have Artist, Title, Album, Year, Track Number, and Label display correctly without manual editing. Genre is handled separately via the genre classification SOP.

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

**WAV cover art limitation:** Rekordbox does not read embedded cover art from WAV files. RIFF INFO has no standard picture field, and Rekordbox ignores ID3v2 picture tags in WAVs. Cover art for WAV tracks must be added manually in Rekordbox after import (drag image onto track artwork area). The SOP still embeds cover art into WAV tag 2 for compatibility with other applications, but documents this limitation to the user.

**Critical:** WAV files **must** have RIFF INFO (tag 3) tags written. Rekordbox ignores ID3v2 tags in WAV files. If only tag 2 is written, Rekordbox will show blank Artist/Title/Album fields.

## Constraints

- **Read-only by default.** Every scan phase is non-destructive. No files are modified until the user approves a fix plan.
- **Batch approval.** Auto-fixable issues are grouped and presented for batch approval. Ambiguous issues require per-item approval.
- **Stop on ambiguity.** If the agent cannot determine the correct fix with high confidence, flag it for user review — never guess.
- **Process in small batches.** Work through one artist or album at a time. Report progress after each.
- **Verify after fixing.** Re-read tags/filenames after every fix to confirm correctness.
- **Never delete audio files.** Renaming and tag editing only. File deletion requires explicit user instruction.
- **WAV dual-tag rule.** WAV files must have tags in both tag 2 (ID3v2) and tag 3 (RIFF INFO). Always write both when fixing WAV tags.

## Prerequisites

### Required tools

| Tool | Purpose | Install |
|------|---------|---------|
| `kid3-cli` | Tag reading/writing, file renaming | `brew install kid3` |
| `exiftool` | Batch tag reading as JSON | `brew install exiftool` |
| `reklawdbox` MCP | Discogs/Beatport metadata lookups | This project |

### Optional tools

| Tool | Purpose |
|------|---------|
| `check_filenames.sh` | Bulk filename convention scan (if available in user's scripts/) |
| `metaflac` | FLAC-specific tag operations |
| `ffprobe` | Quick format inspection |

---

## Convention Reference

These are the default conventions this SOP validates against. Users should confirm or customize these before starting.

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

### Required tags (all files)

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

### What must NOT be set

| Tag | Reason |
|-----|--------|
| Genre | Left blank for manual Rekordbox curation via reklawdbox |

### WAV-specific

WAV files require tags in both:
- **tag 2** (ID3v2.3.0) — general compatibility
- **tag 3** (RIFF INFO) — Rekordbox reads this

---

## Step 0: Scope Selection

**Goal:** Define what to audit and confirm conventions with the user.

### Present to user

```
Collection Audit — Scope Selection

What would you like to audit?
1. Entire collection (collection/)
2. Entire play directory (play/)
3. Specific artist directory
4. Specific album
5. Specific play subdirectory (e.g., play/play29/)
6. Custom path

Conventions:
- Filenames: NN Artist - Title.ext
- Required tags: Artist, Title, Track, Album, Year
- Genre: NOT set (manual Rekordbox curation)
- WAV: Both tag 2 and tag 3

Confirm conventions or specify overrides?
```

### Estimate scale

After user selects scope:

```sh
# Count audio files in scope
find "TARGET_PATH" -type f \( -name "*.flac" -o -name "*.wav" -o -name "*.mp3" -o -name "*.m4a" \) | wc -l
```

If file count exceeds 500, recommend processing in artist-level batches:

```
Found ~2400 audio files in collection/.
Recommend processing artist-by-artist to keep batches manageable.
Start with artists A-C? Or specify a starting artist?
```

---

## Step 1: Filename Scan

**Goal:** Detect files whose names don't match the `NN Artist - Title.ext` convention.

### 1a: Run filename check (if check_filenames.sh available)

```sh
check_filenames.sh -rv "TARGET_PATH"
```

This reports albums with incorrect filenames and a summary count.

### 1b: Manual filename scan (if script not available)

For each album directory, check files against the pattern:

```sh
# List all audio files and check pattern
cd "ALBUM_PATH"
for f in *.flac *.wav *.mp3 *.m4a; do
    [ -f "$f" ] || continue
    if [[ ! "$f" =~ ^[0-9][0-9]\ .+\ -\ .+\..+$ ]]; then
        echo "MISMATCH: $f"
    fi
done
```

### 1c: Directory naming scan

For each album directory, check for convention violations:

```sh
# Check for tech specs in album directory names
find "TARGET_PATH" -type d | while read -r dir; do
    name=$(basename "$dir")
    # Flag directories with common tech spec patterns
    if [[ "$name" =~ \[FLAC\]|\[WAV\]|\[MP3\]|24-96|16-44|24bit|PBTHAL|vtwin88cube ]]; then
        echo "TECH_SPECS: $dir"
    fi
    # Flag album directories missing year
    # (only for dirs that contain audio files directly)
    if ls "$dir"/*.{flac,wav,mp3} >/dev/null 2>&1; then
        if [[ ! "$name" =~ \([0-9]{4}\) ]]; then
            echo "NO_YEAR: $dir"
        fi
    fi
done
```

### 1d: Loose track naming scan

For play directories, check loose tracks at root:

```sh
# Check for (Original Mix) in filenames
ls "PLAY_PATH"/*"(Original Mix)"* 2>/dev/null && echo "HAS_ORIGINAL_MIX"

# Check for tracks missing artist-title separator
for f in "PLAY_PATH"/*.{flac,wav,mp3}; do
    [ -f "$f" ] || continue
    name=$(basename "$f")
    # Loose tracks should match: Artist - Title.ext
    if [[ ! "$name" =~ ^.+\ -\ .+\..+$ ]]; then
        echo "BAD_LOOSE: $name"
    fi
done
```

### Record findings

Track all filename issues with their paths. Do not fix anything yet — this is read-only.

---

## Step 2: Tag Audit

**Goal:** Read tags from audio files and detect missing, empty, or inconsistent metadata.

### 2a: Batch tag read

For each album directory:

```sh
cd "ALBUM_PATH"
exiftool -j -Artist -Title -Album -Year -TrackNumber -Publisher -AlbumArtist *.flac *.wav *.mp3 2>/dev/null
```

### 2b: Detect tag issues

For each file, check against these patterns. Categorize every issue found.

#### Issue A: Empty artist tag

**Detection:** Artist tag is empty or missing.

```
File: 01 Kangding Ray - Amber Decay.wav
Tags: Artist=(empty), Title="Kangding Ray - Amber Decay"
```

**Auto-fix available:** Parse artist from filename. If filename matches `NN ARTIST - TITLE.ext`, extract artist and title.

#### Issue B: Artist-in-title

**Detection:** Title tag contains a hyphen-separated prefix that matches the expected artist.

```
File: 01 Kangding Ray - Amber Decay.wav
Tags: Artist="Kangding Ray", Title="Kangding Ray - Amber Decay"
```

**Auto-fix available:** Strip artist prefix from title. Title should be `Amber Decay`.

#### Issue C: Missing required tags

**Detection:** Any of Artist, Title, Track, Album, Year is empty.

**Fix depends on source:** May be derivable from filename, directory name, or Discogs lookup.

#### Issue D: WAV missing tag 3 (RIFF INFO)

**Detection:** WAV file has tag 2 but not tag 3.

```sh
kid3-cli -c "tag 3" -c "get" "file.wav"
# If output shows no tags or empty fields → tag 3 missing
```

**Auto-fix available:** Copy tag 2 values to tag 3.

#### Issue E: Genre tag set

**Detection:** Genre tag is not empty.

**Action:** Flag for user review — user may want to keep existing genre or clear it.

#### Issue F: Track number format

**Detection:** Track number is zero-padded in tag (`01` instead of `1`) or missing.

**Auto-fix available:** Parse from filename or fix format.

#### Issue G: Filename/tag mismatch

**Detection:** Filename says one thing, tags say another.

```
File: 03 Artist A - Some Track.flac
Tags: Artist="Artist B", Title="Different Track", Track=5
```

**Action:** Flag for user review — cannot determine which source is correct.

### 2c: Efficient batch processing

For large collections, process one artist directory at a time:

```sh
# Get list of all artist directories
ls -d "collection"/*/ | sort

# For each artist, read all tags at once
for artist_dir in "collection"/*/; do
    artist=$(basename "$artist_dir")
    echo "=== Scanning: $artist ==="

    for album_dir in "$artist_dir"*/; do
        [ -d "$album_dir" ] || continue
        album=$(basename "$album_dir")

        # Read all tags in album
        exiftool -j -Artist -Title -Album -Year -TrackNumber \
            "$album_dir"/*.{flac,wav,mp3,m4a} 2>/dev/null
    done
done
```

### 2d: Use reklawdbox for metadata gaps

When tags are missing and cannot be derived from filenames:

```
lookup_discogs(artist="Kangding Ray", title="Amber Decay")
lookup_beatport(artist="Kangding Ray", title="Amber Decay")
```

Use lookup results to fill Album, Year, and Publisher tags. Never auto-set Genre from lookup results.

---

## Step 3: Issue Report

**Goal:** Present all findings to the user, categorized by fix type.

### Issue categories

#### Auto-fixable (batch approval)

Issues where the correct fix is unambiguous. Present as a batch for single approval.

| # | Issue | File | Current | Proposed Fix |
|---|-------|------|---------|--------------|
| 1 | Empty artist | `01 Kangding Ray - Amber Decay.wav` | Artist=(empty) | Artist="Kangding Ray" |
| 2 | Artist-in-title | `01 Kangding Ray - Amber Decay.wav` | Title="Kangding Ray - Amber Decay" | Title="Amber Decay" |
| 3 | WAV tag 3 missing | `01 Kangding Ray - Amber Decay.wav` | tag 3=(empty) | Copy from tag 2 |

#### Needs approval (per-item)

Issues where multiple valid fixes exist. Present each with options.

```
Issue: Missing year for album "Unknown Album"
  Path: collection/Artist/Unknown Album/
  Options:
    a) Set year to 2020 (from Discogs match)
    b) Set year to 2019 (from alternative Discogs match)
    c) Skip — leave as-is
    d) Other — specify year
```

#### Needs investigation

Issues that require the user to inspect the file or provide information the agent cannot determine.

```
Issue: Filename/tag conflict
  File: 03 Artist A - Track.flac
  Filename says: Track 3, "Artist A - Track"
  Tags say: Track 5, Artist="Artist B", Title="Other Track"
  → Which source is correct?
```

### Summary table

```
Collection Audit Results — [scope]
═══════════════════════════════════════
Total files scanned:        2,400
Files passing all checks:   2,180 (90.8%)
Issues found:                 220

By category:
  Auto-fixable:              180 (approve batch fix?)
  Needs per-item approval:    25
  Needs investigation:        15

By issue type:
  Empty artist tag:           45
  Artist-in-title:            45
  WAV tag 3 missing:          60
  Missing required tags:      30
  Filename mismatch:          15
  Directory naming:            8
  Genre set (clear?):         12
  (Original Mix) in name:      5
═══════════════════════════════════════
```

---

## Step 4: Fix Execution

**Goal:** Apply approved fixes safely, in batches.

### 4a: Auto-fixes (after batch approval)

Process one album at a time. For each album with auto-fixable issues:

#### Fix empty artist / artist-in-title

Parse artist and title from filename, write to tags:

```sh
cd "ALBUM_PATH"

# For each file: parse filename, write corrected tags
# Pattern: "NN Artist Name - Track Title.ext"
for f in *.wav *.flac *.mp3; do
    [ -f "$f" ] || continue
    filename=$(basename "$f")

    # Extract components from filename
    # Strip extension
    base="${filename%.*}"
    # Extract track number (first 2 chars)
    track_num="${base:0:2}"
    # Extract "Artist - Title" (everything after "NN ")
    rest="${base:3}"
    # Split on " - "
    artist="${rest%% - *}"
    title="${rest#* - }"

    # Verify parse looks correct before writing
    echo "  File: $filename"
    echo "  → Artist: $artist"
    echo "  → Title: $title"
    echo "  → Track: $((10#$track_num))"
done
```

After confirming the parse output looks correct, write tags:

```sh
# For FLAC/MP3:
kid3-cli -c "select '$filename'" -c "tag 2" \
         -c "set artist '$artist'" \
         -c "set title '$title'" \
         -c "set track number $((10#$track_num))" \
         -c "save" .

# For WAV (both tag 2 AND tag 3):
kid3-cli -c "select '$filename'" \
         -c "tag 2" \
         -c "set artist '$artist'" \
         -c "set title '$title'" \
         -c "set track number $((10#$track_num))" \
         -c "tag 3" \
         -c "set artist '$artist'" \
         -c "set title '$title'" \
         -c "set track number $((10#$track_num))" \
         -c "save" .
```

#### Fix WAV tag 3 missing

Read existing tag 2 values, write them to tag 3:

```sh
# Read current tag 2 values
kid3-cli -c "get" "file.wav"

# Write same values to tag 3
kid3-cli -c "select 'file.wav'" \
         -c "tag 3" \
         -c "set artist 'ARTIST_FROM_TAG2'" \
         -c "set title 'TITLE_FROM_TAG2'" \
         -c "set album 'ALBUM_FROM_TAG2'" \
         -c "set date YEAR_FROM_TAG2" \
         -c "set track number TRACK_FROM_TAG2" \
         -c "save" .
```

#### Fix (Original Mix) in filenames

```sh
cd "PLAY_PATH"
for f in *" (Original Mix)".*; do
    [ -e "$f" ] && mv "$f" "${f/ (Original Mix)/}"
done
```

#### Fill missing tags from reklawdbox lookups

When Artist and Title are known but Album/Year/Publisher are missing:

```
lookup_discogs(artist="Artist Name", title="Track Title")
```

If a clear match is found, write the additional tags. If multiple matches or no match, flag for user review.

### 4b: Per-item fixes (with individual approval)

Present each issue to the user with proposed fix and wait for approval:

```
Fix 1/25: Missing album year
  Path: collection/Artist/Album Name/
  Discogs match: "Album Name" (2019) on Label Records

  Proposed: Rename directory to "Album Name (2019)"
            Set date=2019, publisher="Label Records" on all tracks

  Apply? [yes / no / skip / other year]
```

### 4c: Investigation items

Present context and ask user to decide:

```
Investigation 1/15: Tag/filename conflict
  File: collection/Artist/Album (2020)/03 Artist - Track.flac

  Filename says: Track 3, "Track"
  Tag says: Track 5, Title="Different Track"

  What should this be?
  a) Trust filename (set tags to match)
  b) Trust tags (rename file to match)
  c) Neither — specify correct values
  d) Skip for now
```

---

## Step 5: Verification

**Goal:** Confirm all fixes were applied correctly.

### 5a: Re-scan fixed files

After completing fixes for an album or batch:

```sh
# Re-read tags
exiftool -j -Artist -Title -Album -Year -TrackNumber "$ALBUM_PATH"/*.{flac,wav,mp3} 2>/dev/null

# For WAV: also verify tag 3
for f in "$ALBUM_PATH"/*.wav; do
    [ -f "$f" ] || continue
    echo "=== $(basename "$f") tag 3 ==="
    kid3-cli -c "tag 3" -c "get" "$f"
done
```

### 5b: Re-run filename check

```sh
check_filenames.sh -v "ALBUM_PATH"
```

### 5c: Report per-album

```
✓ Fixed: Kangding Ray / Ultrachroma (2022)
  Files: 10
  Fixes applied:
    - Artist tag written (was empty): 10 files
    - Title tag cleaned (artist prefix removed): 10 files
    - WAV tag 3 synced: 10 files
  Verified: All tags correct, all filenames correct
```

---

## Step 6: Final Report

**Goal:** Summarize the entire audit session.

```
═══════════════════════════════════════════════
Collection Audit Complete
═══════════════════════════════════════════════

Scope: collection/ (full scan)
Files scanned: 2,400
Time: ~45 minutes

Results:
  Already correct:     2,180 (90.8%)
  Auto-fixed:            180
  Fixed with approval:    22
  Skipped by user:         3
  Still needs attention:  15

Fixes by type:
  Artist tags written:     45
  Titles cleaned:          45
  WAV tag 3 synced:        60
  Missing tags filled:     30
  Filenames corrected:     15
  Directories renamed:      7

Remaining issues:
  - collection/Unknown Artist/Mystery Album/
    → 8 files with no identifiable metadata
  - collection/Various Artists/[no label]/
    → 7 files need label identification

WAV cover art (manual Rekordbox step):
  45 WAV files have embedded cover art in tag 2 but
  Rekordbox will not import it. After importing these
  tracks, set cover art manually in Rekordbox:
  - Kangding Ray / Ultrachroma (2022) — 10 WAV tracks
  - System 7 / 777 (1993) — 10 WAV tracks
  - [etc.]

Next steps:
  1. Manually review remaining 15 issues
  2. Import/reimport fixed tracks into Rekordbox
  3. Set cover art for WAV tracks manually in Rekordbox
  4. Run genre classification SOP
═══════════════════════════════════════════════
```

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

```
Input:  "01 Kangding Ray - Amber Decay.wav"
Output: track=1, artist="Kangding Ray", title="Amber Decay"
```

Parsing steps:
1. Strip file extension
2. First 2 characters = track number (zero-padded)
3. Character 3 = space (skip)
4. Everything from character 4 to first ` - ` = artist
5. Everything after first ` - ` = title

**Edge case:** Title contains ` - ` (e.g., "Artist - Track - Subtitle"). Use first occurrence only for the artist/title split, unless the artist name is known from the directory structure.

**Edge case:** VA compilations. Each track may have a different artist. The filename artist should match the per-track artist tag, not the AlbumArtist.

### Loose tracks

Pattern: `Artist Name - Track Title.ext` (no track number prefix)

```
Input:  "Kangding Ray - Amber Decay.wav"
Output: artist="Kangding Ray", title="Amber Decay"
```

## Appendix C: Safe kid3-cli Patterns

### Read before write

Always read current tags before modifying:

```sh
# Read current state
kid3-cli -c "get" "file.wav"

# Only then write
kid3-cli -c "select 'file.wav'" -c "tag 2" -c "set artist 'Name'" -c "save" .
```

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
cd "album_directory"
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
