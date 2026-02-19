---
id: xml-import-export
title: "rekordbox XML Import/Export Reference"
type: reference
source:
  file: "guides/xml-format-spec.md + manual/31-preferences.md + faq/library-and-collection.md + docs/rekordbox-internals.md"
  url: "https://rekordbox.com/en/support/developer/"
  version: "7.x"
topics: [export, import, library, metadata, xml]
modes: [common, export, performance]
confidence: high
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

# rekordbox XML Import/Export Reference

This document consolidates XML workflow behavior from official rekordbox docs and reklawdbox project notes.

## What XML Is Used For

- Importing external playlist libraries into the [Bridge] pane via [Imported Library].
- Exporting collection and playlist metadata for external tools.
- Sharing track metadata, beatgrid, cue marks, and playlist tree structure in a documented XML format.

## Exporting Collection as XML

- In rekordbox, use `File > Export Collection in xml format` to export your collection metadata.
- FAQ guidance for Tribe XR references this as the primary export path.
- XML export can include beatgrid information when enabled in [Preferences] > [Advanced].

## Auto-Export and Preferences

- In [Preferences] > [Advanced], [rekordbox xml] settings include [Export BeatGrid information].
- [Imported Library] specifies the XML file location used to browse under [rekordbox xml].
- [Tribe XR] settings include XML auto-export workflow and Dropbox destination settings.
- [Auto Export] (EXPORT mode) controls automatic export behavior for music imported from external devices.

## Importing XML via Bridge

From developer guidance:
- Build a UTF-8 XML file in rekordbox-supported schema (header starts with `<?xml version="1.0" encoding="UTF-8" ?>`).
- Configure the file location at `[File] > [Preferences] > [Bridge] > [Imported Library]`.
- rekordbox then exposes the imported playlist structure in the [Bridge] pane.
- Constraint: multiple playlists/folders with the same name cannot exist at the same directory level.

## XML Format Specification Summary

Based on `guides/xml-format-spec.md` (XML format list PDF):
- Root tree: `DJ_PLAYLISTS > PRODUCT / COLLECTION / PLAYLISTS`.
- `COLLECTION` contains track metadata fields like `TrackID`, `Name`, `Artist`, `Genre`, `AverageBpm`, `Location`, and others.
- `TRACK > TEMPO` stores beatgrid timing (`Inizio`, `Bpm`, `Metro`, `Battito`).
- `TRACK > POSITION_MARK` stores cue/loop marks (`Type`, `Start`, `End`, `Num`).
- `PLAYLISTS > NODE` stores folder/playlist hierarchy and track keys by TrackID or Location.
- Strings are UTF-8; LOCATION is URI-encoded, expected under `file://localhost/` path format.

## Known Issues and Workarounds

- Reimport consistency: keep stable `TrackID` and/or `Location` values to reduce duplicate/mismatch behavior across repeated imports.
- File paths: URI encoding and path normalization are required; malformed `Location` values commonly break matching.
- Playlist naming collisions: same-level duplicate playlist/folder names are not allowed in imported structures.
- Operational safety for reklawdbox: prefer idempotent XML generation and dry-run validation before replacing production XML files.

## Source Evidence Highlights

From `manual/31-preferences.md`:
- [rekordbox xml], [Explorer], and [SEARCH MOBILE button] in the [Media Browser] window. |
- [Imported Library] | Specify the playlist library (the location of the xml file) to browse on [rekordbox xml]. |
- [Tribe XR] | Set up the XML file for use with Tribe XR. |
- [Auto Export] (EXPORT mode) | Music files imported from an external device are automatically exported. | |
- [Export BeatGrid information] | When exporting music file information as an xml file, beatgrid information can be output to the xml file. |

From `faq/library-and-collection.md`:
- **Can I automatically export XML format collections?**: By performing settings with the following procedures, you can automatically export collections when closing rekordbox. 1. Open [Preferences] > [Advanced] category > [Others] tab. 2. Enable [XML Auto Export] in [Tribe XR]. 3. Specify a saving destination on [Location of the xml file on dropbox].
- **Can the rekordbox library be used on Tribe XR?**: Yes you can. It can be used by exporting collection in XML format. Export collection in XML format with the following procedures. 1. Select [File] menu > [Export Collection in xml format]. 2. Copy the exported XML file on to Dropbox. 3. Connect to Dropbox from Tribe XR and load the exported XML.

## Related Sources

- `docs/rekordbox/guides/xml-format-spec.md`
- `docs/rekordbox/reference/developer-integration.md`
- `docs/rekordbox/manual/31-preferences.md`
- `docs/rekordbox/faq/library-and-collection.md`
- `docs/rekordbox-internals.md`

## Related Documents

- [reference/developer-integration.md](developer-integration.md) (export, import, library, metadata)
- [faq/library-and-collection.md](../faq/library-and-collection.md) (export, import, library, metadata)
- [guides/xml-format-spec.md](../guides/xml-format-spec.md) (export, import, metadata, xml)
- [manual/03-adding-tracks.md](../manual/03-adding-tracks.md) (import, library, xml)
- [manual/09-collaborative-playlists.md](../manual/09-collaborative-playlists.md) (export, import, xml)
- [features/overview.md](../features/overview.md) (export, library)
- [guides/device-library-backup.md](../guides/device-library-backup.md) (export, library)
- [guides/introduction.md](../guides/introduction.md) (export, library)
