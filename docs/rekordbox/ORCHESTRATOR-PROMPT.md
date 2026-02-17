# Orchestrator Prompt — Rekordbox Knowledge Corpus

Paste this into a new Claude Code session to start the pipeline.

---

## Prompt

You are orchestrating the creation of a comprehensive rekordbox 7.x knowledge corpus. Your job is to spawn subagents in waves, track their progress, and ensure quality across three phases.

**Read the plan first:**

```
Read docs/rekordbox/ORCHESTRATION-PLAN.md
```

All source material is already downloaded in `docs/rekordbox/source-material/` (18 active PDFs, ~611 pages). The complete FAQ (288 items) is at `docs/rekordbox/rekordbox7-faq.md`. No web fetching is needed for PDFs — they're local files.

---

### PHASE A: Transcription

Create output directories first:

```bash
mkdir -p docs/rekordbox/{manual,guides,faq,features,reference}
```

Then spawn agents in 4 waves. Use `subagent_type: "general-purpose"` for all agents. Run each wave's agents in parallel using multiple Task tool calls in a single message. Wait for each wave to complete before starting the next.

#### Wave 1 (10 agents)

**M1** — Manual pages 1-20:
```
Transcribe the rekordbox 7.2.8 instruction manual to markdown.

SOURCE: /Users/vz/projects/crate-dig/docs/rekordbox/source-material/rekordbox7.2.8_manual_EN.pdf
PAGES: 1-20

Read pages 1-20 using the Read tool. Transcribe VERBATIM to markdown — do not summarize or omit any content.

Write SEPARATE files for each major section, splitting at the major headings visible in the content:
- docs/rekordbox/manual/01-introduction.md (pages up to ~6 — "Introduction" + "[INFO] window")
- docs/rekordbox/manual/02-collection-window.md (pages ~7-12 — "About the [Collection] window")
- docs/rekordbox/manual/03-adding-tracks.md (pages ~13-20 — "Adding a track")

Rules:
- Preserve ALL headings as markdown headings (# ## ### etc matching the PDF hierarchy)
- Convert ALL tables to markdown tables
- Preserve ALL lists (bulleted and numbered)
- Keep UI element names in brackets exactly as they appear: [Collection], [BPM], [Preferences]
- For screenshots/figures write: [Screenshot: brief description of what the image shows]
- For cross-references write: → See {section-name} (page {N})
- Do NOT add commentary, interpretation, or summaries

Add this YAML frontmatter to EACH file (adjust id, title, pages, topics per file):
---
id: {slug}
title: "{title from the PDF heading}"
type: manual
source:
  file: rekordbox7.2.8_manual_EN.pdf
  pages: "{page range}"
  version: "7.2.8"
topics: [{relevant topics from: analysis, backup, beatgrid, browsing, cloud, collection, color, collaborative-playlists, comments, compatibility, connection, cue-points, devices, dvs, edit, effects, equipment, export, file-formats, genre, history, hot-cue, import, interface, key, library, lighting, link, memory-cue, metadata, midi, mixing, mobile, onelibrary, pads, performance, phrase, playlists, playback, preferences, pro-dj-link, rating, recording, sampler, search, sequencer, slicer, stems, streaming, subscription, system-requirements, track-suggestion, usb, video, waveform, xml}]
modes: [{common, export, performance, lighting, edit — whichever apply}]
confidence: pending
last_verified: null
transcribed_by: agent
verified_by: null
---
```

**M2** — Manual pages 21-40. Same instructions as M1 but:
- PAGES: 21-40
- Output files: `04-management.md`, `05-editing-track-info.md`, `06-searching.md`, `07-playlists.md`

**M3** — Manual pages 41-60. Same pattern:
- PAGES: 41-60
- Output files: `08-intelligent-cue.md`, `09-collaborative-playlists.md`, `10-mobile-devices.md`

**M4** — Manual pages 60-80:
- PAGES: 60-80
- Output files: `11-export-mode-screen.md`

**M5** — Manual pages 80-100:
- PAGES: 80-100
- Output files: `12-export-preparing.md`, `13-export-pro-dj-link.md`, `14-export-playing.md` (start — agent should note where content ends for next agent)

**M6** — Manual pages 100-120:
- PAGES: 100-120
- Output files: `14-export-playing.md` (continue/complete), `15-export-lan.md`, `16-export-mixing.md`, `17-export-recording.md`
- NOTE: If `14-export-playing.md` was already started by M5, append to it or create the complete version here.

**G5** — Phrase Edit guide:
```
Transcribe the rekordbox Phrase Edit operation guide to markdown.

SOURCE: /Users/vz/projects/crate-dig/docs/rekordbox/source-material/rekordbox7.0.5_Phrase_Edit_operation_guide_EN.pdf
Read ALL pages (8 total). Transcribe VERBATIM.
OUTPUT: docs/rekordbox/guides/phrase-edit.md

[Same rules as manual agents above]

Frontmatter: type: guide, source.version: "7.0.5"
```

**G10** — MIDI Learn guide (8pp):
- SOURCE: `rekordbox7.0.5_midi_learn_operation_guide_EN.pdf`
- OUTPUT: `docs/rekordbox/guides/midi-learn.md`

**G12** — Keyboard Shortcuts (10pp):
- SOURCE: `rekordbox7.0.5_default_keyboard_shortcut_reference_EN.pdf`
- OUTPUT: `docs/rekordbox/guides/keyboard-shortcuts.md`

**G17** — XML Format Spec (1pp):
- SOURCE: `xml_format_list.pdf`
- OUTPUT: `docs/rekordbox/guides/xml-format-spec.md`

#### Wave 2 (10 agents)

**M7** — Manual pages 121-140 → `18-performance-screen.md`
**M8** — Manual pages 140-160 → `19-performance-preparing.md`, `20-performance-playing.md` (start)
**M9** — Manual pages 160-180 → `20-performance-playing.md` (end), `21-performance-recording.md`, `22-performance-effects.md`
**M10** — Manual pages 180-200 → `22-performance-effects.md` (end), `23-sampler-deck.md`, `24-sequencer.md`, `25-slicer.md`
**M11** — Manual pages 200-222 → `26-capture.md`, `27-active-censor.md`, `28-stems.md`, `29-mix-point-link.md`
**M12** — Manual pages 222-242 → `30-info-window.md`, `31-preferences.md`
**M13** — Manual pages 242-259 → `32-menu-list.md`, `33-appendix.md`
**G6** — Edit Mode (17pp) → `docs/rekordbox/guides/edit-mode.md` (source: `rekordbox6.1.1_edit_operation_guide_EN.pdf`, version: "6.1.1")
**G8** — DVS Setup (19pp) → `docs/rekordbox/guides/dvs-setup.md` (source: `rekordbox7.0.5_dvs_setup_guide_EN.pdf`)
**G13** — Device Library Backup (13pp) → `docs/rekordbox/guides/device-library-backup.md` (source: `rekordbox6.7.4_device_library_backup_guide_EN.pdf`, version: "6.7.4")

#### Wave 3 (10 agents)

**G1** — Introduction (27pp, read in 2 batches: 1-20 then 21-27) → `docs/rekordbox/guides/introduction.md` (source: `rekordbox7.0.7_introduction_EN.pdf`, version: "7.0.7")
**G2** — Cloud Library Sync (36pp, read 1-20 then 21-36) → `docs/rekordbox/guides/cloud-library-sync.md` (source: `rekordbox7.2.8_cloud_library_sync_operation_guide_EN.pdf`)
**G3** — CloudDirectPlay (24pp, read 1-20 then 21-24) → `docs/rekordbox/guides/cloud-direct-play.md` (source: `rekordbox7.2.2_CloudDirectPlay_EN.pdf`, version: "7.2.2")
**G4** — Lighting Mode (71pp, read in 4 batches: 1-20, 21-40, 41-60, 61-71) → `docs/rekordbox/guides/lighting-mode.md` (source: `rekordbox7.0.7_lighting_operation_guide_EN.pdf`, version: "7.0.7")
**G7** — Video Function (26pp, read 1-20 then 21-26) → `docs/rekordbox/guides/video-function.md` (source: `rekordbox7.0.5_video_operation_guide_EN.pdf`)
**G9** — Streaming Services (37pp, read 1-20 then 21-37) → `docs/rekordbox/guides/streaming-services.md` (source: `rekordbox7.2.10_streaming_service_usage_guide_EN.pdf`, version: "7.2.10")
**G11** — Pad Editor (14pp) → `docs/rekordbox/guides/pad-editor.md` (source: `rekordbox7.2.8_pad_editor_operation_guide_EN.pdf`)
**G14** — USB Export (5pp) → `docs/rekordbox/guides/usb-export.md` (source: `USB_export_guide_en_251007.pdf`. NOTE: This is infographic-style — describe visual elements as `[Infographic: description]`)
**G15** — PRO DJ LINK Setup (17pp) → `docs/rekordbox/guides/pro-dj-link-setup.md` (source: `PRODJLINK_SetupGuide_ver2_en.pdf`)
**G16** — Performance Mode Connection (17pp) → `docs/rekordbox/guides/performance-mode-connection.md` (source: `rekordbox5.3.0_connection_guide_for_performance_mode_EN.pdf`, version: "5.3.0")

#### Wave 4 (7 agents)

**F1** — FAQ split (lines 1-1200):
```
You are splitting the rekordbox FAQ into topic-specific files.

Read docs/rekordbox/rekordbox7-faq.md lines 1-1200.

For each Q&A (delimited by --- separators), assign it to ONE of these output files based on topic:
- docs/rekordbox/faq/plans-and-billing.md (subscription, trials, hardware unlock, owner registration, accounts, payments)
- docs/rekordbox/faq/streaming.md (TIDAL, Apple Music, Spotify, SoundCloud, Beatport, Beatsource, streaming)
- docs/rekordbox/faq/hardware-compatibility.md (compatible equipment, waveform colors, file formats, exFAT, DVS hardware)

Write each output file with YAML frontmatter:
---
id: faq-{slug}
title: "FAQ: {Category Name}"
type: faq
source:
  file: rekordbox7-faq.md
  url: "https://rekordbox.com/en/support/faq/rekordbox7/"
  version: "7.x"
topics: [{relevant topics}]
modes: [common]
confidence: pending
last_verified: null
transcribed_by: agent
verified_by: null
---

Preserve the Q&A format exactly: ### Question heading, answer text, --- separator.
If a Q&A doesn't fit any of your 3 categories, append it to a temporary "uncategorized" section at the end of plans-and-billing.md — the next agent will pick it up.
```

**F2** — FAQ split (lines 1200-2400):
Same approach, output files: `faq/library-and-collection.md`, `faq/cloud-and-sync.md`, `faq/stems-and-effects.md`

**F3** — FAQ split (lines 2400-3525):
Same approach, output files: `faq/usb-and-devices.md`, `faq/lighting-and-video.md`, `faq/troubleshooting.md`
Also: check for any "uncategorized" sections left by F1/F2 and redistribute them.

**W1** — Feature overview:
```
Fetch https://rekordbox.com/en/feature/overview/ using WebFetch.
Transcribe ALL content to docs/rekordbox/features/overview.md with YAML frontmatter (type: feature).
```

**W2** — What's new in v7:
```
Fetch https://rekordbox.com/en/2024/05/introducing-rekordbox-ver-7/ using WebFetch.
Transcribe to docs/rekordbox/features/whats-new-v7.md (type: feature).
```

**W3** — Cloud setup guide:
```
Fetch https://rekordbox.com/en/cloud-setup-guide/ using WebFetch.
Transcribe to docs/rekordbox/features/cloud-setup-guide.md (type: guide).
```

**W4** — Developer/XML integration:
```
Fetch https://rekordbox.com/en/support/developer/ using WebFetch.
Transcribe to docs/rekordbox/reference/developer-integration.md (type: reference).
```

---

### PHASE B: Verification

After ALL Phase A agents complete, spawn verification agents in the same wave structure. Each verification agent gets a fresh context (do NOT resume Phase A agents).

For each output file, spawn an agent with this prompt template:

```
You are verifying a transcription of official rekordbox documentation.

SOURCE: {source_pdf_or_url}
PAGES: {page_range} (if PDF)
TRANSCRIPTION: {output_md_path}

Steps:
1. Read the source material (PDF pages via Read tool, or original FAQ/web content)
2. Read the transcription markdown file
3. Compare section by section:
   - Are ALL headings present and correctly leveled?
   - Are ALL paragraphs present? Spot-check first and last sentence of each section.
   - Are ALL tables correct? Check row counts, column counts, spot-check 3 values.
   - Are ALL list items present?
   - Is there any HALLUCINATED content not in the source?
   - Is the YAML frontmatter complete and the topics/modes accurate?
4. If corrections needed: use the Edit tool to fix the markdown file directly.
5. Update the YAML frontmatter:
   - Set confidence to "verified"
   - Set last_verified to "2026-02-17"
   - Set verified_by to "agent"
6. Report your findings as: "VERIFIED" or "CORRECTED: {brief list of what was fixed}"
```

Batch verification agents the same way as transcription (4 waves of ~10).

For FAQ files, the verification agent should read the original `rekordbox7-faq.md` and confirm that every Q&A appears in exactly one categorized file and none were dropped.

---

### PHASE C: Organization & Mapping

After ALL Phase B agents complete, run these 5 agents SEQUENTIALLY (each depends on the prior).

**C1 — Build manifest.yaml:**
```
Read the YAML frontmatter of every .md file in docs/rekordbox/{manual,guides,faq,features,reference}/ using Grep for the frontmatter blocks.

Generate docs/rekordbox/manifest.yaml following this schema:

schema_version: 1
corpus: rekordbox
description: "Comprehensive rekordbox 7.x knowledge corpus"
software_version: "7.2.8"
last_updated: "2026-02-17"
source_documents:
  pdfs: 18
  faq_items: 288
  web_pages: 4

taxonomy:
  topics: [analysis, backup, beatgrid, browsing, cloud, collection, color, collaborative-playlists, comments, compatibility, connection, cue-points, devices, dvs, edit, effects, equipment, export, file-formats, genre, history, hot-cue, import, interface, key, library, lighting, link, memory-cue, metadata, midi, mixing, mobile, onelibrary, pads, performance, phrase, playlists, playback, preferences, pro-dj-link, rating, recording, sampler, search, sequencer, slicer, stems, streaming, subscription, system-requirements, track-suggestion, usb, video, waveform, xml]
  modes: [common, export, performance, lighting, edit]
  types: [manual, guide, faq, feature, reference]

documents:
  - id: {from frontmatter}
    title: {from frontmatter}
    type: {from frontmatter}
    path: {relative path from docs/rekordbox/}
    topics: {from frontmatter}
    modes: {from frontmatter}
    confidence: {from frontmatter}

Validate: every path exists, every id is unique, every topic/mode is from the controlled vocabulary.
```

**C2 — Build README.md:**
```
Read docs/rekordbox/manifest.yaml.

Generate docs/rekordbox/README.md containing:

1. Title and overview (what this corpus covers, software version)
2. How an agent should use this corpus (manifest-first discovery pattern)
3. Priority consultation order for common tasks:
   - "I need to understand the rekordbox UI" → manual/02-collection-window.md, manual/11-export-mode-screen.md, etc.
   - "I need to import/export XML" → reference/xml-import-export.md, guides/xml-format-spec.md
   - "I need to manage the library" → manual/03-adding-tracks.md, manual/04-management.md, etc.
   - "I need to export to USB" → guides/usb-export.md, faq/usb-and-devices.md
   - "I need to understand preferences/settings" → manual/31-preferences.md
4. Cross-reference table: topic → list of document paths
5. Mode → list of document paths
6. Source material summary
```

**C3 — Build glossary:**
```
Grep all .md files in docs/rekordbox/{manual,guides,faq,features}/ for terms in square brackets (e.g. [Collection], [BPM], [STEMS]).

Generate docs/rekordbox/reference/glossary.md with:
- YAML frontmatter (id: glossary, type: reference)
- Alphabetical list of every unique bracketed term
- Brief definition based on context from the corpus
- Which documents reference it (as relative links)
```

**C4 — Build XML import/export reference:**
```
Read these files and consolidate XML knowledge into a single reference:
- docs/rekordbox/guides/xml-format-spec.md
- docs/rekordbox/reference/developer-integration.md
- docs/rekordbox/manual/31-preferences.md (the Bridge/XML sections)
- docs/rekordbox/faq/library-and-collection.md (any XML-related Q&As)
- docs/rekordbox-internals.md (existing project knowledge)

Generate docs/rekordbox/reference/xml-import-export.md with:
- YAML frontmatter (id: xml-import-export, type: reference)
- Complete guide to XML operations in rekordbox:
  - Exporting collection as XML (File menu)
  - Auto-export settings (Preferences > Advanced > Others)
  - Importing via Bridge pane (Preferences > Bridge > Imported Library)
  - XML format specification (elements, attributes)
  - Known issues and workarounds (reimport bug, etc.)
- Cross-references to source documents
```

**C5 — Add cross-reference links:**
```
Scan every .md file in docs/rekordbox/{manual,guides,faq,features,reference}/ for:
- "See {section}" or "→ See" references
- Mentions of other document topics

For each document, add a "## Related Documents" section at the bottom with relative markdown links to related documents (based on shared topics from manifest.yaml).

Use the Edit tool to append to each file. Do not modify existing content.
```

---

### Final Validation

After Phase C completes:
1. Read `docs/rekordbox/manifest.yaml` — verify it parses correctly and all paths resolve
2. Randomly select 3 documents from different types (manual, guide, faq) and read them to confirm they have proper frontmatter, verified confidence, and sensible content
3. Report total document count, verified count, and any issues found

---

### Error Handling

- If an agent fails or times out, note the failure and continue with the rest of the wave. Re-run failed agents after the wave completes.
- If a PDF read fails (corrupted page, image-only page), have the agent note `[Unreadable: page N]` and continue.
- If two agents write to the same file (overlap on manual sections), the later wave's agent should read what exists and append/merge rather than overwrite.
- Report all issues to the user at phase boundaries.
