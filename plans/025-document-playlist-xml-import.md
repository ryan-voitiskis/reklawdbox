# Plan 025: Document the playlist XML import path

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- site/src/partials/sops/set-building.mdx site/src/partials/sops/pool-building.mdx site/src/partials/sops/chapter-set-planning.mdx site/src/partials/sops/xml-import-steps.mdx site/src/content/docs/reference/xml-export.mdx src/xml.rs src/tools/help_handler.rs src/tools/tests.rs scripts/mcp-smoke.mjs
> ```
>
> Reconfirm the XML shape produced by the live exporter and the current
> Rekordbox import UI before editing. A changed playlist-export format or a
> current Rekordbox version that cannot import playlists from rekordbox XML is
> a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: 023
- **Category**: docs
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

Three set-building SOPs export playlists with `write_xml`, then tell users to
use a nonexistent **File → Import Collection** command. The supported metadata
path configures **Imported Library**, browses **rekordbox xml → All Tracks**,
and uses **Import To Collection**; it still does not bring an exported playlist
into Rekordbox's playlist tree. A user can complete a long curation session,
follow the wrong handoff, and still not see the planned playlist. The docs need
separate, named handoffs for metadata XML and playlist XML.

## Current state

- `site/src/partials/sops/set-building.mdx:136-144` writes a playlist XML and
  then says **File → Import Collection**.
- `site/src/partials/sops/pool-building.mdx:111-121` and
  `site/src/partials/sops/chapter-set-planning.mdx:145-158` repeat the same
  handoff.
- `site/src/partials/sops/xml-import-steps.mdx:5-9` is correctly written for
  importing staged track metadata: configure **Imported Library**, browse
  **rekordbox xml → All Tracks**, then use **Import To Collection**. Keep it
  canonical for that purpose.
- `site/src/content/docs/reference/xml-export.mdx` explains XML export but does
  not distinguish the two Rekordbox import paths.
- The repository's Rekordbox 7 manual snapshot documents playlist import from
  rekordbox XML at `docs/rekordbox/manual/32-menu-list.md:130-140` and the
  browser-tree/drag route at
  `docs/rekordbox/manual/09-collaborative-playlists.md:35-43`.
- `src/xml.rs` and the XML tests in `src/tools/tests.rs` are implementation
  evidence only. This plan does not change the exporter.
- SOP partials are embedded into MCP help through `include_str!`; their updated
  wording is not present in the runtime binary until it is rebuilt.
- `include_str!` returns raw MDX and does not execute nested Astro imports.
  Therefore simply rendering `<XmlPlaylistImportSteps />` in the SOP source
  would leave an unresolved component tag in runtime `help()`. This plan must
  compose that one shared partial into the returned help text and test all
  three affected topics.

The documentation must preserve this distinction:

| Export purpose           | Rekordbox handoff                                                                                                    |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------- |
| staged track metadata    | configure **Imported Library**, browse **rekordbox xml → All Tracks**, then use **Import To Collection**             |
| generated playlist/order | expand **rekordbox xml → Playlists** in the browser and import or drag the playlist into the Rekordbox playlist tree |

The executor must verify the exact labels against the currently supported
Rekordbox 7 UI and official/manual evidence. If labels vary by platform or
minor version, document both supported routes without inventing automation.

## Commands you will need

| Purpose                 | Command                                                                                                                                                             | Expected on success                                        |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------- |
| Find ambiguous handoffs | `rg -n -U "write_xml[\\s\\S]{0,1000}Import Collection" site/src/partials/sops/{set-building,pool-building,chapter-set-planning}.mdx`                                | exit 1 after the rewrite                                   |
| Confirm both handoffs   | `rg -n -e "Import To Collection" -e "rekordbox xml" -e "All Tracks" -e "Playlists" -e "drag" site/src/partials/sops site/src/content/docs/reference/xml-export.mdx` | exit 0; metadata and playlist routes are clearly separated |
| Embedded SOP build      | `cargo build --release`                                                                                                                                             | exit 0                                                     |
| Help composition tests  | `cargo test -p reklawdbox playlist_import_help_contract -- --nocapture`                                                                                             | exit 0; three topics contain expanded instructions         |
| MCP smoke               | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`                                                                         | exit 0; three playlist-help assertions pass without a DB   |
| Docs build              | `cd site && npm run build`                                                                                                                                          | exit 0                                                     |
| Format                  | `dprint check`                                                                                                                                                      | exit 0                                                     |

## Scope

**In scope**:

- `site/src/partials/sops/xml-playlist-import-steps.mdx` — create
- `site/src/partials/sops/set-building.mdx`
- `site/src/partials/sops/pool-building.mdx`
- `site/src/partials/sops/chapter-set-planning.mdx`
- `site/src/content/docs/reference/xml-export.mdx`
- `src/tools/help_handler.rs` for one narrow shared-partial expansion helper
- `src/tools/tests.rs` for DB-free help composition assertions
- `scripts/mcp-smoke.mjs` for real stdio assertions on the three affected help
  topics
- `plans/README.md` for the status row only

**Out of scope**:

- Changing XML serialization, playlist ordering, track matching, or
  `write_xml` parameters.
- A general MDX renderer or broad help-system refactor; runtime composition is
  limited to the one explicit playlist-import component tag.
- Changing the existing metadata-only `xml-import-steps.mdx` partial.
- Automating the Rekordbox GUI or writing to `master.db`.
- Claiming that a playlist import also applies every staged metadata field.
- Claiming that XML import merges safely without user review.
- Broad workflow-page rewrites reserved for Plans 026 and 031.

## Git workflow

- Branch: `codex/025-document-playlist-xml-import`
- Preferred commit: `docs(xml): document playlist import handoff`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Verify the current export and import contracts

Inspect the playlist branch of `write_xml`, its tests, and the current local
manual snapshots. If internet access is available, verify the UI terms against
official Rekordbox 7 documentation. Record evidence in the PR/commit notes, not
as speculative prose in the site.

Confirm that the three SOPs create a named playlist under the XML playlist
tree, and that the metadata partial remains appropriate for exports containing
only staged track fields.

**Verify**:

```bash
rg -n "playlist|PLAYLISTS|NODE" src/xml.rs src/tools/tests.rs
rg -n "Import Playlist|rekordbox xml|Playlists" docs/rekordbox/manual
```

Expected: implementation/test evidence for playlist nodes and manual evidence
for a playlist-specific import route.

### Step 2: Create one canonical playlist-import partial

Create `site/src/partials/sops/xml-playlist-import-steps.mdx`. It must tell the
user to:

1. save/locate the XML produced by `write_xml`;
2. expose **rekordbox xml** in Rekordbox's browser tree if it is hidden;
3. expand its **Playlists** node;
4. preview the generated playlist name and order;
5. import or drag only that playlist into the desired Rekordbox playlist
   location;
6. verify count, first/last tracks, and representative order before using it;
7. keep the XML until the verification passes.

Include a short note that **Import To Collection** from **rekordbox xml → All
Tracks** is the path for staged track metadata, not the primary playlist
handoff described here. Do not state that importing a playlist writes staged
metadata.

**Verify**:

```bash
rg -n "rekordbox xml|All Tracks|Playlists|drag|track count|Import To Collection" site/src/partials/sops/xml-playlist-import-steps.mdx
```

Expected: exit 0; the playlist route, verification, and distinction all
appear.

### Step 3: Use the partial in all three playlist-producing SOPs

Import and render the new partial after the successful `write_xml` step in:

- Set Building;
- Pool Building;
- Chapter Set Planning.

Remove the local **File → Import Collection** playlist instructions. Preserve
each workflow's approval checkpoint, output filename, playlist name, and
post-import verification. Do not copy the steps into three separate prose
blocks.

Because runtime help reads these files with `include_str!`, add a narrow helper
in `src/tools/help_handler.rs` that requires and removes the exact
`import XmlPlaylistImportSteps ...` line, then replaces the exact
`<XmlPlaylistImportSteps />` tag with the `include_str!` contents of the shared
partial before returning Set, Pool, or Chapter help. Do not build a general MDX
renderer. A missing/duplicate import or component marker in any of the three
source SOPs must fail a focused test rather than silently return incomplete
help. Runtime help must contain neither the raw import nor unresolved component
tag after expansion.

Because these SOPs are model instructions, make the agent stop after export and
hand control to the user for the Rekordbox UI step. The agent must not report
the playlist as installed before the user confirms verification.

**Verify**:

```bash
for file in site/src/partials/sops/{set-building,pool-building,chapter-set-planning}.mdx; do
  test "$(rg -c "XmlPlaylistImportSteps" "$file")" -eq 2
done
rg -n -U "write_xml[\s\S]{0,1000}File.*Import Collection" site/src/partials/sops/{set-building,pool-building,chapter-set-planning}.mdx
cargo test -p reklawdbox playlist_import_help_contract -- --nocapture
```

Expected: every SOP has one import plus one render, the stale File-menu grep
exits 1, and DB-free help tests prove `set`, `pool`, and `chapter` each contain
the expanded `rekordbox xml`, `Playlists`, drag/import, and verification text
with no raw `import XmlPlaylistImportSteps` line or unresolved component tag.

### Step 4: Split the XML reference into metadata and playlist handoffs

In `site/src/content/docs/reference/xml-export.mdx`, add a small decision table
or two titled procedures. Explain what the user exported, which Rekordbox UI
path applies, and what to verify. Link to the relevant workflows without
duplicating their full SOPs.

Keep the existing backup, staging, and field-support claims unchanged unless
drift shows they are factually wrong; such drift is a STOP condition and should
be routed to the relevant plan rather than silently expanded here.

**Verify**:

```bash
rg -n -e "Track metadata" -e "Playlist" -e "Import To Collection" -e "All Tracks" -e "rekordbox xml" site/src/content/docs/reference/xml-export.mdx
```

Expected: exit 0; the reference has separately titled metadata and playlist
paths and names both distinct Rekordbox handoffs.

### Step 5: Rebuild embedded help and the site

Run the release build so `include_str!` captures the new SOP text. Extend the
DB-free MCP smoke to call `help(topic='set')`, `help(topic='pool')`, and
`help(topic='chapter')`; for each, assert the returned text contains the shared
playlist import route and contains no unresolved component tag. Report a compact
`playlistImportHelp` topic/byte summary. Each live assertion must also reject
the raw import line. Then build the site. Inspect the
rendered XML reference and one agent SOP route at desktop and mobile widths.
Confirm the two import procedures cannot be mistaken for one another and that
the ordered list is readable.

**Verify**:

```bash
cargo test -p reklawdbox xml -- --nocapture
cargo test -p reklawdbox playlist_import_help_contract -- --nocapture
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
rg -n -e "playlistImportHelp" -e "XmlPlaylistImportSteps" scripts/mcp-smoke.mjs
dprint check
cd site && npm run build
test -f dist/reference/xml-export/index.html
```

Expected: every command exits 0; existing XML tests pass, the three live help
topics contain expanded instructions and no unresolved tag, and the rendered
reference route exists. Record the desktop/mobile inspection result in the
review handoff.

## Test plan

- No XML/export behavior changes are allowed. The only Rust production change
  is narrow model-facing help composition; existing XML tests remain the
  exporter regression oracle and must not be rewritten to make documentation
  pass.
- Static checks must prove all three playlist SOPs share the new partial and no
  longer pair `write_xml` with the metadata import route.
- Focused Rust tests and real stdio smoke must prove the shared partial is
  expanded in all three compile-time embedded help topics; a raw component tag
  is a failure.
- Build and visually inspect the Starlight site.

## Done criteria

- [ ] Metadata and playlist XML imports are described as different operations.
- [ ] All three playlist-producing SOPs use one canonical import partial.
- [ ] Runtime help expands that partial for Set, Pool, and Chapter topics.
- [ ] The user verifies playlist count and order before completion is claimed.
- [ ] The metadata import partial remains unchanged and canonical for staged fields.
- [ ] Existing XML tests, release build, MCP smoke, `dprint check`, and site build pass.
- [ ] Desktop and mobile rendering pass for the XML reference and one agent SOP.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- The live exporter no longer emits playlist nodes or order.
- Supported Rekordbox 7 documentation no longer exposes a playlist XML import
  path.
- The exact UI varies in a way that cannot be documented without a tested
  platform/version qualification.
- A correct handoff would require changing `write_xml` or automating Rekordbox.
- The help handler can no longer compose one explicit shared fragment without a
  general renderer or broader runtime-help redesign.
- Existing XML tests fail before the documentation change.
- Any in-scope file materially differs from the recorded excerpts.

## Maintenance notes

- Keep one partial for metadata import and one for playlist import; do not merge
  them into an ambiguous universal procedure.
- Any new playlist-producing workflow must use the playlist partial.
- Reverify Rekordbox UI labels when the supported major version changes.
- SOP changes require a rebuilt/released binary before MCP hosts receive them.
