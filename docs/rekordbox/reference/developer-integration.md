---
id: developer-integration
title: "Developer Integration"
type: reference
source:
  url: "https://rekordbox.com/en/support/developer/"
  version: "7.x"
topics: [export, import, library, metadata, xml]
modes: [common]
confidence: verified
last_verified: "2026-02-17"
transcribed_by: agent
verified_by: agent
---

# rekordbox for Developers

## XML playlist

You can now display your rekordbox playlists in the [Bridge] pane by importing playlist information from an XML file.

1. Create an XML file and save it in your desired location.

2. Open the previously created XML file in a text editor. The first row should display as:

   ```xml
   <?xml version="1.0" encoding="UTF-8" ?>
   ```

   In order to save playlists and their information to rekordbox, all rows from the second row and beyond must follow a format which rekordbox supports. Please refer to [a list of XML formats which rekordbox supports (PDF)](https://cdn.rekordbox.com/files/20200410160904/xml_format_list.pdf).

3. Start up rekordbox and select the generated XML file you want to import under [File] > [Preferences] > [Bridge] > [Imported Library].

Multiple playlists or playlist folders with the same name can't exist in the same level of a directory.

## Workflow Response Provenance

Corpus-backed workflow responses include additive provenance fields:

- `consulted_documents`: ordered, de-duplicated corpus document paths consulted for the workflow.
- `manifest_status`: corpus retrieval status. Current values are `ok`, `empty`, or `unavailable`.
- `corpus_warning` (optional): present when manifest-first retrieval falls back to default references (for example, manifest load failure or no ranked matches).
- `write_xml` no-change contract: when no staged changes exist, the response remains a JSON payload with `"message": "No changes to write."` plus `track_count`, `changes_applied`, and provenance fields.

XML workflow note: XML operations use manifest-first retrieval for XML/reference docs and fall back to stable XML anchors (`reference/xml-import-export.md`, `guides/xml-format-spec.md`, and this developer integration reference) when needed.

Genre workflow note: genre normalization operations use manifest-first retrieval across genre/metadata/library docs and fall back to stable genre/library references when retrieval is unavailable or empty.

Fallback behavior: workflows still return normal operation results; provenance indicates fallback via non-`ok` `manifest_status` and optional `corpus_warning`.

## Support

### Ask the forum

[View forum](https://community.pioneerdj.com/hc/en-us/community/topics)

### Inquiries

[Make an Inquiry](https://forums.pioneerdj.com/hc/en-us/requests/new?ticket_form_id=72145)

## Related Documents

- [reference/xml-import-export.md](xml-import-export.md) (export, import, library, metadata)
- [faq/library-and-collection.md](../faq/library-and-collection.md) (export, import, library, metadata)
- [guides/xml-format-spec.md](../guides/xml-format-spec.md) (export, import, metadata, xml)
- [manual/03-adding-tracks.md](../manual/03-adding-tracks.md) (import, library, xml)
- [manual/09-collaborative-playlists.md](../manual/09-collaborative-playlists.md) (export, import, xml)
- [features/overview.md](../features/overview.md) (export, library)
- [guides/device-library-backup.md](../guides/device-library-backup.md) (export, library)
- [guides/introduction.md](../guides/introduction.md) (export, library)
