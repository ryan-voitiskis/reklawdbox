# Plan 021: State the real safety boundary and use scoped host permissions

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- README.md site/src/content/docs/index.mdx site/src/content/docs/getting-started/index.mdx site/src/content/docs/concepts/index.mdx site/src/content/docs/concepts/safety.mdx
> ```
>
> If any in-scope file changed, compare every safety and permission claim below
> with the live text before proceeding. A changed runtime mutation boundary is
> a STOP condition.

## Status

- **Priority**: P1
- **Effort**: S
- **Risk**: LOW
- **Depends on**: none
- **Category**: docs
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

The public onboarding currently treats read-only access to Rekordbox
`master.db` as proof that disabling the MCP host's permission checks is safe.
That is not the product's actual boundary: tag and artwork tools can mutate
audio files, XML export writes files and runs a backup, setup writes host
configuration, and the internal store persists cache and workflow state. The
documentation must distinguish enforced database safety from direct file
operations and procedural agent approvals so users can grant informed,
appropriately scoped authority.

## Current state

- `site/src/content/docs/index.mdx` is the public splash/onboarding page.
- `site/src/content/docs/getting-started/index.mdx` is the detailed install and
  host setup guide.
- `site/src/content/docs/concepts/safety.mdx` is the canonical public safety
  explanation.
- `site/src/content/docs/concepts/index.mdx` summarizes the architecture for
  nontechnical readers.
- `README.md` repeats the architectural boundary for repository visitors.
- Runtime files cited below are evidence only and are out of scope.

Current `site/src/content/docs/index.mdx:41-47` recommends a host-wide bypass
and calls it necessary:

```md
3. **Start a conversation**

   `cd ~/Music && claude --dangerously-skip-permissions`

   The `--dangerously-skip-permissions` flag is needed for multi-step workflows.
```

Current `site/src/content/docs/getting-started/index.mdx:57-65` says the bypass
is safe solely because the database is read-only and metadata changes use XML.

Current `site/src/content/docs/concepts/safety.mdx:20-24,36-49` says nothing is
written to disk, every step requires sign-off, and XML is the only output.

The implementation has a narrower guarantee:

- `src/db.rs:15-20` opens `master.db` with `SQLITE_OPEN_READ_ONLY`. Preserve
  this as the strongest enforced safety property. Do not reproduce the
  database key from that file in documentation or tests.
- `src/tools/mod.rs:663-686` exposes direct audio-tag and artwork writes.
- `src/tools/file_tag_handlers.rs:135-154` makes `write_file_tags` use
  `dry_run=false` by default; the write branch begins at line 198.
- `site/src/content/docs/workflows/library-cleanup.mdx:15-17` already states
  that Collection Audit writes directly to audio files. Use this wording as a
  consistency anchor, but do not edit that file in this plan.
- `site/src/content/docs/mcp-tools/files-system.mdx:35-50` accurately documents
  direct tag writes and the default. Preserve that contract.

`README.md:86-95` correctly says there is no write path to `master.db`, but its
phrasing "No write path exists in the codebase" and "all proposed changes"
overgeneralizes beyond the database/staged-metadata path.

Use this vocabulary consistently:

1. **Rekordbox database** — technically enforced read-only access.
2. **Staged Rekordbox metadata** — `update_tracks` changes live in
   `ChangeManager` until XML export or discard.
3. **Direct file operations** — tag and artwork tools can change audio files;
   workflows must preview and request approval where their SOP says so.
4. **Local application state** — cache, audit state, setup/configuration,
   backups, and XML exports write outside `master.db`.
5. **Host permission prompts** — an independent defense controlled by the MCP
   host; agent workflow approval is procedural, not a universal runtime gate.

## Commands you will need

| Purpose                         | Command                                                                                                                                                                                                            | Expected on success                                |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------- |
| Find unsafe blanket claims      | `rg -n -e "dangerously-skip-permissions" -e "only output is XML" -e "Nothing is written to disk" -e "Every step requires your sign-off" -e "No write path exists in the codebase" README.md site/src/content/docs` | exit 1 after the rewrite; no matches               |
| Confirm direct-write disclosure | `rg -n -e "audio files" -e "direct file" -e "internal store" -e "permission" site/src/content/docs/concepts/safety.mdx`                                                                                            | exit 0; all five safety layers are represented     |
| Format                          | `dprint check`                                                                                                                                                                                                     | exit 0                                             |
| Build docs                      | `cd site && npm ci && npm run build`                                                                                                                                                                               | exit 0; all Starlight routes and LLM outputs build |

## Suggested executor toolkit

- Use the Browser skill if available to inspect the built homepage, Install,
  Concepts, and Safety pages at desktop and mobile widths.
- Use the current official Claude Code CLI documentation only to link to host
  permission configuration. Do not freeze an unverified third-party allowlist
  syntax into the site.

## Scope

**In scope** (the only source/documentation files you may modify):

- `README.md`
- `site/src/content/docs/index.mdx`
- `site/src/content/docs/getting-started/index.mdx`
- `site/src/content/docs/concepts/index.mdx`
- `site/src/content/docs/concepts/safety.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Any change to `master.db` access, `ChangeManager`, file-tag tools, artwork
  tools, setup behavior, backup behavior, or tool defaults.
- Adding a new runtime permission or confirmation framework.
- Changing workflow SOPs; Plans 022–025 correct specific workflow contracts.
- Claiming that XML import cannot overwrite existing metadata or that file
  operations are reversible.
- Recommending a blanket host permission bypass under a different spelling.
- Copying any credential, database key, token, or secret value into docs.

## Git workflow

- Branch: `codex/021-correct-safety-permission-guidance`
- Use Conventional Commits; preferred final message:
  `docs(safety): clarify permission and write boundaries`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Establish the layered safety model on the canonical page

Rewrite `site/src/content/docs/concepts/safety.mdx` around the five vocabulary
layers above. Keep the read-only `master.db` guarantee prominent and precise.
Add a compact table with columns such as operation, destination, enforcement,
and user checkpoint, covering at minimum:

- reading `master.db`;
- staged metadata updates;
- XML export and automatic backup;
- audio tag writes;
- artwork extraction/embedding;
- enrichment/audio cache writes;
- setup/configuration writes.

State explicitly that workflow approval prompts are SOP behavior and do not
replace host permissions. Say which operations are direct and which can be
previewed. Do not imply that every direct write has rollback support.

**Verify**:

```bash
rg -n "master\.db|staged|audio files|artwork|cache|backup|host permission" site/src/content/docs/concepts/safety.mdx
```

Expected: exit 0; each safety layer appears in a specific section or table row.

### Step 2: Remove blanket permission bypasses from onboarding

In `site/src/content/docs/index.mdx` and
`site/src/content/docs/getting-started/index.mdx`:

- use the normal host launch path without `--dangerously-skip-permissions`;
- explain that the host may ask for tool approval;
- recommend granting only the permissions needed for the chosen workflow;
- link to the Safety page for the operation matrix;
- present any reduction of host prompts as an explicit host-specific choice,
  not a requirement and not something made safe by the read-only DB.

Keep the existing install flow usable. Do not perform the larger first-session
redesign reserved for Plan 030.

**Verify**:

```bash
rg -n "dangerously-skip-permissions|needed for multi-step|This is safe here" site/src/content/docs
```

Expected: exit 1; no matches.

### Step 3: Align the overview and README without weakening the DB guarantee

Update `site/src/content/docs/concepts/index.mdx`, the homepage's "Safe by
design" section, and `README.md:84-95` so they say:

- no code path writes to Rekordbox `master.db`;
- proposed Rekordbox metadata edits use staging and XML;
- separate file-management tools can write audio tags/artwork;
- local cache/config/export files are expected application outputs.

Do not turn the overview into a second full Safety page. Use one sentence plus
a direct link for detail.

**Verify**:

```bash
rg -n "No write path exists in the codebase|only output is XML|nothing is written" README.md site/src/content/docs/index.mdx site/src/content/docs/concepts
```

Expected: exit 1; no overbroad claims remain.

### Step 4: Build and inspect the rendered pages

Run the docs build, serve `site/dist` locally, and inspect:

- `/`
- `/getting-started/`
- `/concepts/`
- `/concepts/safety/`

At desktop and mobile widths confirm the safety table does not overflow, the
normal launch command is copyable, caution text is readable, and all Safety
links resolve.

**Verify**:

```bash
cd site && npm run build
test -f dist/index.html
test -f dist/getting-started/index.html
test -f dist/concepts/safety/index.html
```

Expected: every command exits 0.

## Test plan

- No Rust test is required because runtime behavior is intentionally unchanged.
- The regression oracle is the exact zero-match grep for the blanket bypass and
  overbroad output claims.
- The Astro production build must succeed.
- Browser QA must cover desktop and mobile because the new operation matrix is
  the only layout-sensitive change.

## Done criteria

- [ ] No `--dangerously-skip-permissions` recommendation remains in the public docs.
- [ ] The Safety page distinguishes database, staging, direct files, local state, and host permissions.
- [ ] `README.md` says no write path exists to `master.db`, not to the whole codebase.
- [ ] `rg -n "only output is XML|Nothing is written to disk|Every step requires your sign-off" README.md site/src/content/docs` returns no matches.
- [ ] `dprint check` exits 0.
- [ ] `cd site && npm run build` exits 0.
- [ ] Desktop and mobile rendered checks pass for the four named routes.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- `master.db` is no longer opened read-only in the live source.
- Direct tag/artwork operations have gained a runtime confirmation layer that
  materially changes the operation matrix.
- Current host documentation no longer supports a normal prompted mode.
- Accurate wording appears to require promising rollback for audio-file writes.
- An in-scope file has been substantially redesigned since `3451803` and the
  excerpts no longer describe it.

## Maintenance notes

- Reviewers should reject wording that uses the read-only database to justify
  unrelated shell or filesystem authority.
- Any future direct-write tool must be added to the Safety operation matrix.
- If a host-specific scoped-permission example is added later, test it against
  that host's current CLI and date/version the example.
- Plans 026 and 030 should reuse this plan's mutation vocabulary rather than
  inventing new labels.
