# Rekordbox Knowledge Corpus — Orchestration Plan

## Goal

Create a comprehensive, agent-traversable knowledge corpus covering everything a DJ needs to know about rekordbox 7.x. Following the manifest-driven architecture pattern used in the legal and security corpuses.

## Source Material Inventory

### PDFs (20 files, 1,696 total pages)

| # | File | Pages | Chunks (20pp max) | Notes |
|---|------|-------|-------------------|-------|
| 1 | `rekordbox7.2.8_manual_EN.pdf` | 259 | 13 | Core manual |
| 2 | `rekordbox7.0.7_introduction_EN.pdf` | 27 | 2 | Install & setup |
| 3 | `rekordbox7.2.8_cloud_library_sync_operation_guide_EN.pdf` | 36 | 2 | |
| 4 | `rekordbox7.2.2_CloudDirectPlay_EN.pdf` | 24 | 2 | |
| 5 | `rekordbox7.0.7_lighting_operation_guide_EN.pdf` | 71 | 4 | |
| 6 | `rekordbox-lighting-available-fixtures.pdf` | 785 | SKIP | DMX fixture catalogue — too large, low value |
| 7 | `rekordbox7.0.5_Phrase_Edit_operation_guide_EN.pdf` | 8 | 1 | |
| 8 | `rekordbox6.1.1_edit_operation_guide_EN.pdf` | 17 | 1 | v6 doc, still current |
| 9 | `rekordbox7.0.5_video_operation_guide_EN.pdf` | 26 | 2 | |
| 10 | `rekordbox7.0.5_dvs_setup_guide_EN.pdf` | 19 | 1 | |
| 11 | `rekordbox7.2.10_streaming_service_usage_guide_EN.pdf` | 37 | 2 | |
| 12 | `rekordbox7.0.5_midi_learn_operation_guide_EN.pdf` | 8 | 1 | |
| 13 | `rekordbox7.2.8_pad_editor_operation_guide_EN.pdf` | 14 | 1 | |
| 14 | `rekordbox7.0.5_default_keyboard_shortcut_reference_EN.pdf` | 10 | 1 | |
| 15 | `rekordbox6.7.4_device_library_backup_guide_EN.pdf` | 13 | 1 | v6 doc, still current |
| 16 | `USB_export_guide_en_251007.pdf` | 5 | 1 | Infographic-style |
| 17 | `PRODJLINK_SetupGuide_ver2_en.pdf` | 17 | 1 | |
| 18 | `rekordbox5.3.0_connection_guide_for_performance_mode_EN.pdf` | 17 | 1 | v5 doc, still current |
| 19 | `xml_format_list.pdf` | 1 | 1 | Single-page XML schema reference |
| 20 | `rekordbox7.0.0_manual_EN.pdf` | 250 | SKIP | Superseded by 7.2.8 |

**Skipped:** #6 (785pp fixture catalogue — low-value repetitive data) and #20 (superseded)
**Active total:** 18 PDFs, ~611 pages, ~38 chunks

### FAQ (1 file, 288 Q&As, already scraped)

The complete rekordbox 7 FAQ has been scraped to a single file:

| File | Q&As | Lines | Size |
|------|------|-------|------|
| `docs/rekordbox/rekordbox7-faq.md` | 288 | 3,525 | 138KB |

Source: `https://rekordbox.com/en/support/faq/rekordbox7/` — this is the superset of all 12 individual FAQ category pages. Extracted via curl + cheerio Node.js script.

Earlier per-category raw text files in `source-material/faq/` are now redundant (subsets of this file).

During Phase A, FAQ agents will split this single file into topic-specific documents with YAML frontmatter, rather than fetching web pages.

### Web Pages (4 pages)

| # | URL | Content |
|---|-----|---------|
| W1 | `rekordbox.com/en/feature/overview/` | Feature overview |
| W2 | `rekordbox.com/en/cloud-setup-guide/` | Cloud setup steps |
| W3 | `rekordbox.com/en/support/developer/` | XML Bridge import |
| W4 | `rekordbox.com/en/2024/05/introducing-rekordbox-ver-7/` | What's new in v7 |

---

## Output Structure

```
docs/rekordbox/
├── manifest.yaml                    # Primary entry point for agents
├── README.md                        # Consultation guide + cross-references
├── source-material/                 # Raw sources (PDFs git-ignored)
│   ├── *.pdf                        # 20 PDFs
│   └── download-sources.sh
├── rekordbox7-faq.md                # Raw scraped FAQ (288 Q&As, input for faq/ split)
├── manual/                          # Verbatim transcriptions of instruction manual
│   ├── 01-introduction.md           # pp 1-6
│   ├── 02-collection-window.md      # pp 7-12
│   ├── 03-adding-tracks.md          # pp 13-20
│   ├── 04-management.md             # pp 21-24
│   ├── 05-editing-track-info.md     # pp 25-26
│   ├── 06-searching.md              # pp 27-38
│   ├── 07-playlists.md              # pp 39-40
│   ├── 08-intelligent-cue.md        # pp 41-45
│   ├── 09-collaborative-playlists.md # pp 46-57
│   ├── 10-mobile-devices.md         # pp 58-59
│   ├── 11-export-mode-screen.md     # pp 60-81
│   ├── 12-export-preparing.md       # pp 82-86
│   ├── 13-export-pro-dj-link.md     # pp 83-86 (or wherever this ends)
│   ├── 14-export-playing.md         # pp 87-108
│   ├── 15-export-lan.md             # pp 109-113
│   ├── 16-export-mixing.md          # pp 114-117
│   ├── 17-export-recording.md       # pp 118-120
│   ├── 18-performance-screen.md     # pp 121-143
│   ├── 19-performance-preparing.md  # pp 144-149
│   ├── 20-performance-playing.md    # pp 150-172
│   ├── 21-performance-recording.md  # pp 173-175
│   ├── 22-performance-effects.md    # pp 176-187
│   ├── 23-sampler-deck.md           # pp 188-192
│   ├── 24-sequencer.md              # pp 193-196
│   ├── 25-slicer.md                 # pp 197-199
│   ├── 26-capture.md                # pp 200-202
│   ├── 27-active-censor.md          # pp 203-206
│   ├── 28-stems.md                  # pp 207-209
│   ├── 29-mix-point-link.md         # pp 210-221
│   ├── 30-info-window.md            # pp 222-223 (approx)
│   ├── 31-preferences.md            # pp ~224-242
│   ├── 32-menu-list.md              # pp ~243-248
│   └── 33-appendix.md               # pp ~249-256 (system req, formats, etc.)
├── guides/                          # Transcriptions of operation guides
│   ├── introduction.md
│   ├── cloud-library-sync.md
│   ├── cloud-direct-play.md
│   ├── lighting-mode.md
│   ├── phrase-edit.md
│   ├── edit-mode.md
│   ├── video-function.md
│   ├── dvs-setup.md
│   ├── streaming-services.md
│   ├── midi-learn.md
│   ├── pad-editor.md
│   ├── keyboard-shortcuts.md
│   ├── device-library-backup.md
│   ├── usb-export.md
│   ├── pro-dj-link-setup.md
│   ├── performance-mode-connection.md
│   └── xml-format-spec.md
├── faq/                             # Categorized from rekordbox7-faq.md (288 Q&As)
│   ├── plans-and-billing.md
│   ├── streaming.md
│   ├── hardware-compatibility.md
│   ├── library-and-collection.md
│   ├── cloud-and-sync.md
│   ├── stems-and-effects.md
│   ├── usb-and-devices.md
│   ├── lighting-and-video.md
│   └── troubleshooting.md
├── features/                        # Web page transcriptions
│   ├── overview.md
│   ├── whats-new-v7.md
│   └── cloud-setup-guide.md
└── reference/                       # Cross-cutting reference material
    ├── xml-import-export.md         # Consolidated XML pipeline knowledge
    ├── glossary.md                  # All rekordbox terminology
    └── developer-integration.md     # Bridge/XML developer info
```

---

## YAML Frontmatter Schema

Every output markdown file gets this frontmatter:

```yaml
---
id: collection-window                  # Unique slug
title: "About the [Collection] Window" # Human title
type: manual | guide | faq | feature | reference
source:
  file: rekordbox7.2.8_manual_EN.pdf   # Source PDF/URL
  pages: "7-12"                        # Page range (PDFs) or URL
  version: "7.2.8"                     # Software version documented
topics:                                # Controlled vocabulary (see below)
  - collection
  - browsing
  - interface
modes:                                 # Which rekordbox mode(s) this covers
  - common                             # common | export | performance | lighting
features: []                           # Specific features covered
confidence: pending                    # pending | verified | high
last_verified: null                    # Set during Phase B
transcribed_by: agent                  # agent | human
verified_by: null                      # Set during Phase B
---
```

### Controlled Vocabulary — Topics

```
analysis, backup, beatgrid, browsing, cloud, collection, color,
collaborative-playlists, comments, compatibility, connection, cue-points,
devices, dvs, edit, effects, equipment, export, file-formats, genre,
history, hot-cue, import, interface, key, library, lighting, link,
memory-cue, metadata, midi, mixing, mobile, onelibrary, pads, performance,
phrase, playlists, playback, preferences, pro-dj-link, rating, recording,
sampler, search, sequencer, slicer, stems, streaming, subscription,
system-requirements, track-suggestion, usb, video, waveform, xml
```

### Controlled Vocabulary — Modes

```
common, export, performance, lighting, edit
```

---

## Manifest Schema

```yaml
# manifest.yaml
schema_version: 1
corpus: rekordbox
description: "Comprehensive rekordbox 7.x knowledge corpus"
software_version: "7.2.8"
last_updated: "2026-02-17"
source_documents:
  pdfs: 19
  faq_pages: 12
  web_pages: 4

taxonomy:
  topics: [analysis, backup, beatgrid, ...]  # Full list above
  modes: [common, export, performance, lighting, edit]
  types: [manual, guide, faq, feature, reference]

documents:
  - id: collection-window
    title: "About the [Collection] Window"
    type: manual
    path: manual/02-collection-window.md
    topics: [collection, browsing, interface]
    modes: [common]
    confidence: verified
  # ... one entry per document
```

---

## Phase A: Transcription

### Goal
Convert all source material to clean markdown with YAML frontmatter.

### Approach
The orchestrator spawns parallel agents, each assigned a specific source document or page range. Each agent:

1. Reads the assigned PDF pages (max 20pp per Read call) or fetches the web page
2. Writes markdown preserving:
   - All headings and hierarchy
   - All lists (bulleted and numbered)
   - All tables (converted to markdown tables)
   - All UI element names in brackets: `[Collection]`, `[BPM]`
   - All figure/screenshot references as `[Screenshot: description]`
   - All cross-references to other manual sections as `→ See [section-id]`
3. Adds YAML frontmatter with `confidence: pending`
4. Does NOT summarize, interpret, or omit content — verbatim transcription

### Agent Assignments

#### Manual Chunks (13 agents, ~20pp each)

```
Agent M1:  pages 1-20    → 01-introduction.md, 02-collection-window.md, 03-adding-tracks.md
Agent M2:  pages 21-40   → 04-management.md, 05-editing-track-info.md, 06-searching.md, 07-playlists.md
Agent M3:  pages 41-60   → 08-intelligent-cue.md, 09-collaborative-playlists.md, 10-mobile-devices.md
Agent M4:  pages 60-80   → 11-export-mode-screen.md
Agent M5:  pages 80-100  → 12-export-preparing.md, 13-export-pro-dj-link.md, 14-export-playing.md (start)
Agent M6:  pages 100-120 → 14-export-playing.md (end), 15-export-lan.md, 16-export-mixing.md, 17-export-recording.md
Agent M7:  pages 121-140 → 18-performance-screen.md
Agent M8:  pages 140-160 → 19-performance-preparing.md, 20-performance-playing.md (start)
Agent M9:  pages 160-180 → 20-performance-playing.md (end), 21-performance-recording.md, 22-performance-effects.md
Agent M10: pages 180-200 → 22-performance-effects.md (end), 23-sampler-deck.md, 24-sequencer.md, 25-slicer.md
Agent M11: pages 200-222 → 26-capture.md, 27-active-censor.md, 28-stems.md, 29-mix-point-link.md
Agent M12: pages 222-242 → 30-info-window.md, 31-preferences.md
Agent M13: pages 242-256 → 32-menu-list.md, 33-appendix.md
```

#### Guide Agents (17 agents, one per guide PDF — fixture list skipped)

```
Agent G1:  introduction.md              (27pp, 2 reads)
Agent G2:  cloud-library-sync.md        (36pp, 2 reads)
Agent G3:  cloud-direct-play.md         (24pp, 2 reads)
Agent G4:  lighting-mode.md             (71pp, 4 reads)
Agent G5:  phrase-edit.md               (8pp, 1 read)
Agent G6:  edit-mode.md                 (17pp, 1 read)
Agent G7:  video-function.md            (26pp, 2 reads)
Agent G8:  dvs-setup.md                 (19pp, 1 read)
Agent G9:  streaming-services.md        (37pp, 2 reads)
Agent G10: midi-learn.md                (8pp, 1 read)
Agent G11: pad-editor.md               (14pp, 1 read)
Agent G12: keyboard-shortcuts.md        (10pp, 1 read)
Agent G13: device-library-backup.md     (13pp, 1 read)
Agent G14: usb-export.md               (5pp, 1 read)
Agent G15: pro-dj-link-setup.md        (17pp, 1 read)
Agent G16: performance-mode-connection.md (17pp, 1 read)
Agent G17: xml-format-spec.md          (1pp, 1 read)
```

#### FAQ Agents (3 agents — split & categorize existing mega-file)

The full FAQ (288 items) is already in `docs/rekordbox/rekordbox7-faq.md`. These agents read it and split into topic-specific files with YAML frontmatter.

```
Agent F1: Read rekordbox7-faq.md lines 1-1200, categorize Q&As, write:
          faq/plans-and-billing.md, faq/streaming.md, faq/hardware-compatibility.md
Agent F2: Read rekordbox7-faq.md lines 1200-2400, categorize Q&As, write:
          faq/library-and-collection.md, faq/cloud-and-sync.md, faq/stems-and-effects.md
Agent F3: Read rekordbox7-faq.md lines 2400-3525, categorize Q&As, write:
          faq/usb-and-devices.md, faq/lighting-and-video.md, faq/troubleshooting.md
```

Each agent assigns every Q&A to exactly one output file based on topic. The target categories:

| Output File | Topics Covered |
|---|---|
| `faq/plans-and-billing.md` | Subscription, trials, hardware unlock, owner registration, accounts |
| `faq/streaming.md` | TIDAL, Apple Music, Spotify, SoundCloud, Beatport, Beatsource |
| `faq/hardware-compatibility.md` | Compatible equipment, waveform colors, file formats, exFAT, DVS |
| `faq/library-and-collection.md` | Library conversion, migration, metadata, search, playlists, XML |
| `faq/cloud-and-sync.md` | Cloud Library Sync, CloudDirectPlay, Dropbox, collaborative playlists |
| `faq/stems-and-effects.md` | STEMS, GROOVE CIRCUIT, DRUM CAPTURE/SWAP, effects |
| `faq/usb-and-devices.md` | USB export, OneLibrary, Device Library, play history, mobile |
| `faq/lighting-and-video.md` | LIGHTING mode, DMX, video function, fixtures |
| `faq/troubleshooting.md` | Operation hints, errors, analysis, Ableton Link, misc |

#### Web Page Agents (4 agents)

```
Agent W1: features/overview.md           (WebFetch)
Agent W2: features/whats-new-v7.md       (WebFetch)
Agent W3: features/cloud-setup-guide.md  (WebFetch)
Agent W4: reference/developer-integration.md (WebFetch)
```

### Total Phase A: ~37 agents

**Parallelism constraint:** Claude Code allows multiple concurrent Task agents. The orchestrator should batch these in waves of ~8-10 to avoid overload. Suggested waves:

- **Wave 1:** M1-M6 + G5, G10, G12, G17 (10 agents — small/fast guides mixed with manual chunks)
- **Wave 2:** M7-M13 + G6, G8, G13 (10 agents)
- **Wave 3:** G1, G2, G3, G4, G7, G9, G11, G14-G16 (10 agents — remaining guides)
- **Wave 4:** F1-F3 + W1-W4 (7 agents — FAQ split + web pages, all fast)

---

## Phase B: Verification

### Goal
Each transcription is verified against its source by a separate agent.

### Approach
One verification agent per document. Each agent:

1. Reads the original source (PDF pages or web page)
2. Reads the transcription markdown
3. Checks:
   - **Completeness:** No missing sections, headings, or content
   - **Accuracy:** No hallucinated content, correct numbers/tables
   - **Structure:** Headings match source hierarchy
   - **Formatting:** Tables render correctly, lists preserved
   - **Cross-references:** Section links point to valid IDs
4. If errors found: edits the markdown to correct them
5. Updates frontmatter: `confidence: verified`, `last_verified: 2026-02-17`, `verified_by: agent`

### Agent Assignments

One agent per output document (~37 agents total — matching Phase A). Can be batched in the same wave structure.

**Critical rule:** A verification agent MUST NOT be the same agent instance that transcribed the document. The orchestrator must ensure fresh agents for verification.

---

## Phase C: Organization & Mapping

### Goal
Build the manifest, README, cross-references, glossary, and reference documents.

### Agents (5 total, sequential)

```
Agent C1: Build manifest.yaml
          - Read all transcribed documents (frontmatter only)
          - Generate complete manifest with all document entries
          - Validate all paths, IDs, and taxonomy terms

Agent C2: Build README.md
          - Priority consultation guide (ordered by typical use case)
          - Cross-reference table: topic → documents
          - Mode → documents mapping
          - Quick-start guide for agents

Agent C3: Build reference/glossary.md
          - Extract all bracketed terms from all documents
          - Define each term
          - Cross-reference to source documents

Agent C4: Build reference/xml-import-export.md
          - Consolidate XML knowledge from:
            - Manual (File menu, Preferences > Advanced > Bridge)
            - xml-format-spec.md
            - developer-integration.md
            - FAQ operation hints (auto-export)
            - Existing docs/rekordbox-internals.md
          - Unified guide for XML pipeline operations

Agent C5: Add cross-reference links
          - Scan all documents for "See [section]" references
          - Convert to relative markdown links
          - Add "Related documents" section to each file
```

---

## Orchestrator Instructions

The orchestrator agent should be spawned with the following instructions:

1. **Read this plan** (`docs/rekordbox/ORCHESTRATION-PLAN.md`)
2. **Create task list** tracking all agents across phases
3. **Execute Phase A** in waves, spawning agents with the Task tool
4. **Wait for each wave** to complete before starting the next
5. **Execute Phase B** after all Phase A agents finish
6. **Execute Phase C** after all Phase B agents finish
7. **Final validation:** Read manifest.yaml, spot-check 3 random documents

### Agent Prompt Template — Transcription (Phase A, PDF)

```
You are transcribing official rekordbox documentation to markdown.

SOURCE: {pdf_path}
PAGES: {start_page}-{end_page}
OUTPUT: {output_path}

Rules:
1. Read the PDF pages using the Read tool (max 20 pages per call)
2. Transcribe VERBATIM to markdown. Do not summarize or omit.
3. Preserve all headings as markdown headings (# ## ### etc.)
4. Convert all tables to markdown tables
5. Preserve all lists (bulleted and numbered)
6. UI elements in brackets: [Collection], [BPM], [Preferences]
7. Screenshots/figures: [Screenshot: brief description of what it shows]
8. Cross-references: → See {section-name} (page {N})
9. Add YAML frontmatter (template provided below)
10. Write the file using the Write tool

Frontmatter template:
---
id: {id}
title: "{title}"
type: {type}
source:
  file: {source_file}
  pages: "{pages}"
  version: "{version}"
topics: [{topics}]
modes: [{modes}]
confidence: pending
last_verified: null
transcribed_by: agent
verified_by: null
---
```

### Agent Prompt Template — Verification (Phase B)

```
You are verifying a transcription of official rekordbox documentation.

SOURCE: {pdf_path} pages {start_page}-{end_page}
TRANSCRIPTION: {md_path}

Steps:
1. Read the source PDF pages
2. Read the transcription markdown
3. Compare section by section:
   - Are all headings present and correctly leveled?
   - Are all paragraphs present? (spot-check first/last sentence of each)
   - Are all tables correct? (check row/column counts, spot-check values)
   - Are all lists complete?
   - Are there any hallucinated additions not in the source?
4. If corrections needed: use Edit tool to fix the markdown
5. Update frontmatter: set confidence to "verified", last_verified to "2026-02-17", verified_by to "agent"
6. Report: "VERIFIED" or "CORRECTED: {list of changes}"
```

---

## Estimated Effort

| Phase | Agents | Est. Time per Agent | Total Wall Clock (parallelized) |
|-------|--------|--------------------|---------------------------------|
| A: Transcription | ~37 | 2-5 min | ~20 min (4 waves) |
| B: Verification | ~37 | 1-3 min | ~12 min (4 waves) |
| C: Organization | 5 | 3-5 min | ~15 min (sequential) |
| **Total** | **~79** | | **~47 min** |

---

## Notes

- The `rekordbox7.0.0_manual_EN.pdf` is superseded by `rekordbox7.2.8_manual_EN.pdf` — skip it
- The `rekordbox-lighting-available-fixtures.pdf` is 785 pages of DMX fixture model tables — skip it (low value for DJ knowledge corpus, can be referenced directly if needed)
- Some guides are older versions (6.1.1, 6.7.4, 5.3.0) but still current for those features — transcribe as-is, note version in frontmatter
- `USB_export_guide_en_251007.pdf` is only 5 pages of infographic-style content — may need description of visuals
- `xml_format_list.pdf` is a single page — the XML element/attribute reference sheet
- FAQ pages with 403 errors during research may need user to provide content
- Web pages can be fetched by agents using WebFetch tool
