# Collection Audit — Agent SOP

Detect and fix naming/tagging convention violations in a music collection. Follow step-by-step.

**Goal:** After this SOP, any track file can be imported into Rekordbox with Artist, Title, Album, Year, Track Number, and Label displaying correctly. Genre is handled separately via the [genre classification SOP](genre-classification.md).

**Conventions:** All directory structure, filename, and tag conventions are defined in [conventions.md](../conventions.md). This SOP references those conventions but does not redefine them.

## Constraints

- **Read-only by default.** No files modified until user approves a fix plan.
- **Stop on ambiguity.** If the correct fix isn't clear, flag for user review — never guess.
- **Process in small batches.** One artist or album at a time. Report progress after each.
- **Verify after fixing.** Re-read tags/filenames after every fix.
- **Never delete audio files.** Renaming and tag editing only.
- **WAV dual-tag rule.** Always write both tag 2 and tag 3 for WAV files.
- **Rekordbox path awareness.** Before any rename or move, check if the file is imported in Rekordbox via `search_tracks(path="...")`. If imported, warn the user that renaming will break Rekordbox references. See [file relocation staging spec](../../spec/file-relocation-staging.md).

## Prerequisites

| Tool             | Purpose                            | Install                 | Required? |
| ---------------- | ---------------------------------- | ----------------------- | --------- |
| `reklawdbox` MCP | DB queries, metadata lookups       | This project            | Yes       |
| `kid3-cli`       | File-level tag writing, renaming   | `brew install kid3`     | For fixes |
| `exiftool`       | WAV tag 3 detection, file tag read | `brew install exiftool` | For fixes |

**DB-first principle:** Use reklawdbox MCP tools for all metadata queries on imported tracks. Only fall back to exiftool/kid3-cli for file-level operations the DB can't do (WAV tag 3 detection, file-tag drift, un-imported files). See [DB-driven tag audit spec](../../spec/db-tag-audit.md).

**Future:** exiftool and kid3-cli will be replaced by native Rust tag tools. See [native tag tools spec](../../spec/native-tag-tools.md).

**Tool availability check:**

```sh
command -v exiftool >/dev/null || echo "Missing exiftool — file-level audit will be limited"
kid3-cli --help >/dev/null 2>&1 || echo "Missing kid3-cli — tag writes unavailable"
```

**Shell compatibility:** All shell snippets require `/bin/bash`. Use full paths (`/usr/bin/find`, `/usr/bin/grep`) to avoid alias conflicts with `fd`/`rg`.

---

## Step 0: Scope Selection

Ask the user what to audit:

1. Entire collection directory
2. Specific artist directory
3. Specific album
4. Specific play subdirectory
5. Custom path

Confirm conventions or ask for overrides. Then assess scope size:

```
search_tracks(path="/path/to/scope", limit=1)
```

Check the total count. If >500 tracks, recommend processing in artist-level batches. Prioritize by severity:

1. **WAV tag 3 fixes** — Rekordbox import blocking
2. **Empty/missing required tags** — tracks can't be identified
3. **Filename normalization** — mismatches and tech-spec cleanup
4. **Missing recommended tags** — non-blocking but improves library

---

## Step 1: DB-Level Tag Audit

**Goal:** Use Rekordbox's DB to detect missing, empty, or inconsistent metadata for imported tracks. No file I/O.

### 1a: Bulk metadata query

```
resolve_tracks_data(path="/path/to/scope", max_tracks=200)
```

For large scopes, page through with `search_tracks` + offset.

### 1b: Issue detection from DB

Check each track's DB metadata against [conventions](../conventions.md):

| Issue                    | Detection                                           | Auto-fixable?                               |
| ------------------------ | --------------------------------------------------- | ------------------------------------------- |
| **Empty artist**         | Artist field empty/missing in DB                    | Yes — parse from filename                   |
| **Artist-in-title**      | Title contains `Artist - Title` pattern             | Yes — strip artist prefix                   |
| **Missing required tag** | Any of Artist/Title/Track/Album/Year empty in DB    | Sometimes — filename, directory, or Discogs |
| **Genre tag set**        | Genre not empty in DB                               | Flag for user review                        |
| **Filename/tag mismatch**| DB Location filename disagrees with DB tag values   | No — flag for user review                   |

### 1c: Metadata lookups for gaps

When tags are missing and can't be derived from filenames, use MCP lookups:

```
lookup_discogs(track_id="...")
lookup_beatport(track_id="...")
```

Use results to fill Album, Year, and Publisher. Never auto-set Genre from lookups.

---

## Step 2: File-Level Audit

**Goal:** Detect issues the DB can't see. Only needed for specific cases.

### 2a: WAV tag 3 detection

For WAV files in scope, check if RIFF INFO tags exist:

```sh
exiftool -s3 -RIFF:Artist "/path/to/file.wav"
```

Empty result means tag 3 is missing. Batch this across all WAV files in scope — only use `kid3-cli` for the actual writes.

### 2b: Un-imported file audit

For files not in the Rekordbox DB (new acquisitions, staging area), fall back to exiftool for tag reads:

```sh
/usr/bin/find "/path/to/scope" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -exec exiftool -j -Artist -Title -Album -Year -Date -TrackNumber -Publisher -AlbumArtist {} + 2>/dev/null
```

### 2c: No-tag file inference

Files with no tags at all (common with 90s-era rips) should not be skipped. The agent should infer metadata from:
- Parent directory name (artist, album, year)
- Companion files (cover.jpg, .nfo, .cue)
- Filename patterns

Flag inferred metadata for user confirmation before writing.

---

## Step 3: Filename Scan

**Goal:** Detect files whose names don't match conventions. Read-only.

### 3a: Album track filename scan

For each album directory, check audio files against canonical format and [acceptable alternates](../conventions.md#acceptable-alternates):

```sh
/usr/bin/find "/path/to/album" -maxdepth 1 -type f \
    \( -iname "*.flac" -o -iname "*.wav" -o -iname "*.mp3" -o -iname "*.m4a" \) \
    -print0 | while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [[ "$name" =~ ^[0-9][0-9]\ .+\ -\ .+\..+$ ]]; then
        : # Canonical — OK
    elif [[ "$name" =~ ^[0-9][0-9]\.\ .+\..+$ ]]; then
        echo "INFO_ALT_FORMAT: $f  (NN. Title — check tags have artist)"
    elif [[ "$name" =~ ^[0-9][0-9]\ -\ .+\..+$ ]]; then
        echo "INFO_ALT_FORMAT: $f  (NN - Title — check tags have artist)"
    elif [[ "$name" =~ ^[0-9]-[0-9][0-9]\ .+\ -\ .+\..+$ ]]; then
        echo "INFO_ALT_FORMAT: $f  (D-NN Artist - Title — multi-disc)"
    else
        echo "MISMATCH: $f"
    fi
done
```

### 3b: Directory naming scan

Check for tech specs in directory names and missing years:

```sh
/usr/bin/find "/path/to/scope" -type d -print0 | while IFS= read -r -d '' dir; do
    name=$(basename "$dir")
    if [[ "$name" =~ \[FLAC\]|\[WAV\]|\[MP3\]|24-96|16-44|24bit|PBTHAL|vtwin88cube ]]; then
        echo "TECH_SPECS: $dir"
    fi
done
```

### 3c: Loose track naming scan

For play directories, check loose tracks match `Artist - Title.ext`:

```sh
/usr/bin/find "/path/to/play" -maxdepth 1 -type f \
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

## Step 4: Issue Report

Present findings to the user in three categories:

1. **Auto-fixable** — unambiguous fixes, present as batch for single approval
2. **Needs per-item approval** — multiple valid fixes exist, present options for each
3. **Needs investigation** — user must inspect and decide

Include a summary with total files scanned, pass rate, and counts per issue type.

**Future:** Audit state will be persisted for idempotent re-runs. See [audit idempotency spec](../../spec/audit-idempotency.md).

---

## Step 5: Fix Execution

### 5a: Auto-fixes (after batch approval)

Process one album at a time.

#### Fix empty artist / artist-in-title

Parse artist and title from filename, write to tags:

```sh
escape_kid3() {
    printf "%s" "$1" | sed "s/'/\\\\'/g"
}

/usr/bin/find "/path/to/album" -maxdepth 1 -type f \
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

    case "${filename##*.}" in
        [Ww][Aa][Vv])
            kid3-cli -c "tag 2" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set tracknumber $track_int" \
                     -c "tag 3" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set tracknumber $track_int" \
                     -c "save" "$path"
            ;;
        [Mm]4[Aa])
            exiftool -overwrite_original \
                -Artist="$artist" -Title="$title" -TrackNumber="$track_int" \
                "$path" >/dev/null
            ;;
        *)
            kid3-cli -c "tag 2" \
                     -c "set artist '$artist_esc'" \
                     -c "set title '$title_esc'" \
                     -c "set tracknumber $track_int" \
                     -c "save" "$path"
            ;;
    esac
done
```

#### Fix WAV tag 3 missing

Read tag 2 values, write to tag 3:

```sh
artist=$(exiftool -s3 -Artist "$path")
title=$(exiftool -s3 -Title "$path")
album=$(exiftool -s3 -Album "$path")
year=$(exiftool -s3 -Year "$path")
track=$(exiftool -s3 -TrackNumber "$path" | sed 's#/.*##')

kid3-cli -c "tag 3" \
         -c "set artist '$(printf "%s" "$artist" | sed "s/'/\\\\'/g")'" \
         -c "set title '$(printf "%s" "$title" | sed "s/'/\\\\'/g")'" \
         -c "set album '$(printf "%s" "$album" | sed "s/'/\\\\'/g")'" \
         -c "set date $year" \
         -c "set tracknumber $track" \
         -c "save" "$path"
```

#### Fix (Original Mix) in filenames

Check Rekordbox import status before renaming:

```
search_tracks(path="filename")  # If imported, warn user
```

```sh
base=$(basename "$f")
new_base=$(printf "%s" "$base" | sed 's/ (Original Mix)//')
mv -- "$f" "$(dirname "$f")/$new_base"
```

### 5b: Per-item fixes

Present each ambiguous issue to the user with options and wait for approval before applying.

### 5c: Investigation items

Present context (filename vs tag values, DB vs file state) and ask the user to decide.

---

## Step 6: Verification

After fixing an album or batch, verify via DB and file reads:

```
resolve_tracks_data(path="/path/to/album")  # Check DB reflects fixes
```

For WAV tag 3 fixes, verify at the file level:

```sh
exiftool -s3 -RIFF:Artist "/path/to/file.wav"
```

Re-run the filename scan from Step 3 on the fixed scope to confirm no remaining mismatches.

---

## Step 7: Final Report

Summarize: scope, files scanned (DB vs file-level), pass rate, fixes applied by type, remaining issues, and next steps (manual review items, Rekordbox import, WAV cover art, genre classification SOP).

---

## Appendix A: Issue Detection Quick Reference

| Issue                    | Source | Detection                              | Auto-fixable?                | Fix method                      |
| ------------------------ | ------ | -------------------------------------- | ---------------------------- | ------------------------------- |
| Empty artist tag         | DB     | Artist empty in resolve_tracks_data    | Yes (if filename has artist) | Parse from filename             |
| Artist-in-title          | DB     | Title contains `Artist - Title`        | Yes                          | Strip artist prefix             |
| Missing Track number     | DB     | TrackNumber empty                      | Yes (if filename has NN)     | Parse from filename             |
| Missing Album tag        | DB     | Album empty                            | Sometimes (from directory)   | Directory name or Discogs       |
| Missing Year tag         | DB     | Year AND Date both empty               | Sometimes                    | Discogs lookup                  |
| WAV tag 3 missing        | File   | exiftool -RIFF:Artist empty            | Yes                          | Copy from tag 2                 |
| Genre set                | DB     | Genre not empty                        | Flag only                    | User decides keep/clear/migrate |
| Filename mismatch        | DB     | Location path vs tag values disagree   | No — ambiguous               | User review                     |
| Tech specs in dir name   | File   | Regex match `[FLAC]` etc.             | Yes                          | Strip from dir name             |
| Missing year in dir name | File   | No `(YYYY)` in album dir              | Sometimes                    | Discogs lookup                  |
| (Original Mix) suffix    | File   | Filename contains it                   | Yes                          | Rename (check import status)    |
| No tags (90s rips)       | File   | All tag fields empty                   | Infer from context           | Directory + filename inference  |

## Appendix B: kid3-cli Patterns

**Quoting:** Prefer passing the file as a positional argument instead of using `select`:

```sh
# SAFER — pass file path as positional argument:
kid3-cli -c "set artist 'Name' 2" -c "set artist 'Name' 3" "/path/to/file.wav"
```

### WAV dual-tag write

```sh
kid3-cli -c "tag 2" \
         -c "set artist 'Name'" \
         -c "set title 'Title'" \
         -c "tag 3" \
         -c "set artist 'Name'" \
         -c "set title 'Title'" \
         -c "save" "/path/to/file.wav"
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
