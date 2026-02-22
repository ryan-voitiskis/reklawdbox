# Backup & Restore Guide

## Overview

reklawdbox includes a backup system that snapshots your Rekordbox library before any
modifications. This document covers usage, restore procedures, and what's protected.

## Backup Location

```
~/Library/Pioneer/rekordbox-backups/
```

## Quick Reference

```bash
# Full backup (~1GB compressed) — databases + analysis + artwork
./backup.sh

# Database only (~92MB compressed) — just the critical metadata files
./backup.sh --db-only

# Pre-operation snapshot (called automatically by reklawdbox tools)
./backup.sh --pre-op

# List all backups
./backup.sh --list

# Restore from a backup
./backup.sh --restore <path-to-backup.tar.gz>
```

## What Gets Backed Up

### Database Backup (--db-only)

These are the critical files that contain your library metadata:

| File | What It Contains |
|------|-----------------|
| `master.db` + `-shm` + `-wal` | Entire library: tracks, playlists, cue points, tags, ratings, everything |
| `master.backup.db` | Rekordbox's own backup #1 |
| `master.backup2.db` | Rekordbox's own backup #2 |
| `master.backup3.db` | Rekordbox's own backup #3 |
| `networkAnalyze6.db` | Analysis tracking and metadata |
| `networkRecommend.db` | Recommendation data |
| `product.db` | Product/licensing data |
| `datafile.edb` + backup | Legacy data store |
| `ExtData.edb` + backup | Extended data store |
| `masterPlaylists6.xml` | Playlist structure |
| `masterPlaylists3.xml` | Legacy playlist structure |
| `automixPlaylist*.xml` | Automix playlists |
| `playlists3.sync` | Playlist sync state |

### Full Backup

Everything above **plus**:
- **USBANLZ/** (~827 MB) — waveforms, beat grids, phrase analysis, cue points in binary format
- **Artwork/** (~209 MB) — album art in three sizes (full, medium, small)

## When to Use Each

| Scenario | Backup Type |
|----------|-------------|
| Before running reklawdbox write operations | `--db-only` (automatic via `--pre-op`) |
| Before a Rekordbox update | Full backup |
| Weekly routine | `--db-only` |
| Before major library reorganization | Full backup |
| Before migrating to a new machine | Full backup |

## Restore Procedure

### Step 1: Close Rekordbox

**Rekordbox MUST be completely closed before restoring.** The restore script checks for this
and will refuse to proceed if Rekordbox is running.

```bash
# Check if running
pgrep -l rekordbox

# If it is, quit via the app or:
osascript -e 'quit app "rekordbox"'
```

### Step 2: Run the Restore

```bash
# List available backups
./backup.sh --list

# Restore a specific backup
./backup.sh --restore ~/Library/Pioneer/rekordbox-backups/db_20260215_233936.tar.gz
```

The script will:
1. Show you the archive contents
2. Tell you whether it's a full or db-only restore
3. Ask for explicit confirmation (type `YES`)
4. Create a safety backup of the current state before restoring
5. Extract the backup files into the Rekordbox data directory

### Step 3: Verify

1. Launch Rekordbox
2. Check your library — tracks, playlists, metadata should be intact
3. Play a track to verify analysis data is working

### Manual Restore (if the script fails)

If you need to restore manually:

```bash
# Close Rekordbox first!

# For a db-only backup:
cd ~/Library/Pioneer/rekordbox/
tar -xzf ~/Library/Pioneer/rekordbox-backups/db_YYYYMMDD_HHMMSS.tar.gz

# For a full backup:
cd ~/Library/Pioneer/
tar -xzf ~/Library/Pioneer/rekordbox-backups/full_YYYYMMDD_HHMMSS.tar.gz
```

## Backup Rotation

The script automatically manages backup count:
- **Full backups**: Keeps the 5 most recent, deletes older ones
- **DB/pre-op backups**: Keeps the 20 most recent

## Storage Estimates

| Library Size | DB Backup | Full Backup |
|-------------|-----------|-------------|
| ~2,800 tracks | ~92 MB | ~800 MB |
| ~5,000 tracks | ~150 MB | ~1.5 GB |
| ~10,000 tracks | ~300 MB | ~3 GB |

## Important Notes

1. **Back up the WAL file** — `master.db-shm` and `master.db-wal` contain uncommitted
   transactions. Always back up all three files together (the script does this).

2. **Rekordbox's own backups** — Rekordbox creates `master.backup.db`, `master.backup2.db`,
   and `master.backup3.db` automatically. Our backups include these too, but they're an
   additional safety net.

3. **Rekordbox's built-in backup** — You can also use Rekordbox's own backup feature at
   **File > Library > Backup Library**. This creates a `.zip` file with the database and
   settings. It's a good complement to our backups.

4. **Audio files are NOT backed up** — Your actual music files (FLAC, WAV, MP3) live in your
   music folder and are not part of the Rekordbox data directory. Back those up separately.
