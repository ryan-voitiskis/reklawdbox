# Collection Conventions

Default conventions for organizing and tagging a music collection. SOPs reference this document instead of embedding conventions inline.

These are starting-point defaults — override per-session by telling the agent your preferences.

## Directory Structure

### Single-artist albums

```
collection/
└── Artist Name/
    └── Album Name (Year)/
        ├── 01 Artist Name - Track Title.flac
        └── cover.jpg
```

### Various Artists compilations

```
collection/
└── Various Artists/
    └── Label Name/
        └── Album Name (Year)/
            ├── 01 Artist A - Track Title.flac
            └── cover.jpg
```

### Multi-disc albums

Disc subdirectories under the album directory. Track numbers restart at 01 per disc.

```
Album Name (Year)/
├── CD1/
│   ├── 01 Artist - Track.flac
│   └── 02 Artist - Track.flac
└── CD2/
    ├── 01 Artist - Track.flac
    └── 02 Artist - Track.flac
```

### Loose tracks (play directories)

```
play/
├── Artist Name - Track Title.wav
└── Artist Name - Track Title.flac
```

### Album type classification

<!-- dprint-ignore -->
| Pattern                        | Type           | Directory structure                   |
| ------------------------------ | -------------- | ------------------------------------- |
| All tracks same artist         | Single Artist  | `Artist/Album (Year)/`                |
| Different artists per track    | VA Compilation | `Various Artists/Label/Album (Year)/` |
| Multiple named artists (A & B) | Collaboration  | `Various Artists/Label/Album (Year)/` |

## Album Directory Naming

- Format: `Album Name (Year)/`
- No artist name in the album directory
- Remove: `[FLAC]`, `[WAV]`, `[MP3]`, `24-96`, `16-44`, `24bit`, usernames (e.g., `PBTHAL`, `vtwin88cube`), catalog numbers
- Preserve: `(Deluxe Edition)`, `Vol. 1`, `(Remastered)`
- Replace `/` with `-`, `:` with `-`

## File Naming

### Canonical format

- **Album tracks:** `NN Artist Name - Track Title.ext` (zero-padded track number, space-dash-space separator)
- **Loose tracks:** `Artist Name - Track Title.ext` (no track number)

### Acceptable alternates

These patterns are valid and should not be flagged as errors during audits:

<!-- dprint-ignore -->
| Pattern                   | Example                            | When acceptable                          |
| ------------------------- | ---------------------------------- | ---------------------------------------- |
| `NN. Title.ext`           | `08. Tune Out.flac`                | Single-artist album, if tags have artist |
| `NN - Title.ext`          | `05 - Invisible Dance.flac`        | Single-artist album, if tags have artist |
| `D-NN Artist - Title.ext` | `2-04 Joey Negro - The Dells.flac` | Multi-disc albums                        |

### Filename parsing rules

The delimiter is always `-` (space-dash-space). Bare hyphens inside artist or title names (e.g., "Jean-Michel Jarre") are not delimiters.

**Album tracks** (`NN Artist Name - Track Title.ext`):

1. Strip file extension
2. Leading digits = track number
3. Skip whitespace after track number
4. Split remainder on first `-` — left side is artist, right side is title

**Edge cases:**

- Title contains `-` (e.g., `Artist - Track - Subtitle`) — split on first `-` only, unless artist is known from directory context.
- VA compilations — filename artist is the per-track artist, not the AlbumArtist.

**Loose tracks** (`Artist Name - Track Title.ext`): split on first `-`.

## Tags

### Rekordbox import readiness

What Rekordbox reads on import by format:

FLAC, MP3, AIFF, and M4A all have full tag support — every field including Label and embedded cover art is read on import. They use different tag containers (Vorbis Comment for FLAC, ID3v2 for MP3 and AIFF, MP4 ilst for M4A) but the field coverage is identical.

WAV is the exception. Rekordbox reads **only** RIFF INFO (tag 3) from WAV files, which has a limited field set:

<!-- dprint-ignore -->
| Field     | WAV support                |
| --------- | -------------------------- |
| Artist    | RIFF INFO (tag 3)          |
| Title     | RIFF INFO (tag 3)          |
| Album     | RIFF INFO (tag 3)          |
| Genre     | RIFF INFO (tag 3)          |
| Year      | RIFF INFO (tag 3)          |
| Comment   | RIFF INFO (tag 3)          |
| Track     | **Not supported**          |
| Label     | **Not supported**          |
| Cover art | **Not imported**           |

**WAV dual-tag rule:** Rekordbox ignores ID3v2 (tag 2) in WAV files. Both must be written — tag 2 for general compatibility, tag 3 for Rekordbox. `write_file_tags` handles this automatically.

**WAV cover art:** Rekordbox cannot import cover art from WAV files. Embed in tag 2 for other apps; WAV tracks need manual cover art in Rekordbox after import.

**WAV label and track number:** RIFF INFO has no publisher or track number fields. For WAV files, these are DB-only — set them via XML import or manually in Rekordbox.

### Reload Tag workflow

Rekordbox's **Reload Tag** (right-click → Reload Tag) re-reads file tags and updates the library DB. This provides a faster alternative to XML import for fields that Rekordbox reads from file tags.

**WAV Reload Tag field support** (RIFF INFO only):

<!-- dprint-ignore -->
| Field   | Reload works? | Notes                                   |
| ------- | ------------- | --------------------------------------- |
| Artist  | Yes           |                                         |
| Title   | Yes           |                                         |
| Album   | Yes           |                                         |
| Genre   | Yes           | Verified 2026-03-03                     |
| Year    | Yes           |                                         |
| Comment | Yes           |                                         |
| Track   | **No**        | RIFF INFO has no track number field     |
| Label   | **No**        | RIFF INFO has no publisher field        |
| BPM     | **No**        | Not in RIFF INFO; Rekordbox uses its own analysis |
| Key     | **No**        | Not in RIFF INFO; Rekordbox uses its own analysis |

**Workflow:** `write_file_tags` → select tracks in Rekordbox → Reload Tag.

After reloading, any previously edited track information is replaced with the reloaded values.

### Required tags

<!-- dprint-ignore -->
| Tag       | Album tracks | Loose tracks | Format                              |
| --------- | ------------ | ------------ | ----------------------------------- |
| Artist    | Required     | Required     | Track-level artist                  |
| Title     | Required     | Required     | Track title only (no artist prefix) |
| Track     | Required     | Not needed   | Integer (not zero-padded)           |
| Album     | Required     | Optional     | Album name                          |
| Date/Year | Required     | Optional     | YYYY                                |

**FLAC year field:** FLAC uses Vorbis Comment `Date` not `Year`. A file has a year if either field is populated.

### Recommended tags

<!-- dprint-ignore -->
| Tag                    | When required                        |
| ---------------------- | ------------------------------------ |
| Publisher/Organization | Always for VA, recommended for all   |
| AlbumArtist            | Required for VA, recommended for all |
| Disc                   | Required for multi-disc albums       |

### Genre policy

Genre is written to file tags via `write_file_tags` and loaded into Rekordbox via Reload Tag.

**Pre-existing genre tags:** If genre tags are already set from source downloads (Bandcamp, Juno, etc.), flag for user review with three options: **(a)** clear the tag, **(b)** keep as-is and document the exception, **(c)** migrate the value to comments before clearing. Do not auto-clear without user approval.
