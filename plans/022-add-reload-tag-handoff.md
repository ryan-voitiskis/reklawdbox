# Plan 022: Synchronize Collection Audit tag fixes before hydration

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- site/src/content/docs/workflows/library-cleanup.mdx site/src/content/docs/workflows/collection-audit.mdx site/src/partials/sops/collection-audit.mdx scripts/mcp-smoke.mjs docs/conventions.md src/cli/hydrate.rs
> ```
>
> If the current workflow already has a complete Reload Tag checkpoint, or if
> hydration no longer reads Rekordbox database metadata, stop and report rather
> than duplicating instructions.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: MED
- **Depends on**: 021
- **Category**: bug
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

Collection Audit writes corrected artist, title, album, and other tags to audio
files, but Rekordbox does not automatically copy those external changes into
`master.db`. The next cleanup session runs `reklawdbox hydrate`, which reads
artist/title/album from `master.db`; without **Reload Tag**, enrichment can use
the same stale values that Session 1 was meant to correct. The handoff must be
explicit, scoped to the changed imported tracks, and warn that Reload Tag can
replace intentional database-only edits.

## Current state

- `site/src/partials/sops/collection-audit.mdx` is the canonical agent SOP and
  is embedded into runtime `help()` with `include_str!`; changing it requires a
  Rust rebuild before an MCP host sees the new instructions.
- `site/src/content/docs/workflows/collection-audit.mdx` is the human workflow
  wrapper.
- `site/src/content/docs/workflows/library-cleanup.mdx` controls the Session 1
  to Session 2 checkpoint.
- `docs/conventions.md` is existing source-backed operational knowledge. It is
  evidence only in this plan.
- `src/cli/hydrate.rs` is runtime evidence only.

Current `site/src/content/docs/workflows/library-cleanup.mdx:37-43` promises
better enrichment after direct file-tag correction, then says only to
spot-check before Session 2:

```md
This fixes artist names, track titles, and file naming conventions. Getting
these right first means better enrichment match rates in Session 2.

No XML needed here — Collection Audit writes directly to files. Spot-check a
few corrected tracks in Rekordbox, then start a fresh session for Session 2.
```

Current `site/src/partials/sops/collection-audit.mdx:68-80` writes tags and
re-scans the files, but does not synchronize Rekordbox:

```md
write_file_tags(writes=[{path: "...", tags: {artist: "...", title: "..."}}])

audit_state(scan, scope="/path/")
```

Current `src/cli/hydrate.rs:388-413` obtains tracks from the Rekordbox database
and normalizes `track.artist`, `track.title`, and `track.album`. Its Discogs
request uses those database values again at lines 724-735.

The established manual contract is already recorded at
`docs/conventions.md:134-156`:

```md
**Workflow:** `write_file_tags` → select tracks in Rekordbox → Reload Tag.

After reloading, any previously edited track information is replaced with the
reloaded values.
```

Preserve the format caveats in that section: Reload Tag does not populate
every field for every audio format. This plan is about artist/title/album and
other fields Rekordbox actually reloads, not label/track-number repair for WAV.

## Commands you will need

| Purpose           | Command                                                                                                                                                                   | Expected on success                           |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- |
| SOP presence      | `rg -n "Reload Tag" site/src/partials/sops/collection-audit.mdx site/src/content/docs/workflows/collection-audit.mdx site/src/content/docs/workflows/library-cleanup.mdx` | exit 0; all three files match                 |
| Overwrite warning | `rg -n -e "replace" -e "overwrite" -e "database-only" site/src/partials/sops/collection-audit.mdx site/src/content/docs/workflows/library-cleanup.mdx`                    | exit 0; both agent and human checkpoints warn |
| Runtime build     | `cargo build --release`                                                                                                                                                   | exit 0; updated embedded SOP is compiled      |
| MCP smoke         | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`                                                                               | exit 0; no protocol violations                |
| Format            | `dprint check`                                                                                                                                                            | exit 0                                        |
| Site build        | `cd site && npm ci && npm run build`                                                                                                                                      | exit 0                                        |

## Suggested executor toolkit

- Use the Browser skill if available to inspect the Collection Audit and
  Library Cleanup pages at desktop and mobile widths.
- Do not automate the Rekordbox UI. The documentation must describe the
  manual checkpoint; live Rekordbox verification belongs to a separate
  operator-approved smoke test.

## Scope

**In scope** (the only source/documentation files you may modify):

- `site/src/partials/sops/collection-audit.mdx`
- `site/src/content/docs/workflows/collection-audit.mdx`
- `site/src/content/docs/workflows/library-cleanup.mdx`
- `scripts/mcp-smoke.mjs`
- `plans/README.md` for the status row only

**Out of scope**:

- Changing `write_file_tags`, `audit_state`, hydration, Rekordbox DB access, or
  XML export behavior.
- Adding direct writes to Rekordbox `master.db`.
- Claiming Reload Tag is safe for every changed field or file format.
- Bulk reloading an entire collection when only a subset changed.
- Editing `docs/conventions.md`; it is the accepted source for field support
  and overwrite semantics.
- Automating Rekordbox or requiring private local audio fixtures.

## Git workflow

- Branch: `codex/022-add-reload-tag-handoff`
- Use Conventional Commits; preferred final message:
  `docs(workflows): add Reload Tag checkpoint`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Make the agent retain a changed-track handoff list

In `site/src/partials/sops/collection-audit.mdx`, require the agent to retain
the path and successful field changes from every non-dry-run
`write_file_tags` result. Failed writes must not appear in the Reload Tag list.

For each successfully changed path, use existing read-only library lookup
(`search_tracks(path="...")`) to determine whether the file is imported into
Rekordbox. Deduplicate by track ID/path and preserve a short display list of
artist/title/path for the user. Files not imported into Rekordbox need no
Reload Tag action.

Do not add a new MCP tool or assume the audit issue payload always contains a
Rekordbox track ID.

**Verify**:

```bash
rg -n "successful|changed.*path|search_tracks\(path|imported" site/src/partials/sops/collection-audit.mdx
```

Expected: exit 0; the SOP distinguishes successful writes and imported files.

### Step 2: Add a blocking Rekordbox synchronization checkpoint

Insert a numbered SOP step after all file-tag fixes and their verification
scan, but before the final report/next-workflow recommendation. It must:

1. show the deduplicated imported-track list;
2. warn that Reload Tag replaces Rekordbox-side edits for reloaded fields;
3. ask the user to select only those tracks in Rekordbox and choose
   **Reload Tag**;
4. pause until the user confirms the action is complete;
5. re-read a bounded sample with `search_tracks` and compare artist/title/album
   against the successful file writes;
6. treat a mismatch as an incomplete handoff, not a clean completion.

Mention the WAV limitations by linking/summarizing the existing convention:
label and track number may remain DB-only and are not proof that Reload Tag
failed.

Renumber later SOP headings and any internal step references consistently.

**Verify**:

```bash
rg -n "Reload Tag|pause|search_tracks|incomplete|WAV" site/src/partials/sops/collection-audit.mdx
```

Expected: exit 0; all five concepts appear in the new step.

### Step 3: Align both human workflow checkpoints

In `site/src/content/docs/workflows/collection-audit.mdx`, add a concise
"After fixes" section explaining the manual handoff and overwrite risk.

In `site/src/content/docs/workflows/library-cleanup.mdx`, replace the Session 1
spot-check text with a hard checkpoint: changed imported tracks have been
reloaded and a bounded sample has been verified before Session 2. Keep the
statement that no XML is needed for Session 1.

Do not paste the whole agent SOP into the new explanation; keep it user-facing.

**Verify**:

```bash
rg -n "Reload Tag" site/src/content/docs/workflows/collection-audit.mdx site/src/content/docs/workflows/library-cleanup.mdx
```

Expected: each file has at least one match.

### Step 4: Rebuild both documentation surfaces

Because the partial is compiled into the binary, run both the site build and
release build. Extend the default DB-free MCP smoke so it calls
`help(topic="audit")`, asserts the returned text contains `Reload Tag`, and
reports an `auditHelp` topic/byte count beside the existing genre-help summary.
Keep `--skip-db` independent of both help calls.

**Verify**:

```bash
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
rg -n -e "topic: 'audit'" -e "Reload Tag" scripts/mcp-smoke.mjs
cd site && npm run build
```

Expected: all commands exit 0; the smoke output contains an `auditHelp` summary,
its internal `Reload Tag` assertion passes, and there are no protocol
violations.

## Test plan

- No production Rust logic changes are expected.
- Add no mocked audio/Rekordbox test; the regression is the embedded SOP text
  and built site.
- The DB-free MCP smoke is the executable oracle for
  `help(topic="audit") -> Reload Tag`; a genre-only help check is insufficient.
- Verify the three documentation surfaces with exact greps.
- Verify the release binary rebuild because `include_str!` freezes SOP text at
  compile time.
- Browser-check that the human warning is concise and not hidden below the raw
  SOP on both desktop and mobile.

## Done criteria

- [ ] The SOP retains only successful changed imported tracks for handoff.
- [ ] The user must confirm scoped Reload Tag before Session 2.
- [ ] The SOP verifies refreshed database-facing values after the manual action.
- [ ] Both human workflow pages explain the overwrite risk.
- [ ] WAV field limitations are not misrepresented.
- [ ] `dprint check` exits 0.
- [ ] `cargo build --release` exits 0.
- [ ] MCP smoke exits 0, reports `auditHelp`, and fails if audit help omits `Reload Tag`.
- [ ] `cd site && npm run build` exits 0.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- Hydration now reads current audio-file tags instead of Rekordbox DB metadata.
- A supported MCP tool can safely trigger Rekordbox Reload Tag and the product
  owner wants that automation instead of a manual checkpoint.
- The changed-track set cannot be derived without adding a new runtime API.
- Current Rekordbox documentation no longer supports Reload Tag or changes its
  overwrite semantics.
- A requested rewrite would imply that Reload Tag updates unsupported WAV
  fields.

## Maintenance notes

- Any future workflow that writes artist/title/album tags before a DB-backed
  operation needs the same synchronization checkpoint.
- Reviewers should check that verification reads Rekordbox metadata, not merely
  re-reads the audio file.
- Keep the changed-track list scoped; "reload the whole collection" is not an
  acceptable simplification because it increases overwrite risk.
- Plan 023 starts from this corrected Session 1/Session 2 boundary and must not
  remove it while rewriting provider prerequisites.
