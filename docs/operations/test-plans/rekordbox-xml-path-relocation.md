# Test Plan: Rekordbox XML Path Relocation

## Goal

Determine whether Rekordbox XML import can update the file path (`Location`) of an existing imported track, or whether it only adds new entries. This is the critical blocker for supporting renames of imported files via the audit engine.

## Background

When a file is renamed on disk, Rekordbox shows it as "missing" until the user manually relocates it. If XML import can update the `Location` for a known track (matched by some other key like title+artist or an internal ID), then the audit engine could export an XML with updated paths after a rename, and the user could reimport to fix references automatically.

## Prerequisites

- Rekordbox 7.x running
- reklawdbox MCP available
- A track imported in Rekordbox that you're willing to temporarily rename

## Test Candidate

Pick a low-risk track. Suggested criteria:
- Not in any active playlists or prepared sets
- Low play count / low rating
- Simple filename (no special characters)

Example from the library:

```
ID:    109176001
Title: #four
Artist: Daniel Stefanik
Path:  /Users/vz/Music/play/play22/Daniel Stefanik - #four.wav
```

## Steps

### 1. Baseline — export XML with current path

```
# Stage a trivial, reversible change to get the track into an export
update_tracks(tracks=[{id: "109176001", comment: "relocation-test"}])
write_xml(output_path="./rekordbox-exports/relocation-test-before.xml")
clear_changes()
```

Open the exported XML. Find the `<TRACK>` entry and note the `Location` attribute (should be a `file://localhost/...` URI). This is the "before" state.

### 2. Rename the file on disk

```bash
mv "/Users/vz/Music/play/play22/Daniel Stefanik - #four.wav" \
   "/Users/vz/Music/play/play22/Daniel Stefanik - #four-RELOCATED.wav"
```

Verify in Rekordbox that the track now shows as "missing" (orange exclamation icon or similar).

### 3. Edit the XML with the new path

Copy the exported XML and edit the `Location` attribute to point to the new filename:

```bash
cp rekordbox-exports/relocation-test-before.xml rekordbox-exports/relocation-test-after.xml
```

Edit `relocation-test-after.xml`: change the `Location` from:

```
file://localhost/Users/vz/Music/play/play22/Daniel%20Stefanik%20-%20%23four.wav
```

to:

```
file://localhost/Users/vz/Music/play/play22/Daniel%20Stefanik%20-%20%23four-RELOCATED.wav
```

Keep all other attributes identical.

### 4. Import the modified XML into Rekordbox

In Rekordbox:
1. File > Import > rekordbox xml
2. Browse to `relocation-test-after.xml`
3. The XML should appear in the sidebar under "rekordbox xml"
4. Find the track in the XML tree
5. Right-click > Import to Collection (or drag to Collection)

### 5. Observe the result

Check the track in your Collection:

| Outcome | Meaning |
|---------|---------|
| **Track path updated, no duplicate** | XML import CAN update paths. The audit engine can export relocation XMLs. |
| **Duplicate entry created** | XML import treats it as a new track. Path relocation via XML is not viable. |
| **Track ignored / nothing happens** | Rekordbox skipped the import because it matched an existing entry but didn't update it. Path relocation via XML is not viable. |
| **Error / warning dialog** | Note the exact message. May indicate partial support. |

### 6. Clean up

Regardless of outcome, restore the original state:

```bash
mv "/Users/vz/Music/play/play22/Daniel Stefanik - #four-RELOCATED.wav" \
   "/Users/vz/Music/play/play22/Daniel Stefanik - #four.wav"
```

If a duplicate was created in Rekordbox, delete it manually.

Revert the comment change if it was imported:

```
update_tracks(tracks=[{id: "109176001", comment: ""}])
write_xml()
# Import to restore original comment
clear_changes()
```

## Results

**Date:** 2026-02-25
**Rekordbox version:** 7.2.10
**Outcome:** Duplicate entry created
**Notes:** XML import created a new track entry pointing to the relocated file. The original track remained in the collection with "missing" status. The duplicate had no cover art (WAV cover art is not exported in XML). Metadata (title, artist, BPM, cue points) was imported correctly on the duplicate. Conclusion: Rekordbox XML import matches tracks by Location path, not by TrackID or metadata — a different Location is always treated as a new track.

## Impact on Audit Spec

- **If path update works:** Add a `relocation_xml` export step to `auto_fix` for rename-safe issues on imported files. The workflow becomes: rename on disk → export XML with new paths → user reimports.
- **If path update doesn't work:** Renames of imported files remain blocked. The audit engine flags them as "needs manual relocation" and the user handles it through Rekordbox's built-in relocate feature.
