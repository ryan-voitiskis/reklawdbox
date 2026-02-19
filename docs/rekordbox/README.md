# Rekordbox Knowledge Corpus

Comprehensive rekordbox 7.x corpus for reklawdbox agent workflows (library management, metadata operations, XML import/export, and operational troubleshooting).

- Software baseline: `7.2.8`
- Last updated: `2026-02-17`
- Document count: `65`
- Source inventory: `18` PDFs, `288` FAQ items, `4` web pages

## Agent Usage Pattern

Use a manifest-first discovery flow:
1. Read `docs/rekordbox/manifest.yaml` to find candidate docs by `topics`, `modes`, and `type`.
2. Resolve a narrow reading set, then open those markdown files directly.
3. Prefer `reference/` docs first for synthesis tasks, then drop to source transcriptions (`manual/`, `guides/`, `faq/`).
4. For XML workflows, validate assumptions against both `guides/xml-format-spec.md` and `reference/developer-integration.md`.

## Retrieval Robustness (Wave 2)

- Manifest index initialization: initialize on first retrieval by loading/parsing `docs/rekordbox/manifest.yaml` into an in-memory index keyed by `topics`, `modes`, and `type`.
- Cache behavior: reuse the initialized index for all later retrievals in the same process to avoid repeated disk I/O/YAML parsing; never cache partial/failed initialization state.
- Determinism: retrieval operates against the cached index and should return stable ordering for identical inputs.
- Fallback on unavailable/malformed manifest: if `manifest.yaml` is missing, unreadable, or malformed, continue with best-effort guidance using the `Priority Consultation Order` below and XML reference anchors instead of hard failing.
- Error signaling: expose manifest health in response metadata/provenance (for example `manifest_status: unavailable` or `manifest_status: malformed`) so callers can distinguish fallback responses.

## Priority Consultation Order

- I need to understand the rekordbox UI
  - `manual/02-collection-window.md`
  - `manual/11-export-mode-screen.md`
  - `manual/18-performance-screen.md`
  - `manual/31-preferences.md`
  - `guides/introduction.md`
- I need to import/export XML
  - `reference/xml-import-export.md`
  - `guides/xml-format-spec.md`
  - `reference/developer-integration.md`
  - `manual/31-preferences.md`
  - `faq/library-and-collection.md`
- I need to manage the library
  - `manual/03-adding-tracks.md`
  - `manual/04-management.md`
  - `manual/06-searching.md`
  - `manual/07-playlists.md`
  - `faq/library-and-collection.md`
- I need to export to USB
  - `guides/usb-export.md`
  - `manual/12-export-preparing.md`
  - `manual/14-export-playing.md`
  - `faq/usb-and-devices.md`
- I need to understand preferences/settings
  - `manual/31-preferences.md`
  - `manual/30-info-window.md`
  - `guides/cloud-library-sync.md`
  - `guides/device-library-backup.md`

## Topic Cross-Reference

| Topic | Documents |
|---|---|
| analysis | `faq/cloud-and-sync.md`, `faq/library-and-collection.md`, `faq/troubleshooting.md`, `guides/phrase-edit.md`, `manual/03-adding-tracks.md`, `manual/08-intelligent-cue.md`, `manual/14-export-playing.md`, `manual/17-export-recording.md`, `manual/20-performance-playing.md`, `manual/31-preferences.md` |
| backup | `faq/usb-and-devices.md`, `features/cloud-setup-guide.md`, `guides/cloud-library-sync.md`, `guides/device-library-backup.md`, `manual/04-management.md` |
| beatgrid | `guides/edit-mode.md`, `manual/14-export-playing.md`, `manual/18-performance-screen.md`, `manual/20-performance-playing.md`, `manual/22-performance-effects.md`, `manual/23-sampler-deck.md`, `manual/25-slicer.md` |
| browsing | `guides/introduction.md`, `guides/keyboard-shortcuts.md`, `manual/02-collection-window.md`, `manual/06-searching.md`, `manual/09-collaborative-playlists.md`, `manual/11-export-mode-screen.md`, `manual/15-export-lan.md`, `manual/19-performance-preparing.md` |
| cloud | `faq/cloud-and-sync.md`, `features/cloud-setup-guide.md`, `features/overview.md`, `features/whats-new-v7.md`, `guides/cloud-direct-play.md`, `guides/cloud-library-sync.md`, `manual/10-mobile-devices.md`, `manual/31-preferences.md` |
| collaborative-playlists | `faq/library-and-collection.md`, `manual/09-collaborative-playlists.md` |
| collection | `faq/library-and-collection.md`, `features/overview.md`, `guides/cloud-library-sync.md`, `guides/introduction.md`, `guides/streaming-services.md`, `manual/02-collection-window.md`, `manual/03-adding-tracks.md`, `manual/04-management.md`, `manual/05-editing-track-info.md`, `manual/06-searching.md`, `manual/07-playlists.md`, `manual/11-export-mode-screen.md` |
| color | `faq/hardware-compatibility.md` |
| comments | - |
| compatibility | `faq/hardware-compatibility.md`, `faq/troubleshooting.md`, `guides/dvs-setup.md`, `guides/performance-mode-connection.md`, `guides/streaming-services.md` |
| connection | `guides/dvs-setup.md`, `guides/midi-learn.md`, `guides/performance-mode-connection.md`, `guides/pro-dj-link-setup.md`, `manual/10-mobile-devices.md`, `manual/13-export-pro-dj-link.md`, `manual/15-export-lan.md` |
| cue-points | `guides/edit-mode.md`, `guides/pad-editor.md`, `manual/08-intelligent-cue.md`, `manual/14-export-playing.md`, `manual/18-performance-screen.md`, `manual/20-performance-playing.md`, `manual/27-active-censor.md` |
| devices | `faq/usb-and-devices.md`, `guides/cloud-direct-play.md`, `guides/device-library-backup.md`, `guides/performance-mode-connection.md`, `guides/pro-dj-link-setup.md`, `guides/usb-export.md`, `manual/10-mobile-devices.md`, `manual/15-export-lan.md`, `manual/17-export-recording.md`, `manual/19-performance-preparing.md`, `manual/31-preferences.md` |
| dvs | `faq/hardware-compatibility.md`, `guides/dvs-setup.md`, `guides/performance-mode-connection.md`, `manual/31-preferences.md`, `manual/32-menu-list.md` |
| edit | `guides/edit-mode.md`, `guides/phrase-edit.md`, `manual/05-editing-track-info.md`, `manual/23-sampler-deck.md` |
| effects | `faq/stems-and-effects.md`, `features/overview.md`, `guides/lighting-mode.md`, `guides/video-function.md`, `manual/18-performance-screen.md`, `manual/22-performance-effects.md`, `manual/27-active-censor.md`, `manual/28-stems.md`, `manual/32-menu-list.md` |
| equipment | `faq/hardware-compatibility.md`, `faq/usb-and-devices.md`, `guides/dvs-setup.md`, `guides/midi-learn.md`, `guides/performance-mode-connection.md`, `guides/pro-dj-link-setup.md`, `manual/19-performance-preparing.md`, `manual/31-preferences.md` |
| export | `faq/cloud-and-sync.md`, `faq/hardware-compatibility.md`, `faq/library-and-collection.md`, `faq/usb-and-devices.md`, `features/overview.md`, `guides/device-library-backup.md`, `guides/introduction.md`, `guides/usb-export.md`, `guides/xml-format-spec.md`, `manual/07-playlists.md`, `manual/09-collaborative-playlists.md`, `manual/10-mobile-devices.md`, `manual/11-export-mode-screen.md`, `manual/12-export-preparing.md`, `manual/13-export-pro-dj-link.md`, `manual/14-export-playing.md`, `manual/15-export-lan.md`, `manual/16-export-mixing.md`, `manual/17-export-recording.md`, `manual/19-performance-preparing.md`, `reference/developer-integration.md`, `reference/xml-import-export.md` |
| file-formats | `faq/hardware-compatibility.md`, `manual/03-adding-tracks.md`, `manual/17-export-recording.md`, `manual/21-performance-recording.md`, `manual/33-appendix.md` |
| genre | `manual/06-searching.md` |
| history | `manual/03-adding-tracks.md`, `manual/14-export-playing.md` |
| hot-cue | `guides/edit-mode.md`, `guides/pad-editor.md`, `manual/08-intelligent-cue.md`, `manual/14-export-playing.md`, `manual/18-performance-screen.md`, `manual/20-performance-playing.md`, `manual/29-mix-point-link.md` |
| import | `faq/library-and-collection.md`, `guides/xml-format-spec.md`, `manual/03-adding-tracks.md`, `manual/07-playlists.md`, `manual/09-collaborative-playlists.md`, `reference/developer-integration.md`, `reference/xml-import-export.md` |
| interface | `features/overview.md`, `features/whats-new-v7.md`, `guides/edit-mode.md`, `guides/introduction.md`, `guides/keyboard-shortcuts.md`, `guides/lighting-mode.md`, `guides/pad-editor.md`, `guides/video-function.md`, `manual/01-introduction.md`, `manual/02-collection-window.md`, `manual/09-collaborative-playlists.md`, `manual/10-mobile-devices.md`, `manual/11-export-mode-screen.md`, `manual/16-export-mixing.md`, `manual/18-performance-screen.md`, `manual/30-info-window.md`, `manual/31-preferences.md`, `manual/32-menu-list.md`, `reference/glossary.md` |
| key | `manual/05-editing-track-info.md`, `manual/06-searching.md`, `manual/16-export-mixing.md`, `manual/20-performance-playing.md`, `manual/23-sampler-deck.md` |
| library | `faq/library-and-collection.md`, `features/cloud-setup-guide.md`, `features/overview.md`, `guides/cloud-library-sync.md`, `guides/device-library-backup.md`, `guides/introduction.md`, `guides/streaming-services.md`, `guides/usb-export.md`, `manual/02-collection-window.md`, `manual/03-adding-tracks.md`, `manual/04-management.md`, `reference/developer-integration.md`, `reference/glossary.md`, `reference/xml-import-export.md` |
| lighting | `faq/lighting-and-video.md`, `features/overview.md`, `guides/introduction.md`, `guides/lighting-mode.md`, `manual/32-menu-list.md` |
| link | `faq/troubleshooting.md`, `guides/pro-dj-link-setup.md`, `manual/13-export-pro-dj-link.md`, `manual/19-performance-preparing.md`, `manual/29-mix-point-link.md` |
| memory-cue | `guides/edit-mode.md`, `manual/08-intelligent-cue.md`, `manual/18-performance-screen.md`, `manual/29-mix-point-link.md` |
| metadata | `faq/library-and-collection.md`, `guides/xml-format-spec.md`, `manual/05-editing-track-info.md`, `reference/developer-integration.md`, `reference/glossary.md`, `reference/xml-import-export.md` |
| midi | `guides/lighting-mode.md`, `guides/midi-learn.md` |
| mixing | `guides/dvs-setup.md`, `guides/video-function.md`, `manual/15-export-lan.md`, `manual/16-export-mixing.md`, `manual/18-performance-screen.md`, `manual/19-performance-preparing.md`, `manual/20-performance-playing.md`, `manual/23-sampler-deck.md`, `manual/29-mix-point-link.md` |
| mobile | `faq/usb-and-devices.md`, `guides/cloud-library-sync.md`, `manual/10-mobile-devices.md` |
| onelibrary | `faq/cloud-and-sync.md`, `faq/hardware-compatibility.md`, `features/cloud-setup-guide.md`, `features/whats-new-v7.md`, `guides/cloud-library-sync.md` |
| pads | `guides/pad-editor.md`, `manual/18-performance-screen.md`, `manual/25-slicer.md`, `manual/26-capture.md`, `manual/27-active-censor.md` |
| performance | `features/overview.md`, `guides/dvs-setup.md`, `guides/introduction.md`, `guides/midi-learn.md`, `guides/pad-editor.md`, `guides/performance-mode-connection.md`, `manual/18-performance-screen.md`, `manual/21-performance-recording.md`, `manual/22-performance-effects.md` |
| phrase | `guides/edit-mode.md`, `guides/lighting-mode.md`, `guides/phrase-edit.md` |
| playback | `guides/cloud-direct-play.md`, `guides/keyboard-shortcuts.md`, `guides/streaming-services.md`, `guides/video-function.md`, `manual/14-export-playing.md`, `manual/18-performance-screen.md`, `manual/20-performance-playing.md`, `manual/23-sampler-deck.md`, `manual/29-mix-point-link.md` |
| playlists | `faq/library-and-collection.md`, `guides/cloud-library-sync.md`, `guides/usb-export.md`, `manual/06-searching.md`, `manual/07-playlists.md`, `manual/08-intelligent-cue.md`, `manual/09-collaborative-playlists.md`, `manual/11-export-mode-screen.md` |
| preferences | `guides/lighting-mode.md`, `guides/video-function.md`, `manual/08-intelligent-cue.md`, `manual/31-preferences.md`, `manual/32-menu-list.md` |
| pro-dj-link | `faq/usb-and-devices.md`, `guides/cloud-direct-play.md`, `guides/pro-dj-link-setup.md`, `manual/13-export-pro-dj-link.md`, `manual/15-export-lan.md` |
| rating | `manual/05-editing-track-info.md`, `manual/06-searching.md` |
| recording | `faq/stems-and-effects.md`, `manual/17-export-recording.md`, `manual/19-performance-preparing.md`, `manual/21-performance-recording.md` |
| sampler | `faq/stems-and-effects.md`, `manual/18-performance-screen.md`, `manual/23-sampler-deck.md`, `manual/24-sequencer.md`, `manual/25-slicer.md`, `manual/26-capture.md` |
| search | `manual/06-searching.md`, `manual/09-collaborative-playlists.md` |
| sequencer | `faq/stems-and-effects.md`, `manual/18-performance-screen.md`, `manual/19-performance-preparing.md`, `manual/24-sequencer.md` |
| slicer | `manual/18-performance-screen.md`, `manual/25-slicer.md`, `manual/26-capture.md` |
| stems | `faq/stems-and-effects.md`, `features/overview.md`, `features/whats-new-v7.md`, `manual/18-performance-screen.md`, `manual/28-stems.md`, `manual/31-preferences.md`, `manual/32-menu-list.md` |
| streaming | `faq/streaming.md`, `features/overview.md`, `features/whats-new-v7.md`, `guides/streaming-services.md` |
| subscription | `faq/plans-and-billing.md`, `features/whats-new-v7.md`, `guides/introduction.md`, `manual/01-introduction.md`, `manual/30-info-window.md`, `manual/31-preferences.md` |
| system-requirements | `guides/introduction.md`, `manual/33-appendix.md` |
| track-suggestion | `faq/library-and-collection.md`, `manual/06-searching.md` |
| usb | `faq/hardware-compatibility.md`, `faq/usb-and-devices.md`, `guides/cloud-direct-play.md`, `guides/device-library-backup.md`, `guides/usb-export.md`, `manual/09-collaborative-playlists.md`, `manual/13-export-pro-dj-link.md`, `manual/14-export-playing.md` |
| video | `faq/lighting-and-video.md`, `guides/video-function.md`, `manual/32-menu-list.md` |
| waveform | `faq/hardware-compatibility.md`, `guides/edit-mode.md`, `guides/phrase-edit.md`, `manual/11-export-mode-screen.md`, `manual/14-export-playing.md`, `manual/18-performance-screen.md`, `manual/19-performance-preparing.md` |
| xml | `guides/xml-format-spec.md`, `manual/03-adding-tracks.md`, `manual/09-collaborative-playlists.md`, `reference/developer-integration.md`, `reference/xml-import-export.md` |

## Mode Cross-Reference

| Mode | Documents |
|---|---|
| common | `faq/cloud-and-sync.md`, `faq/hardware-compatibility.md`, `faq/library-and-collection.md`, `faq/plans-and-billing.md`, `faq/stems-and-effects.md`, `faq/streaming.md`, `faq/troubleshooting.md`, `faq/usb-and-devices.md`, `features/cloud-setup-guide.md`, `features/overview.md`, `features/whats-new-v7.md`, `guides/cloud-direct-play.md`, `guides/cloud-library-sync.md`, `guides/device-library-backup.md`, `guides/introduction.md`, `guides/keyboard-shortcuts.md`, `guides/phrase-edit.md`, `guides/pro-dj-link-setup.md`, `guides/streaming-services.md`, `guides/xml-format-spec.md`, `manual/01-introduction.md`, `manual/02-collection-window.md`, `manual/03-adding-tracks.md`, `manual/04-management.md`, `manual/05-editing-track-info.md`, `manual/06-searching.md`, `manual/07-playlists.md`, `manual/08-intelligent-cue.md`, `manual/09-collaborative-playlists.md`, `manual/10-mobile-devices.md`, `reference/developer-integration.md`, `reference/glossary.md`, `reference/xml-import-export.md` |
| export | `features/overview.md`, `guides/cloud-direct-play.md`, `guides/device-library-backup.md`, `guides/introduction.md`, `guides/keyboard-shortcuts.md`, `guides/pro-dj-link-setup.md`, `guides/streaming-services.md`, `guides/usb-export.md`, `manual/06-searching.md`, `manual/09-collaborative-playlists.md`, `manual/10-mobile-devices.md`, `manual/11-export-mode-screen.md`, `manual/12-export-preparing.md`, `manual/13-export-pro-dj-link.md`, `manual/14-export-playing.md`, `manual/15-export-lan.md`, `manual/16-export-mixing.md`, `manual/17-export-recording.md`, `manual/30-info-window.md`, `manual/31-preferences.md`, `manual/32-menu-list.md`, `manual/33-appendix.md`, `reference/glossary.md`, `reference/xml-import-export.md` |
| performance | `features/overview.md`, `guides/dvs-setup.md`, `guides/introduction.md`, `guides/keyboard-shortcuts.md`, `guides/midi-learn.md`, `guides/pad-editor.md`, `guides/performance-mode-connection.md`, `guides/streaming-services.md`, `guides/video-function.md`, `manual/06-searching.md`, `manual/07-playlists.md`, `manual/18-performance-screen.md`, `manual/19-performance-preparing.md`, `manual/20-performance-playing.md`, `manual/21-performance-recording.md`, `manual/22-performance-effects.md`, `manual/23-sampler-deck.md`, `manual/24-sequencer.md`, `manual/25-slicer.md`, `manual/26-capture.md`, `manual/27-active-censor.md`, `manual/28-stems.md`, `manual/29-mix-point-link.md`, `manual/30-info-window.md`, `manual/31-preferences.md`, `manual/32-menu-list.md`, `manual/33-appendix.md`, `reference/glossary.md`, `reference/xml-import-export.md` |
| lighting | `faq/lighting-and-video.md`, `features/overview.md`, `guides/introduction.md`, `guides/lighting-mode.md`, `reference/glossary.md` |
| edit | `guides/edit-mode.md`, `reference/glossary.md` |

## Source Material Summary

| Type | Count | Notes |
|---|---:|---|
| Manual docs | 33 | rekordbox 7.2.8 instruction manual sections |
| Guide docs | 17 | operation guides (cloud, dvs, lighting, video, midi, usb, etc.) |
| FAQ docs | 9 | 288 Q&As categorized by operational topic |
| Feature docs | 3 | official web feature/update/setup pages |
| Reference docs | 3 | developer+xml, glossary, and xml workflow consolidation |
| Total corpus docs | 65 | excludes this README and manifest |

## Notes

- Manual page overlap warnings at section boundaries are intentional (pages 60, 140, 200, 242).
- Frontmatter taxonomy uses controlled vocab from `ORCHESTRATION-PLAN.md`.
- Run `bash docs/rekordbox/validate-corpus.sh` after bulk edits.
