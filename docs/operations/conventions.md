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

| Pattern                   | Example                            | When acceptable                          |
| ------------------------- | ---------------------------------- | ---------------------------------------- |
| `NN. Title.ext`           | `08. Tune Out.flac`                | Single-artist album, if tags have artist |
| `NN - Title.ext`          | `05 - Invisible Dance.flac`        | Single-artist album, if tags have artist |
| `D-NN Artist - Title.ext` | `2-04 Joey Negro - The Dells.flac` | Multi-disc albums                        |

### Filename parsing rules

**Album tracks** (`NN Artist Name - Track Title.ext`):

1. Strip file extension
2. First 2 chars = track number (zero-padded)
3. Char 3 = space (skip)
4. Char 4 to first ` - ` = artist
5. After first ` - ` = title

**Edge cases:**
- Title contains `-` (e.g., "Artist - Track - Subtitle") — split on first ` - ` only, unless artist is known from directory context.
- VA compilations — filename artist is the per-track artist, not the AlbumArtist.

**Loose tracks** (`Artist Name - Track Title.ext`): split on first ` - `.

## Tags

### Rekordbox import readiness

What Rekordbox reads on import by format:

| Field     | FLAC            | WAV               | MP3             |
| --------- | --------------- | ----------------- | --------------- |
| Artist    | Vorbis Comment  | RIFF INFO (tag 3) | ID3v2 (tag 2)   |
| Title     | Vorbis Comment  | RIFF INFO (tag 3) | ID3v2 (tag 2)   |
| Album     | Vorbis Comment  | RIFF INFO (tag 3) | ID3v2 (tag 2)   |
| Year      | Vorbis Comment  | RIFF INFO (tag 3) | ID3v2 (tag 2)   |
| Track     | Vorbis Comment  | RIFF INFO (tag 3) | ID3v2 (tag 2)   |
| Cover art | Embedded (auto) | **Not imported**  | Embedded (auto) |

**WAV critical:** Rekordbox reads **only** RIFF INFO (tag 3) from WAV files. ID3v2 (tag 2) is ignored. Both must be written — tag 2 for general compatibility, tag 3 for Rekordbox.

**WAV cover art:** Rekordbox cannot import cover art from WAV files. Embed in tag 2 for other apps; WAV tracks need manual cover art in Rekordbox after import.

### Required tags

| Tag       | Album tracks | Loose tracks | Format                              |
| --------- | ------------ | ------------ | ----------------------------------- |
| Artist    | Required     | Required     | Track-level artist                  |
| Title     | Required     | Required     | Track title only (no artist prefix) |
| Track     | Required     | Not needed   | Integer (not zero-padded)           |
| Album     | Required     | Optional     | Album name                          |
| Date/Year | Required     | Optional     | YYYY                                |

**FLAC year field:** FLAC uses Vorbis Comment `Date` not `Year`. A file has a year if either field is populated.

### Recommended tags

| Tag                    | When required                        |
| ---------------------- | ------------------------------------ |
| Publisher/Organization | Always for VA, recommended for all   |
| AlbumArtist            | Required for VA, recommended for all |
| Disc                   | Required for multi-disc albums       |

### Genre policy

Genre tags are left blank in files. Genre is managed exclusively through Rekordbox via the [genre classification SOP](sops/genre-classification.md).

**Pre-existing genre tags:** If genre tags are already set from source downloads (Bandcamp, Juno, etc.), flag for user review with three options: **(a)** clear the tag, **(b)** keep as-is and document the exception, **(c)** migrate the value to comments before clearing. Do not auto-clear without user approval.
