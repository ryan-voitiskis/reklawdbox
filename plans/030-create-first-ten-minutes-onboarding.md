# Plan 030: Create a bounded first-ten-minutes onboarding journey

> **Executor instructions**: Follow this plan step by step and verify the built
> production journey, not only source text. Stop on any STOP condition instead
> of adding providers or mutation to make the demo impressive. Update this
> plan's row in `plans/README.md` only if the orchestrator/reviewer does not own
> the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- Cargo.toml site/src/content/docs/index.mdx site/src/content/docs/getting-started site/src/components site/src/data/workflows.mjs site/astro.config.mjs scripts/check-doc-contract.mjs scripts/check-doc-contract.test.mjs scripts/release.sh .github/workflows/docs-pages.yml src/tools/library_handlers.rs src/types.rs src/tools/mod.rs
> ```
>
> Plans 021, 023, 026, and 027 must be integrated. Reconfirm `read_library` is still
> a no-parameter, read-only, network-free first action and that the canonical
> workflow records are usable. If a bounded first result requires a new tool or
> writable/provider operation, stop.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 021, 023, 026, 027
- **Category**: direction / onboarding / UX
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

The current homepage moves from setup straight into a large Library Cleanup
workflow. That path can write audio tags, require provider credentials, analyze
many files, and run for hours. A new user has not yet seen proof that the server
can read their library or learned which workflow matches their goal.

The onboarding journey should separate installation, connection, a safe first
result, and discovery. The first connected session must be useful without
network access, cache coverage, audio analysis, staged changes, file writes, or
XML export.

## Current state

- `site/src/content/docs/index.mdx:35-59` combines setup/launch with Library
  Cleanup as the universal starting point. Plan 021 removes the blanket host
  permission bypass, but does not redesign the journey.
- `site/src/content/docs/getting-started/index.mdx:69-77` already uses the right
  bounded idea: show a library summary with track count, playlists, and genres.
- The same page currently sends every successful install directly to a
  five-session cleanup workflow.
- `src/tools/mod.rs:277-283` exposes `read_library` with no user selector.
- `src/tools/library_handlers.rs:79-84` reads library statistics, and
  `src/types.rs:222-232` defines total tracks, genre distribution, playlists,
  average BPM, and key distribution in the result.
- Plan 026 supplies accurate workflow impact/side-effect/prerequisite/handoff facts for a
  choice surface. Consume those records rather than duplicating them.

The phrase “first ten minutes” means the first connected session after a
supported installation succeeds. Do not promise that package installation,
source compilation, host restart, or troubleshooting completes in ten minutes.

## Commands you will need

| Purpose           | Command                                                                                                                                                                     | Expected on success                          |
| ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| Contract gate     | `node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist`                                                                                  | exit 0; first-session structural checks pass |
| Old universal CTA | `rg -n -e "recommended starting point for every new user" -e "Start here: Library Cleanup" site/src/content/docs/index.mdx site/src/content/docs/getting-started/index.mdx` | exit 1                                       |
| Route             | `test -f site/dist/getting-started/first-session/index.html`                                                                                                                | exit 0 after build                           |
| Build             | `cd site && npm run build`                                                                                                                                                  | exit 0                                       |
| Format            | `dprint check`                                                                                                                                                              | exit 0                                       |

## Scope

**In scope**:

- `site/src/content/docs/index.mdx`
- `site/src/content/docs/getting-started/index.mdx`
- `site/src/content/docs/getting-started/first-session.mdx` — create
- `site/src/components/GoalChooser.astro` — create
- `site/src/data/workflows.mjs` only if a presentation-neutral intent field is
  required; preserve Plan 026's canonical contract and validator
- `site/astro.config.mjs`
- `site/src/styles/custom.css` only if existing Starlight tokens are inadequate
- `scripts/check-doc-contract.mjs`
- `scripts/check-doc-contract.test.mjs`
- `.github/workflows/docs-pages.yml` and `scripts/release.sh` only to enforce
  `Cargo.toml` in the same docs-contract trigger set as the homepage version
  sentinel while preserving Plan 027's `Cargo.lock` dependency trigger
- `plans/README.md` for the status row only

**Out of scope**:

- Changing setup, MCP host configuration, permission enforcement, or
  `read_library` runtime behavior.
- Adding Discogs authorization, provider lookup, hydration, audio analysis,
  cache writes, staging, file mutation, or XML to the first-session prompt.
- Rewriting detailed workflow pages or implementing onboarding state/telemetry.
- Claiming every host has the same setup mechanism.
- Promising installation or a full collection scan completes within ten
  minutes.

## Git workflow

- Branch: `codex/030-create-first-ten-minutes-onboarding`
- Preferred commit: `docs(onboarding): add a bounded first session`.
- Do not push or open a PR unless explicitly instructed.

## First-session prompt contract

The only runnable prompt on the new first-result path must be exactly:

```text
Use only reklawdbox's read_library tool. Show me:
- my total track and playlist counts
- my top genres
- my average BPM and key distribution

Do not call external services, analyze audio, use or populate enrichment/audio
caches, write files, stage changes, or export XML.
```

Do not add a second copy-paste prompt on this page that calls another tool. A
link to later goals is enough.

Bracket that one prompt with exactly one marker pair:

````md
<!-- doc-contract:first-session-prompt tool=read_library -->

```text
...the prompt above...
```

<!-- /doc-contract:first-session-prompt -->
````

The marker must contain exactly one `text` fence, and that must be the only
runnable fence on the entire page. Missing, duplicate, nested, malformed, or
unmatched markers fail. The checker must also scan every other fence on the page
and reject any live MCP tool name/call outside the marked prompt. Fixtures must
cover a second tool inside the marker, a live tool outside it, a second marker
pair, and each missing negative boundary.

## Canonical goal mapping

Export an ordered `goalDefinitions` array with exact objects
`{ id, title, summary }`, and add a validated presentation-neutral `goals`
array to every Plan 026 workflow record. Use this exact order/membership:

| Goal ID             | Title                          | Workflow records                                           |
| ------------------- | ------------------------------ | ---------------------------------------------------------- |
| `inspect-health`    | Inspect collection health      | `library-health`                                           |
| `clean-library`     | Clean existing metadata        | `library-cleanup`, `collection-audit`, `metadata-backfill` |
| `prepare-downloads` | Prepare newly downloaded music | `batch-import`                                             |
| `classify-genres`   | Classify or audit genres       | `genre-classification`, `genre-audit`                      |
| `build-for-mixing`  | Build for mixing               | `set-building`, `pool-building`, `chapter-set-planning`    |
| `explore-dj-ideas`  | Explore DJ ideas               | `dj-prompts`                                               |

Write a concise, capability-bounded summary for each definition; “Explore DJ
ideas” must not promise catalog browsing. The validator must reject unknown,
duplicate, reordered, or empty goal definitions/record arrays, enforce exact
membership, and prove all eleven workflows are covered once by these groups.
`GoalChooser.astro` consumes both exports and selects workflows by `goals`,
never by hard-coded workflow IDs. Within multi-record groups render each
canonical workflow as its own direct link; do not add an accordion or
intermediate goal click. “Browse all workflows” and “Reference overview” are
fixed navigation links rather than synthetic workflow records.

## Steps

### Step 1: Verify the supported installation and first-call matrix

Before rewriting navigation, inspect the live setup CLI, README, MCP host
configuration docs, and `read_library` handler. Publish only currently
supported paths:

- Homebrew/binary setup and the hosts the `setup` command actually configures:
  Claude Code at the current `~/Music/.mcp.json` scope and a detected Claude
  Desktop config;
- manual configuration where automatic setup is unsupported (including Codex
  if that remains the integrated behavior);
- source-build users invoke `./target/release/reklawdbox setup`, not an
  uninstalled PATH binary;
- the current `setup` command installs and validates Essentia before host
  configuration, so its automatic path needs Python/venv even though Essentia
  is not used by the first session;
- users who cannot or do not want to install Essentia can use the already
  documented manual host configuration with the absolute Homebrew/source
  binary path, then reconnect and run the same first session;
- Codex remains a manual configuration path;
- reconnection is host-specific: Claude Code uses `/mcp` or a new conversation,
  Claude Desktop must be relaunched, and Codex must start a new task after its
  MCP configuration changes.

Do not duplicate a full host manual on the first-session page. Keep it on
Install and link to troubleshooting for missing tools or DB detection.

**Verify**:

```bash
cargo build --release
./target/release/reklawdbox setup --help
rg -n -e "Claude" -e "Codex" -e "Desktop" -e "target/release/reklawdbox setup" -e "Essentia" -e "install_essentia" -e "Manual configuration" site/src/content/docs/getting-started/index.mdx README.md src/cli/setup.rs
```

Expected: build/help exit 0 and the evidence search supports each published
host/source/Essentia statement, including automatic setup ordering and the
manual no-Essentia host-config fallback. If docs and setup behavior disagree,
STOP instead of carrying the mismatch into the new page.

### Step 2: Create the dedicated first-session route

Add `/getting-started/first-session/` with these sections:

1. **Before you start** — installation complete, host reconnected, Rekordbox
   library available; define the ten-minute boundary. State that automatic
   `setup` attempts Essentia first, while manual host configuration can connect
   the binary without Essentia and this first result does not use it.
2. **Copy this first prompt** — exactly the bounded contract above.
3. **What you should see** — only fields actually returned by `LibraryStats`.
4. **What just happened** — one read through the enforced read-only DB path; no
   external request, audio analysis/cache prerequisite, direct write, staged
   change, or XML output.
5. **If it did not work** — direct links to missing-tool, host config, and DB
   path troubleshooting.
6. **Choose what to do next** — render the shared goal chooser.

Do not imply that reading statistics changes cache/audit state unless the live
handler proves it does. Avoid celebratory claims that mask an empty library;
explain zero-track output as a configuration/import diagnostic.

**Verify**:

```bash
test -f site/src/content/docs/getting-started/first-session.mdx
rg -n -e "read_library" -e "external services" -e "analyze audio" -e "write files" -e "stage changes" -e "enrichment/audio" -e "export XML" -e "zero" site/src/content/docs/getting-started/first-session.mdx
! rg -n -e "lookup_discogs" -e "enrich_tracks" -e "analyze_audio_batch" -e "update_tracks" -e "write_xml" site/src/content/docs/getting-started/first-session.mdx
```

Expected: the route source exists, all bounded/no-side-effect statements are
present, and no second executable tool call appears.

### Step 3: Build a goal chooser from canonical workflow records

Create a server-rendered `GoalChooser.astro` using Plan 026's records. Present
at least these intents:

- inspect collection health;
- clean existing metadata;
- prepare newly downloaded music;
- classify or audit genres;
- build a set, compatible pool, or chapter plan;
- explore bounded DJ ideas and supplied candidates;
- browse all workflows/reference.

Each goal group must show a direct canonical workflow card for every matching
record, with collection-impact class, any direct user files, local-state writes,
created XML/backup/artwork outputs, and prerequisite summary before its link.
Put the lowest-commitment collection-read-only path first. Render its local-state
field from the canonical record; at the audited
Plan 026 base Library Health writes neither cache nor audit state, so do not
invent those effects. Do not call it bounded or exhaustive unless Plan 028 or
later integrated behavior proves it. The component must work without JavaScript
and preserve keyboard/visible focus order.

Use the required canonical `goals` arrays and exact membership above; do not add
a workflow-ID mapping inside the component.

**Verify**:

```bash
rg -n -e "workflows.mjs" -e "goals" -e "libraryImpact" -e "localStateWrites" -e "outputs" -e "prerequisites" site/src/components/GoalChooser.astro
! rg -n -e "client:load" -e "client:only" site/src/components/GoalChooser.astro
node -e "import('./site/src/data/workflows.mjs').then(m => m.validateWorkflows(m.workflows))"
```

Expected: the chooser imports canonical data, visibly consumes impact/local/
output/prerequisite fields, selects records through `goals`, uses no client
hydration, and the workflow module validates the exact membership.

### Step 4: Reframe the homepage journey

Change the primary sequence to:

1. install and connect;
2. get a safe first result;
3. choose a goal with side effects visible.

The primary CTA should lead to Install; the next-step CTA should lead to First
Session. Replace the universal Library Cleanup promotion with one visually
secondary compact link to `/workflows/`; do not render `GoalChooser` or direct
workflow cards on the homepage, because those would bypass the primary Install
→ First Session → goal journey. Keep Library Cleanup available through the
chooser/catalog, where its canonical direct file-write and long-running
characteristics are visible.

Preserve any release-script sentinel/version string used by automation. Plan
027's checker should fail if this string disappears.

**Verify**:

```bash
rg -n -e "/getting-started/" -e "/getting-started/first-session/" -e "/workflows/" site/src/content/docs/index.mdx
! rg -n -e "recommended starting point for every new user" -e "Start here: Library Cleanup" site/src/content/docs/index.mdx
```

Expected: the three-stage journey is linked and the universal cleanup CTA is
absent while the version sentinel remains unchanged in the diff.

### Step 5: Separate install completion from discovery

Keep host-specific commands, source-build notes, and troubleshooting on
`/getting-started/`. After the existing verification step, link to the new
first-session page rather than full cleanup. Remove any duplicated “run this
summary” prompt so there is one canonical copy-paste version on the new page;
the Install page can describe the expected handoff and link.

Use these exact top-level sidebar entries in this order:

```js
{ slug: 'getting-started', label: 'Install' }
{ slug: 'getting-started/first-session', label: 'First 10 minutes' }
{ slug: 'workflows', label: 'Choose a workflow' }
```

Do not create a circular “next” flow.

**Verify**:

```bash
rg -n "/getting-started/first-session/" site/src/content/docs/index.mdx site/src/content/docs/getting-started/index.mdx
rg -n -e "Install" -e "First 10 minutes" -e "Choose a workflow" site/astro.config.mjs
```

Expected: homepage and Install both link forward to First Session, and sidebar
line order/targets are exactly Install → First 10 minutes → Choose a workflow
with no reverse primary CTA.

### Step 6: Extend deterministic documentation checks

Add focused fixture/live assertions to Plan 027's checker:

- the first-session source/route exists;
- exactly one well-formed
  `doc-contract:first-session-prompt tool=read_library` marker pair contains the
  page's sole runnable fence, whose exact prompt names only `read_library`;
- the prompt includes all six negative boundaries: external service, audio
  analysis, enrichment/audio cache use/population, file write, staged change,
  XML;
- it contains no hydrate, provider lookup, tag/artwork, staging, or XML tool
  call;
- homepage and Install link to First Session;
- the universal cleanup CTA is absent;
- `goalDefinitions` and record memberships validate exactly, and GoalChooser
  resolves records through those canonical exports;
- the homepage contains exactly one `v<crate-version> —` sentinel matching
  `Cargo.toml`.

Because `Cargo.toml` is the sentinel's source of truth, add it to both the docs
CI path filter and release docs-sensitive predicate. A crate-version-only
change must run the contract rather than waiting for a simultaneous site edit.
Add a pure checker helper that parses the CI `on.push.paths` and
`on.pull_request.paths` inventories plus the arguments inside release
`docs_contract_changed()`. Require `Cargo.toml` and the already-integrated
`Cargo.lock` in all three inventories while preserving every other
pre-existing required path. Fixture omission from push, pull request, and
release separately; a repository-wide text match is not evidence that either
path is wired into the correct predicate.

Use the exact marker rules above rather than snapshotting whole pages. Fixtures
must cover missing/duplicate/malformed markers, a runnable fence outside the
marker, a second live tool inside/outside it, every omitted boundary, invalid or
reordered goals, and a missing/mismatched/duplicate version sentinel.

**Verify**:

```bash
node --test scripts/check-doc-contract.test.mjs
cd site && npm run build && cd ..
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: fixture and live checks exit 0, including every negative marker,
goal, version, and three-inventory trigger case above.

### Step 7: Walk the production journey on desktop and mobile

Build and serve `site/dist`. At desktop and 390px mobile, follow:

- `/` → Install;
- Install → First 10 minutes;
- First Session → one read-only workflow and one mutation-capable workflow via
  direct links inside expanded goal groups;
- First Session → all workflows.

Confirm primary actions are clear without scrolling past contradictory CTAs,
the prompt is copyable, cards have no horizontal overflow, collection/local/
output labels do not rely on color, focus order matches visual order, and a
user reaches a workflow destination in exactly three deliberate selections
after landing. Do not count or require an intermediate goal/accordion click.

If a live MCP smoke is performed, call only `read_library`; do not authorize
providers or mutations.

**Verify**:

```bash
cd site && npm run build
test -f dist/index.html
test -f dist/getting-started/index.html
test -f dist/getting-started/first-session/index.html
test -f dist/workflows/index.html
```

Expected: all routes build. Record desktop, 390px mobile, and keyboard results
for the complete landing → install → first result → chosen goal path; note the
number of deliberate selections and any overflow/focus defect.

## Test plan

- Contract fixtures structurally enforce the exact one-tool marker/prompt,
  goals, version sentinel, and journey links.
- The live schema remains the oracle for `read_library` name/parameters.
- Astro build proves the route, imports, and canonical record lookups.
- Production-preview desktop/mobile/keyboard walkthrough proves discovery UX.
- No runtime test is required because this is documentation/UI composition
  only.

## Done criteria

- [ ] A dedicated first-session route exists for an already connected user.
- [ ] Its only runnable first action is `read_library`.
- [ ] Exactly one marked prompt and no other runnable fence pass every negative fixture.
- [ ] No-network, no-analysis/cache prerequisite, no-write, no-stage, and no-XML boundaries are explicit.
- [ ] Homepage and Install lead through first result before workflow choice.
- [ ] Full Library Cleanup is no longer the universal onboarding path.
- [ ] Goal choices expose collection impact, local/output side effects, and prerequisite differences from canonical data.
- [ ] Ordered goal definitions cover all eleven workflows and link directly without an intermediate click.
- [ ] Supported host/source-build/Essentia guidance is accurate and bounded.
- [ ] The primary landing-to-workflow journey is exactly three selections and the homepage version sentinel still matches the crate.
- [ ] `Cargo.toml`-only and `Cargo.lock`-only changes trigger the docs contract in CI and release preflight.
- [ ] Checker fixtures/live gate, site build, and `dprint check` pass.
- [ ] Desktop, mobile, and keyboard walkthroughs pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- A prerequisite plan is missing or its safety/setup facts conflict.
- `read_library` now touches networks, audio, writable state, or lacks the
  documented result fields.
- A useful bounded result requires implementing a new tool.
- The canonical workflow catalog is missing or factually unresolved.
- Starlight navigation cannot express a non-circular journey without a broader
  layout rewrite.
- Accurate host setup depends on an unsupported/unverified configuration path.

## Maintenance notes

- Keep the first-session prompt tied to stable, no-parameter, read-only
  behavior.
- Do not gradually add provider or mutation calls to this trust-building path.
- Keep install time and connected-session time claims distinct.
- Review the page whenever `read_library`, setup host support, or workflow
  impact/side-effect records change.
