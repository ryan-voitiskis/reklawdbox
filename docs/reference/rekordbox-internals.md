# Rekordbox Internals: Knowledge Corpus

> Reference document for reklawdbox development. Covers Rekordbox file structure,
> database encryption, XML format, analysis files, and ecosystem tools.
> Last updated: 2026-02-15

---

## Table of Contents

1. [File Locations (macOS)](#1-file-locations-macos)
2. [The Encrypted Database (master.db)](#2-the-encrypted-database-masterdb)
3. [Database Schema](#3-database-schema)
4. [XML Export Format](#4-xml-export-format)
5. [XML Import/Export Workflow](#5-xml-importexport-workflow)
6. [Analysis Files (ANLZ)](#6-analysis-files-anlz)
7. [Metadata Fields Reference](#7-metadata-fields-reference)
8. [pyrekordbox Reference](#8-pyrekordbox-reference)
9. [Ecosystem Tools](#9-ecosystem-tools)
10. [Gotchas and Footguns](#10-gotchas-and-footguns)
11. [Architecture Decision: XML vs DB](#11-architecture-decision-xml-vs-db)

---

## 1. File Locations (macOS)

### Primary Data Directory

```
~/Library/Pioneer/rekordbox/
```

| File | Size (typical) | Description |
|------|---------------|-------------|
| `master.db` | 25 MB | **Main library database** (SQLCipher encrypted) |
| `master.db-shm` | 33 KB | SQLite shared memory file |
| `master.db-wal` | 4.3 MB | SQLite write-ahead log |
| `master.backup.db` | 22 MB | Rekordbox's own backup (auto-created) |
| `master.backup2.db` | 22 MB | Secondary backup |
| `master.backup3.db` | 22 MB | Tertiary backup |
| `networkAnalyze6.db` | 815 KB | Analysis tracking DB (unencrypted SQLite) |
| `networkRecommend.db` | 352 KB | Recommendation tracking DB (unencrypted SQLite) |
| `product.db` | 25 KB | Product/licensing data (encrypted) |
| `datafile.edb` | 2.9 KB | Legacy RB5 data store |
| `datafile.backup.edb` | 2.9 KB | Legacy RB5 backup |
| `ExtData.edb` | 3.0 KB | Extended data store |
| `ExtData.backup.edb` | 3.0 KB | Extended data backup |
| `masterPlaylists6.xml` | 3.6 KB | Playlist structure index (RB6+) |
| `masterPlaylists3.xml` | 514 B | Playlist structure index (RB5) |
| `automixPlaylist.xml` | 82 B | Automix playlist data |
| `automixPlaylist6.xml` | 82 B | Automix playlist data (RB6+) |
| `playlists3.sync` | 195 B | Playlist sync state |

### Analysis & Media Files

```
~/Library/Pioneer/rekordbox/share/PIONEER/
├── USBANLZ/     (827 MB) — Analysis data, ~8500 files for ~2840 tracks
│   └── <hex>/<uuid>/
│       ├── ANLZ0000.DAT   — Core analysis (beat grid, waveform, cues, path)
│       ├── ANLZ0000.EXT   — Extended analysis (color waveforms, phrase analysis)
│       └── ANLZ0000.2EX   — Additional waveform data
└── Artwork/     (209 MB) — Album artwork, ~5800 files
    └── <hex>/<uuid>/
        ├── artwork.jpg     — Full resolution
        ├── artwork_m.jpg   — Medium thumbnail
        └── artwork_s.jpg   — Small thumbnail
```

### Other Rekordbox Locations

```
~/Library/Preferences/com.pioneerdj.rekordboxdj.plist    — App preferences
~/Library/Preferences/com.electron.rekordboxagent.plist   — Agent preferences
~/Library/Caches/com.pioneerdj.rekordboxdj/               — App cache (~2 MB)
~/Library/Caches/com.pioneer.Upmgr_rekordbox/             — Updater cache
~/Library/Caches/rekordbox/                                — Lock files only
~/Library/Logs/PioneerLog/                                 — Link/sync logs
```

### Key for Decryption (options.json)

```
~/Library/Application Support/Pioneer/rekordboxAgent/storage/options.json
```

This file contains the `db-path` and parameters used alongside the hardcoded encryption key.

### Settings File

```
~/Library/Application Support/Pioneer/rekordbox6/rekordbox3.settings
```

XML file with Rekordbox configuration, including `masterDbDirectory` which points to the database location.

### Total Library Size

- Database files: ~92 MB
- Analysis files: ~827 MB
- Artwork: ~209 MB
- **Total: ~1.1 GB**

---

## 2. The Encrypted Database (master.db)

### Encryption Details

- **Algorithm**: SQLCipher 4 (AES-256-CBC)
- **KDF**: PBKDF2-HMAC-SHA512, 256,000 iterations
- **HMAC**: HMAC-SHA512
- **Page size**: 4096 bytes
- **The key is universal** — same key for every Rekordbox 6/7 installation worldwide

### The Key

The key is a 64-character hex string. It was originally extractable from the Electron app's
`app.asar` JavaScript source. Since Rekordbox v6.6.5, the JS is compiled to V8 bytecode
(`.jsc` files), so the key is no longer in plaintext.

pyrekordbox hardcodes an obfuscated blob and decodes it via:
1. Base85 decode
2. XOR with 16-byte key (`657f48f84c437cc1`)
3. zlib decompress

The resulting key starts with `402fd...` (this is used as a validity check).

### Accessing from TypeScript/Node.js

Options for reading the encrypted DB:

1. **`@journeyapps/sqlcipher`** — SQLCipher-compatible SQLite binding for Node.js
2. **`better-sqlite3`** compiled with SQLCipher support
3. **Decrypt-then-read** — Use SQLCipher to create an unencrypted copy:
   ```sql
   PRAGMA key = '<hex_key>';
   ATTACH DATABASE 'decrypted.db' AS plaintext KEY '';
   SELECT sqlcipher_export('plaintext');
   DETACH DATABASE plaintext;
   ```

### Unencrypted Databases

The `networkAnalyze6.db` and `networkRecommend.db` are **not** encrypted (standard SQLite).
They contain analysis tracking metadata:

```sql
-- networkAnalyze6.db / networkRecommend.db schema
CREATE TABLE manage_tbl(
  SongID integer primary key,
  SongFilePath text not null,      -- Full path to audio file
  AnalyzeFilePath text not null,   -- Path to ANLZ files
  AnalyzeStatus integer not null,
  AnalyzePhrase integer not null,
  AnalyzeKey integer not null,
  AnalyzeBPMRange integer not null,
  RekordboxVersion text,
  AnalyzeVersion integer,
  TrackID text,
  TrackCheckSum integer,
  Duration integer,
  AnalysisType integer,
  OSVersion text,
  UploadStatus integer not null,
  Modified integer not null,
  DateUpload text,
  DateDownload text,
  AnalyzeFingerPrint integer,
  StreamingProvider text,
  StreamingTrackID text
);
```

---

## 3. Database Schema

The encrypted `master.db` contains **34 tables**. All IDs are `VARCHAR(255)` strings.

### Common Columns (present on all `djmd*` tables)

| Column | Type | Description |
|--------|------|-------------|
| `ID` | VARCHAR(255) PK | Primary key |
| `UUID` | VARCHAR(255) | Universal unique identifier |
| `rb_data_status` | Integer | Cloud sync status |
| `rb_local_data_status` | Integer | Local data status |
| `rb_local_deleted` | SmallInteger | Soft-delete flag |
| `rb_local_synced` | SmallInteger | Cloud sync flag |
| `usn` | BigInteger | Update sequence number (cloud) |
| `rb_local_usn` | BigInteger | Local USN |
| `created_at` | Text | Timestamp: `YYYY-MM-DD HH:MM:SS.SSS +00:00` |
| `updated_at` | Text | Last update timestamp |

### DjmdContent (Track Metadata) — Key Columns

| Column | Type | Notes |
|--------|------|-------|
| `Title` | VARCHAR(255) | Track title |
| `FolderPath` | VARCHAR(255) | Full file path |
| `ArtistID` | VARCHAR(255) FK | -> djmdArtist |
| `AlbumID` | VARCHAR(255) FK | -> djmdAlbum |
| `GenreID` | VARCHAR(255) FK | -> djmdGenre |
| `KeyID` | VARCHAR(255) FK | -> djmdKey |
| `ColorID` | VARCHAR(255) FK | -> djmdColor |
| `LabelID` | VARCHAR(255) FK | -> djmdLabel |
| `RemixerID` | VARCHAR(255) FK | -> djmdArtist |
| `ComposerID` | VARCHAR(255) FK | -> djmdArtist |
| `BPM` | Integer | BPM x 100 (e.g., 12800 = 128.00) |
| `Length` | Integer | Duration in seconds |
| `BitRate` | Integer | Bit rate |
| `BitDepth` | Integer | Bit depth |
| `SampleRate` | Integer | Sample rate in Hz |
| `FileType` | Integer | 1=MP3, 4=M4A, 5=FLAC, 11=WAV, 12=AIFF |
| `Rating` | Integer | Star rating |
| `ReleaseYear` | Integer | Release year |
| `Commnt` | Text | Comment field (note: typo in schema — `Commnt`) |
| `Tag` | VARCHAR(255) | Tag field |
| `ImagePath` | VARCHAR(255) | Album art path |
| `AnalysisDataPath` | VARCHAR(255) | Path to ANLZ files |
| `Analysed` | Integer | 0=none, 105=standard, 121=advanced, 233=locked |
| `DJPlayCount` | VARCHAR(255) | Play count |
| `TrackNo` | Integer | Track number on album |
| `DiscNo` | Integer | Disc number |
| `ISRC` | VARCHAR(255) | ISRC code |

### Metadata Lookup Tables

| Table | Key Columns |
|-------|-------------|
| `djmdArtist` | ID, Name, SearchStr |
| `djmdAlbum` | ID, Name, AlbumArtistID, ImagePath, Compilation, SearchStr |
| `djmdGenre` | ID, Name |
| `djmdKey` | ID, ScaleName, Seq |
| `djmdLabel` | ID, Name |
| `djmdColor` | ID, ColorCode, SortKey, Commnt |

### Playlist Tables

| Table | Key Columns |
|-------|-------------|
| `djmdPlaylist` | ID, Seq, Name, Attribute (0=playlist, 1=folder, 4=smart), ParentID |
| `djmdSongPlaylist` | ID, PlaylistID (FK), ContentID (FK), TrackNo |

### Cue Points (DjmdCue)

| Column | Type | Notes |
|--------|------|-------|
| `ContentID` | VARCHAR(255) FK | Track reference |
| `InMsec` | Integer | Cue position in milliseconds |
| `OutMsec` | Integer | Loop end in milliseconds |
| `Kind` | Integer | 0=memory cue, 3=load point, 4=loop |
| `Color` | Integer | Color (-1 = no color) |
| `Comment` | VARCHAR(255) | Cue label |

### Other Tables

| Table | Purpose |
|-------|---------|
| `djmdMyTag` / `djmdSongMyTag` | Custom tag organization |
| `djmdHotCueBanklist` / `djmdSongHotCueBanklist` | Hot cue banks |
| `djmdHistory` / `djmdSongHistory` | Play session history |
| `djmdSampler` / `djmdSongSampler` | Sampler pads |
| `djmdRelatedTracks` / `djmdSongRelatedTracks` | Related tracks |
| `djmdActiveCensor` | Explicit content censoring |
| `djmdProperty` | DB metadata (version, device ID) |
| `djmdMenuItems` | UI menu configuration |
| `agentRegistry` | Local key-value store |
| `contentFile` | File paths, hashes, sync status |

---

## 4. XML Export Format

### Official Documentation

Pioneer provides a PDF specification:
https://cdn.rekordbox.com/files/20200410160904/xml_format_list.pdf

Developer page: https://rekordbox.com/en/support/developer/

### Structure

```xml
<?xml version="1.0" encoding="UTF-8"?>
<DJ_PLAYLISTS Version="1.0.0">
  <PRODUCT Name="rekordbox" Version="7.2.10" Company="AlphaTheta"/>
  <COLLECTION Entries="2839">
    <TRACK TrackID="1" Name="Track Title" Artist="Artist Name"
           Composer="" Album="Album Name" Grouping="" Genre="Deep House"
           Kind="FLAC File" Size="45234567" TotalTime="432"
           DiscNumber="0" TrackNumber="1" Year="2023"
           AverageBpm="124.00" DateModified="2023-05-15"
           DateAdded="2023-06-01" BitRate="1411" SampleRate="44100"
           Comments="Great track" PlayCount="12"
           LastPlayed="2023-12-01" Rating="204"
           Location="file://localhost/Users/vz/Music/track.flac"
           Remixer="" Tonality="Am" Label="Giegling" Mix=""
           Colour="0x25FDE9">
      <TEMPO Inizio="0.123" Bpm="124.00" Metro="4/4" Battito="1"/>
      <POSITION_MARK Name="" Type="0" Start="0.123" Num="-1"/>
      <POSITION_MARK Name="Drop" Type="0" Start="64.500" Num="0"
                     Red="40" Green="226" Blue="20"/>
    </TRACK>
    <!-- ... more tracks ... -->
  </COLLECTION>
  <PLAYLISTS>
    <NODE Type="0" Name="ROOT" Count="2">
      <NODE Type="0" Name="My Folder" Count="1">
        <NODE Type="1" Name="Deep Cuts" KeyType="0" Entries="3">
          <TRACK Key="1"/>
          <TRACK Key="2"/>
          <TRACK Key="3"/>
        </NODE>
      </NODE>
    </NODE>
  </PLAYLISTS>
</DJ_PLAYLISTS>
```

### TRACK Attributes (Complete)

| Attribute | Type | Description | Notes |
|-----------|------|-------------|-------|
| `TrackID` | string | Track identifier | Unique within XML |
| `Name` | string | Track title | |
| `Artist` | string | Artist name | |
| `Composer` | string | Composer/producer | |
| `Album` | string | Album name | |
| `Grouping` | string | Grouping tag | Not a direct DB column |
| `Genre` | string | Genre name | Free text |
| `Kind` | string | File type description | e.g., "FLAC File", "WAV File", "MP3 File" |
| `Size` | int | File size in bytes | |
| `TotalTime` | int | Duration in seconds | No decimals |
| `DiscNumber` | int | Disc number | |
| `TrackNumber` | int | Track number | |
| `Year` | int | Release year | |
| `AverageBpm` | float | BPM with decimals | e.g., "128.00" |
| `DateModified` | string | Last modified | Format: `yyyy-mm-dd` |
| `DateAdded` | string | Date added to library | Format: `yyyy-mm-dd` |
| `BitRate` | int | Bit rate in Kbps | |
| `SampleRate` | float | Sample rate in Hz | |
| `Comments` | string | Comment field | Free text, **editable via XML** |
| `PlayCount` | int | Play count | |
| `LastPlayed` | string | Last played date | Format: `yyyy-mm-dd` |
| `Rating` | int | Star rating | 0=0, 51=1, 102=2, 153=3, 204=4, 255=5 |
| `Location` | string | File URI | `file://localhost/path/to/file.ext` |
| `Remixer` | string | Remixer name | |
| `Tonality` | string | Musical key | Classic: "Am", "Bb", "F#m"; Alphanumeric: "8A", "6B" |
| `Label` | string | Record label | |
| `Mix` | string | Mix name | |
| `Colour` | string | Track color | Hex RGB: `0xFF007F` |

### TEMPO Sub-element (Beat Grid)

| Attribute | Type | Description |
|-----------|------|-------------|
| `Inizio` | float | Start position in seconds |
| `Bpm` | float | BPM value at this point |
| `Metro` | string | Time signature (e.g., "4/4", "3/4", "7/8") |
| `Battito` | int | Beat number in bar (1-4 for 4/4) |

Multiple TEMPO elements per track are valid (for tracks with BPM changes).

### POSITION_MARK Sub-element (Cue Points)

| Attribute | Type | Description |
|-----------|------|-------------|
| `Name` | string | Cue point label |
| `Type` | int | 0=Cue, 1=Fade-In, 2=Fade-Out, 3=Load, 4=Loop |
| `Start` | float | Start position in seconds |
| `End` | float | End position in seconds (for loops) |
| `Num` | int | Hot cue slot: A=0, B=1, C=2...; Memory cue = -1 |
| `Red` | int | Red channel (0-255) — hot cue color |
| `Green` | int | Green channel (0-255) — hot cue color |
| `Blue` | int | Blue channel (0-255) — hot cue color |

### Playlist NODE Attributes

| Attribute | Type | Description |
|-----------|------|-------------|
| `Type` | int | 0=Folder, 1=Playlist |
| `Name` | string | Folder/playlist name |
| `Count` | int | Number of child nodes (folders only) |
| `Entries` | int | Number of tracks (playlists only) |
| `KeyType` | int | 0=TrackID, 1=Location |

### Track Colors (Colour attribute)

| Name | Hex | RGB |
|------|-----|-----|
| Rose | `0xFF007F` | 255, 0, 127 |
| Red | `0xFF0000` | 255, 0, 0 |
| Orange | `0xFFA500` | 255, 165, 0 |
| Lemon | `0xFFFF00` | 255, 255, 0 |
| Green | `0x00FF00` | 0, 255, 0 |
| Turquoise | `0x25FDE9` | 37, 253, 233 |
| Blue | `0x0000FF` | 0, 0, 255 |
| Violet | `0x660099` | 102, 0, 153 |

### Musical Key (Tonality) Values

Rekordbox uses classic notation. The Camelot wheel mapping:

| Classic | Camelot | Classic | Camelot |
|---------|---------|---------|---------|
| G#m | 1A | B | 1B |
| Ebm | 2A | Gb | 2B |
| Bbm | 3A | Db | 3B |
| Fm | 4A | Ab | 4B |
| Cm | 5A | Eb | 5B |
| Gm | 6A | Bb | 6B |
| Dm | 7A | F | 7B |
| Am | 8A | C | 8B |
| Em | 9A | G | 9B |
| Bm | 10A | D | 10B |
| F#m | 11A | A | 11B |
| C#m | 12A | E | 12B |

Display format selectable in Preferences > View > Key display format: Classic or Alphanumeric (Camelot).

### Rating Values

| Stars | XML Value |
|-------|-----------|
| 0 | 0 |
| 1 | 51 |
| 2 | 102 |
| 3 | 153 |
| 4 | 204 |
| 5 | 255 |

---

## 5. XML Import/Export Workflow

### Exporting

1. In Rekordbox: **File > Export Collection in xml format**
2. Choose save location in file dialog
3. Exports the entire collection with all metadata, playlists, cue points, and beat grids

### Importing

1. In Rekordbox: **File > Import > Import Playlist/Collection**
2. Navigate to the XML file
3. Select and click Open
4. Tracks appear in the "rekordbox xml" section of the browser

### Setting as Live XML Source

1. **Preferences > Advanced > rekordbox xml** — set path to your XML file
2. **Preferences > View > Layout** — enable "rekordbox xml"
3. The XML library appears as a separate tree in the browser sidebar
4. You can drag tracks/playlists from XML view into your main collection

### What Survives a Round-Trip (Export → Modify → Import)

**Preserved on reimport:**
- Genre, Comments, Rating, Colour — **these are the primary targets for reklawdbox**
- Track title, artist, album, composer, remixer, label
- Tonality (key), BPM
- Cue points (POSITION_MARK) — including hot cue colors
- Beat grids (TEMPO)
- Playlist structure and track ordering
- Play count, dates

**NOT preserved / limitations:**
- My Tags — not represented in XML format
- Hot Cue Banks — not in XML
- Phrase analysis (PSSI) — not in XML
- Smart playlist criteria — not in XML
- Related tracks — not in XML
- Analysis status flags — not in XML
- Waveform data — not in XML (stored in ANLZ files)
- Active censors — not in XML

**Matching behavior on import:**
- Rekordbox matches tracks by `Location` (file path)
- If the path doesn't match any existing track, a new entry is created
- **BUG**: If a track with the same path already exists, metadata is **NOT updated** via standard import (RB 5.6.1+). You must use the two-step workaround: import playlist first, then select all tracks and import again to force overwrite.
- TrackIDs in the XML are local to that XML file, not the same as DB IDs
- XML import is **additive only** — removing tracks from XML doesn't delete them from RB

### Critical Import Notes

1. **Location paths must be valid** — Rekordbox won't import tracks it can't find on disk
2. **URI encoding** — paths use `file://localhost/` prefix with URL-encoded special chars
3. **Entries count must match** — `<COLLECTION Entries="N">` must equal actual track count
4. **Rekordbox should be closed** when writing XML intended for import (to avoid conflicts)
5. **XML is UTF-8** — ensure proper encoding for international characters

---

## 6. Analysis Files (ANLZ)

### File Types

| Extension | Contents |
|-----------|----------|
| `.DAT` | Core: beat grid, monochrome waveform, cue points, file path |
| `.EXT` | Extended: color waveforms, extended cue points with colors, phrase analysis |
| `.2EX` | Additional: extra waveform formats (PWV6, PWV7, PWVC) |

### Binary Format

All values are **big-endian**.

**File header** (28 bytes): Magic `PMAI`, header length (28), file length, 4 reserved uint32s.

**Each section** starts with a 12-byte tag envelope: 4-char type code, header length, total tag length.

### Tag Types

| Code | Name | File | Description |
|------|------|------|-------------|
| `PPTH` | Path | .DAT | UTF-16-BE file path |
| `PQTZ` | Beat Grid | .DAT | Beat entries: position (ms), BPM x100 |
| `PQT2` | Extended Beat Grid | .EXT | More detailed beat data |
| `PCOB` | Cue List | .DAT/.EXT | Memory cues and hot cues |
| `PCO2` | Extended Cue | .EXT | Cues with color and comments |
| `PSSI` | Phrase Analysis | .EXT | Song structure (intro/verse/chorus/bridge/outro) |
| `PWAV` | Waveform Preview | .DAT | Monochrome, 400 entries |
| `PWV3` | Detail Waveform | .EXT | 150 entries/second |
| `PWV4` | Color Preview | .EXT | 1200-column color waveform |
| `PWV5` | Color Detail | .EXT | RGB waveform |
| `PVBR` | VBR Index | .DAT | 400 frame indices for VBR files |

### PSSI XOR Encryption

Rekordbox 6+ XOR-encrypts the phrase analysis data in exported ANLZ files:

```
XOR_MASK = CB E1 EE FA E5 EE AD EE E9 D2 E9 EB E1 E9 F3 E8 E9 F4 E1
```

Detection: check if `mood` bytes (offset 18-19) are valid (1-3). If not, decrypt by XORing
each byte from offset 18 with `(XOR_MASK[i % 19] + len_entries) & 0xFF`.

---

## 7. Metadata Fields Reference

### Fields Most Useful for DJ Workflow

| Field | XML Attribute | DB Column | Editable via XML | Notes |
|-------|--------------|-----------|-----------------|-------|
| **Genre** | `Genre` | `GenreID` (FK) | Yes | Free text — our primary target |
| **Comments** | `Comments` | `Commnt` | Yes | Free text — great for notes/tags |
| **Rating** | `Rating` | `Rating` | Yes | 0/51/102/153/204/255 |
| **Color** | `Colour` | `ColorID` (FK) | Yes | 8 preset colors (hex) |
| **Key** | `Tonality` | `KeyID` (FK) | Yes | Classic or Camelot notation |
| **BPM** | `AverageBpm` | `BPM` | Yes | Float in XML, int x100 in DB |
| **Artist** | `Artist` | `ArtistID` (FK) | Yes | |
| **Album** | `Album` | `AlbumID` (FK) | Yes | |
| **Label** | `Label` | `LabelID` (FK) | Yes | |
| **Remixer** | `Remixer` | `RemixerID` (FK) | Yes | |
| **Year** | `Year` | `ReleaseYear` | Yes | |
| **Grouping** | `Grouping` | — | Yes | Not a direct DB column |
| **Mix** | `Mix` | — | Yes | Mix name field |
| **My Tag** | — | `djmdSongMyTag` | **No** | DB only, not in XML |
| **Energy** | — | — | No | Not a native RB field (use Comments) |

### DB-Only Fields (not in XML)

- History (play sessions)
- My Tags
- Hot Cue Banks
- Smart playlist criteria
- Phrase analysis
- ISRC codes
- Sampler pad assignments
- Related tracks grouping

### Genre Strategy for reklawdbox

The `Genre` field is free text — Rekordbox has no predefined genre list.
This means we can set any genre string we want. Recommendations:

- Use consistent, hierarchical genre names (e.g., "House / Deep House")
- The `Comments` field is excellent for additional tags that don't fit genre
- `Grouping` can serve as a secondary classification
- `Color` can encode broad genre families visually (e.g., Green=House, Blue=Techno)
- `Rating` can encode energy or quality

---

## 8. pyrekordbox Reference

### What It Is

Python library by Dylan Jones for reading/writing Rekordbox data files.
GitHub: https://github.com/dylanljones/pyrekordbox

### Capabilities

- **Read/write encrypted master.db** via SQLCipher
- **Read/write XML** export files
- **Parse/write ANLZ** analysis files (beat grids, cue points, waveforms)
- **Parse MySettings** files
- Auto-detects Rekordbox installation paths on macOS and Windows
- Full SQLAlchemy ORM for all 34 database tables

### Key Architecture Details

1. **Encryption**: Hardcoded obfuscated blob → Base85 → XOR → zlib → hex key
2. **DB Access**: SQLAlchemy + `sqlcipher3` (pysqlcipher dialect)
3. **USN Tracking**: `RekordboxAgentRegistry` auto-increments update sequence numbers on commit
4. **ANLZ Parsing**: Uses `construct` library for binary struct definitions
5. **XML Handling**: Standard `xml.etree.cElementTree` with custom Track/Node wrappers

### Relevance to reklawdbox

We will **not** build on pyrekordbox directly (it's Python), but we learn from it:

1. The decryption key and process (replicate in Node.js if we go the DB route)
2. The complete database schema (34 tables, all column types)
3. The ANLZ binary format (if we ever need to parse waveforms/phrases)
4. The XML Track class API design (good model for our TypeScript types)
5. The gotchas (USN management, foreign key constraints, Rekordbox must be closed)

---

## 9. Ecosystem Tools

### Libraries & Frameworks

| Tool | Language | What It Does | URL |
|------|----------|-------------|-----|
| **pyrekordbox** | Python | Full RB data access (DB, XML, ANLZ) | [GitHub](https://github.com/dylanljones/pyrekordbox) |
| **rekordbox-connect** | Node.js | Reads encrypted master.db directly, emits change events | [GitHub](https://github.com/chrisle/rekordbox-connect) |
| **rbx-gen** | TypeScript | Generates Rekordbox XML from playlists | [GitHub](https://github.com/FunctionDJ/rbx-gen) |
| **rekordbox-library-fixer** | Node.js | XML manipulation tools for library fixes | [GitHub](https://github.com/koraysels/rekordbox-library-fixer) |
| **prolink-connect** | TypeScript | Pioneer DJ Link protocol (CDJ communication) | [GitHub](https://github.com/evanpurkhiser/prolink-connect) |
| **crate-digger** | Java | Parses RB export data (USB drives), Kaitai Struct defs | [GitHub](https://github.com/Deep-Symmetry/crate-digger) |
| **dj-data-converter** | Clojure | Converts between RB/Traktor/Serato XML | [GitHub](https://github.com/digital-dj-tools/dj-data-converter) |
| **DJ-Tools (djtools)** | Python | Tag-based playlist automation from Genre/Comments fields | [GitHub](https://github.com/a-rich/DJ-Tools) |
| **Rekord Buddy** | C++ | Cross-platform library converter (open source) | [GitHub](https://github.com/gadgetmies/RekordBuddy) |
| **CueGen** | C# | Creates RB cue points from Mixed in Key | [GitHub](https://github.com/mganss/CueGen) |
| **Automark** | Python | Auto-generates cue points for RB | [GitHub](https://github.com/MichelleAppel/Automark-for-Rekordbox) |

### MCP Servers

| Tool | Language | What It Does | URL |
|------|----------|-------------|-----|
| **rekordbox-mcp** | Python | MCP server for RB database (read-only, uses pyrekordbox) | [GitHub](https://github.com/davehenke/rekordbox-mcp) |

> **Note**: rekordbox-mcp by Dave Henke is directly relevant — it exposes track search, playlist ops,
> and library stats via MCP using FastMCP 2.0 + pyrekordbox. Read-only for safety. Our MCP server
> could follow a similar pattern but in TypeScript, and add write capabilities via XML.

### Commercial Tools

| Tool | What It Does | Price |
|------|-------------|-------|
| **Lexicon DJ** | Library management, format conversion, batch editing | $12/mo or $120/yr |
| **Rekordcloud** | Library tools, duplicate scanner, auto cues, conversion | $7.50-15/mo |
| **MIXO** | Library management and conversion | Free tier + paid |
| **DJCU** (DJ Conversion Utility) | Format conversion | One-time purchase |

### Audio Analysis Tools

| Tool | What It Does | URL |
|------|-------------|-----|
| **Chromaprint / AcoustID** | Audio fingerprinting | [acoustid.org](https://acoustid.org/) |
| **Essentia** | Audio feature extraction (BPM, key, energy) | [essentia.upf.edu](https://essentia.upf.edu/) |
| **aubio** | Beat/onset/key detection | [aubio.org](https://aubio.org/) |

### Emerging: OneLibrary

AlphaTheta (Pioneer's parent) launched OneLibrary in 2025 — a unified export format
across Rekordbox, Traktor, and djay Pro. Specification is **not** public. Interesting
for the future but not actionable for us yet.

---

## 10. Gotchas and Footguns

### Database Access

1. **Rekordbox MUST be closed** before writing to `master.db` — concurrent access will corrupt
2. **WAL mode** — `master.db-shm` and `master.db-wal` contain uncommitted data. Back up all three files together
3. **USN management** — every write must increment update sequence numbers or cloud sync breaks
4. **Foreign keys** — setting `GenreID = "123"` isn't enough; the djmdGenre row must exist
5. **The `Commnt` typo** — it's `Commnt` not `Comment` in the DB schema
6. **IDs are strings** — VARCHAR(255), not integers, despite looking numeric

### XML Import

7. **THE REIMPORT BUG (critical)** — In Rekordbox 5.6.1+ (including all of RB6 and RB7), tracks that already exist in the collection are **NOT updated** when imported from XML via the standard method. **Workaround**: right-click playlist > "Import To Collection" (imports new tracks), then select all tracks (Cmd+A), right-click > "Import To Collection" again (forces overwrite of existing). This two-step process is required for our genre-tagging workflow to work.
8. **Entries count must match** — mismatch between `<COLLECTION Entries="N">` and actual track count causes import failure
9. **Location matching** — RB matches by file path on import; wrong paths = orphaned entries
10. **URI encoding** — spaces become `%20`, special chars must be properly encoded
11. **No My Tags in XML** — if you rely on My Tags, XML won't preserve them
12. **Rating values are weird** — not 0-5 but 0/51/102/153/204/255
13. **Genre is just text** — no validation, no predefined list. Consistency is on us
14. **XML is additive only** — removing a track from the XML does NOT remove it from Rekordbox on reimport
15. **Memory cue colors lost** — memory cues export but their colors don't (only hot cue colors survive)
16. **Kind is locale-dependent** — German exports produce `"Mp3-Datei"` instead of `"MP3 File"`
17. **Date format inconsistency** — RB sometimes exports `yyyy-m-d` instead of `yyyy-mm-dd`

### File Paths

13. **Windows → macOS migration** — your library has legacy `E:/audio/` and `C:/Users/moey/` paths in `networkAnalyze6.db`. The main DB should have updated paths
14. **Artwork paths** — stored as hashed UUIDs, not human-readable
15. **ANLZ paths** — similarly hashed, referenced by `AnalysisDataPath` in DjmdContent

### General

16. **No official API** — everything we do is reverse-engineered or uses the XML interchange format
17. **Updates can break things** — Pioneer changed the key storage in v6.6.5 and could change the DB schema
18. **Backup before everything** — this cannot be overstated

---

## 11. Architecture Decision: DB Reads + XML Writes

### Chosen: Hybrid Approach

**Workflow**: Read directly from encrypted master.db → stage changes in memory → write Rekordbox XML → reimport

This combines the strengths of both approaches:

- **DB reads** eliminate the manual export step — the MCP server connects directly to the
  encrypted master.db via SQLCipher (rusqlite with bundled-sqlcipher in Rust), giving instant
  read access to all 34 tables without user interaction
- **XML writes** maintain safety — changes are written as standard Rekordbox XML files,
  which are reimported through Rekordbox's official import workflow. No direct DB writes
  means zero risk of database corruption or USN tracking issues

### Why Not Pure XML?

Requiring a manual XML export before every session added friction. Since reads are
non-destructive (read-only SQLCipher connection), direct DB access is safe for the read side.

### Why Not Pure DB Writes?

Writing to master.db requires careful USN management, Rekordbox must be closed, and a bug
could corrupt the library. XML writes are the official interchange format and avoid all of
these risks.

### Implementation

- **Language**: Rust (single static binary, ~8.5 MB arm64, zero runtime dependencies)
- **DB access**: rusqlite with bundled-sqlcipher feature, read-only connection
- **Writes**: Template-string XML generation (no XML library dependency)
- **State**: In-memory change staging via Arc<Mutex<HashMap>>

---

## References

### Official
- [Rekordbox Developer Page](https://rekordbox.com/en/support/developer/)
- [Rekordbox XML Format Specification (PDF)](https://cdn.rekordbox.com/files/20200410160904/xml_format_list.pdf)

### Format Documentation
- [pyrekordbox GitHub](https://github.com/dylanljones/pyrekordbox)
- [pyrekordbox DB6 Format Docs](https://pyrekordbox.readthedocs.io/en/latest/formats/db6.html)
- [pyrekordbox XML Format Docs](https://pyrekordbox.readthedocs.io/en/latest/formats/xml.html)
- [Deep Symmetry ANLZ Analysis](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html)
- [Deep Symmetry DJ Link Ecosystem](https://djl-analysis.deepsymmetry.org/) — Canonical format reference
- [Pioneer DB Encryption Research](https://github.com/liamcottle/pioneer-rekordbox-database-encryption)

### Community
- [Pioneer DJ Community Forums](https://community.pioneerdj.com/)
- [DJ-Tools Tagging Guide](https://a-rich.github.io/DJ-Tools-dev-docs/conceptual_guides/tagging_guide/) — Genre tagging workflow reference
- [rekordbox-mcp](https://github.com/davehenke/rekordbox-mcp) — Existing MCP server for Rekordbox
