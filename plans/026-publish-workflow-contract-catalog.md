# Plan 026: Publish a canonical workflow contract catalog

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving on. If a
> STOP condition occurs, stop and report rather than inventing a contract.
> Update this plan's row in `plans/README.md` only if the orchestrator/reviewer
> does not own the portfolio index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- site/src/content/docs/workflows site/src/content/docs/agent/index.mdx site/src/components site/src/data site/astro.config.mjs
> ```
>
> Plans 021, 023, 024, and 025 must be present on the execution base. Compare
> their integrated safety, readiness, recovery, and Rekordbox-handoff wording
> with every record below. Missing dependencies or an unresolved workflow fact
> is a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 021, 023, 024, 025
- **Category**: direction / docs / UX
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

The site presents workflows as prose pages with inconsistent prerequisite,
duration, mutation, recovery, and final-handoff language. The overview says all
workflows follow one read → enrich → stage → export lifecycle even though
Library Health is read-only, Collection Audit writes audio files directly, and
set workflows export playlists. A user cannot compare paths safely before
opening them, and later docs checks have no structured source of truth.

This plan introduces one small machine-readable catalog and uses it to render a
human choice surface and consistent per-workflow contracts. It does not change
workflow runtime behavior.

## Current state

- `site/src/content/docs/workflows/index.mdx:10` asserts a universal lifecycle.
- The site currently has exactly eleven user workflow pages:
  `library-cleanup`, `collection-audit`, `metadata-backfill`,
  `genre-classification`, `genre-audit`, `set-building`, `pool-building`,
  `chapter-set-planning`, `batch-import`, `library-health`, and `dj-prompts`.
- `site/src/content/docs/workflows/library-cleanup.mdx:15-17` correctly calls
  out direct audio-file writes in Session 1.
- Batch Import performs direct file operations; Library Health is read-only;
  Set/Pool/Chapter workflows stage or export playlist results. These cannot
  share one mutation label.
- `site/astro.config.mjs:104-166` builds the sidebar but does not expose the
  Workflows or Reference landing pages as first-class discovery destinations.
- There is no `site/src/data/` workflow contract today.
- Plans 021–025 define canonical vocabulary for safety, provider readiness,
  internal-state recovery, and XML handoff. Reuse it exactly.

## Canonical record contract

Create `site/src/data/workflows.mjs` as the single ordered export. It must have
exactly one record for each of the eleven IDs above and no rendered prose blobs.
The model must preserve conditionality rather than flattening every possible
effect into an unconditional claim. Each record must contain, at minimum:

```js
{
  id,
  title,
  route,
  summary,
  audience,
  kind,
  libraryImpact,
  sideEffects: {
    stagedMetadata: {
      creates,
      flushesExistingOnExport,
    },
    directUserFiles,
    localStateWrites,
    outputs,
  },
  network,
  runtimeHelp,
  prerequisites,
  scope,
  duration,
  resumability,
  approval,
  recovery,
  output,
  rekordboxHandoff,
  variants,
}
```

Use finite vocabulary where possible:

- `kind`: `workflow` or `catalog`; `dj-prompts` is a catalog whose materially
  different recipes are represented in `variants`;
- `libraryImpact`: `read-only`, `staged-metadata`, `direct-audio-files`,
  `direct-library-files`, or `mixed`; this summarizes impact on the user's
  Rekordbox/audio collection, not all process writes;
- `sideEffects.stagedMetadata.creates`: whether the workflow itself stages
  metadata;
- `sideEffects.stagedMetadata.flushesExistingOnExport`: whether a playlist-only
  export also drains unrelated changes that were already staged;
- each `directUserFiles`, `localStateWrites`, `outputs`, and handoff entry is an
  object `{ kind, mode, condition? }`, where `mode` is `always`, `conditional`,
  `optional`, or `on-export`; a conditional/optional entry requires a concise
  condition;
- direct-file kinds include `audio-tags`, `embedded-artwork`,
  `extracted-artwork`, `downloaded-artwork`, `move-rename`,
  `archive-extraction`, `archive-move`, and `directory-create-remove`;
- local-state kinds include `enrichment-cache`, `audio-cache`, `audit-state`,
  `preset`, `timbral-normalization`, and `provider-session`; do not describe
  credentials or collapse automatic timbral statistics into user calibration;
- output kinds include `backup`, `metadata-xml`, `playlist-xml`,
  `artwork-file`, and `organized-library-files`;
- `network` is an object with `level: none | conditional | required` and an
  optional condition/reason; use `conditional` for cache/gap/recipe-dependent
  access instead of calling it universally optional or required;
- `runtimeHelp` is either `null` or
  `{ topic, menuOrder, recommendedOrder }`. `topic` is the primary accepted
  `help()` topic, `menuOrder` is a unique 1–9 position, and
  `recommendedOrder` is either a unique 1–7 position or `null`;
- `duration`: qualified text such as `minutes`, `hours`, `overnight`, or
  `scope-dependent`, never an unsupported guarantee;
- `resumability`: state the real checkpoint/cursor behavior, including current
  limitations that Plan 028 will later improve;
- handoff kinds include `reload-tag`, `metadata-xml`, `playlist-xml`,
  `library-file-import`, `manual-cover-art`, `manual-relocate`,
  `import-or-delete-orphans`, `assign-playlists`, and `remove-duplicates`;
  use an empty array for no handoff and ordered entries for Library Cleanup;
- `variants` is empty for a normal workflow. `dj-prompts` must have one
  evidence-backed variant per current recipe, each able to override network,
  local-state writes, prerequisites, duration, and output without pretending
  that all recipes share one contract.

`dj-prompts.variants` has no open-ended inheritance. Its exact shape is:

```js
{
  id,
  title,
  summary,
  network,
  localStateWrites,
  prerequisites,
  duration,
  output,
}
```

Each variant inherits only these validated parent invariants: collection
`libraryImpact: 'read-only'`, no staged metadata, no direct user-file writes,
no filesystem outputs, and no Rekordbox handoff. Every field shown above is
required on the variant itself; there are no partial overrides or fallback
merges. The six IDs, in page order, are:

1. `gig-prep` — `Gig Prep`
2. `collection-gap-analysis` — `Collection Gap Analysis`
3. `dig-session-partner` — `Dig Session Partner`
4. `post-gig-debrief` — `Post-Gig Debrief`
5. `harmonic-journey-planning` — `Harmonic Journey Planning`
6. `practice-session-design` — `Practice Session Design`

The validator must require exactly these six unique IDs/titles in this order
and apply the same network, local-write token/mode, prerequisite, duration, and
output validation to each variant.

The runtime-help mapping is also exact. `library-cleanup` and `dj-prompts` use
`runtimeHelp: null`. The other records use this mapping, with menu order derived
from their relative order in the canonical 11-record module:

| Workflow             | Topic         | Menu | Recommended |
| -------------------- | ------------- | ---- | ----------- |
| Collection Audit     | `audit`       | 1    | 1           |
| Metadata Backfill    | `metadata`    | 2    | 2           |
| Genre Classification | `genre`       | 3    | 3           |
| Genre Audit          | `genre audit` | 4    | 4           |
| Set Building         | `set`         | 5    | 5           |
| Pool Building        | `pool`        | 6    | 6           |
| Chapter Set Planning | `chapter`     | 7    | 7           |
| Batch Import         | `import`      | 8    | `null`      |
| Library Health       | `health`      | 9    | `null`      |

Plan 027 will compare the nine-entry runtime menu and seven-entry recommended
sequence to these fields. Do not make runtime help expose the two records with
`null`, and do not treat the 11-page site catalog, nine runtime SOPs, and seven
ordered cleanup steps as one inventory.

The validator must reject `libraryImpact: 'read-only'` when staged metadata or
direct user-file effects are present. It must reject
`flushesExistingOnExport: true` unless playlist XML export is possible, reject
conditional/optional entries without a condition, validate all finite tokens,
require variants for `kind: 'catalog'`, and enforce the exact runtime-help
subset/topics/contiguous menu and recommended positions above. A workflow may
still be read-only with respect to the collection while writing cache/audit
state; the cards and contract must disclose that local write. `output`
describes the user-visible result, while `sideEffects.outputs` enumerates files
created on disk. Arrays are also appropriate for prerequisites, approval
checkpoints, and recovery. JSDoc typedefs may provide editor validation, but do
not add a build dependency solely for schema validation in this plan.

### Critical classifications to preserve

The executor must still inspect every page/SOP/handler, but these audited edge
cases are binding unless live evidence has changed:

- **Batch Import** directly extracts/archives/moves library files, can write
  tags and artwork, conditionally uses provider/cache/session state, and ends
  with a normal Rekordbox library-file import plus conditional manual WAV cover
  art. Verify the concrete file-import handoff against
  `docs/rekordbox/manual/03-adding-tracks.md`; do not leave it as "next steps."
- **Collection Audit** writes audit state, conditionally writes tags/renames
  unimported files, and has conditional Reload Tag and manual-relocate
  handoffs. It does not stage metadata or export XML.
- **DJ Prompts** is a catalog, not one homogeneous workflow. Local recipes and
  lookup/research recipes have different network/cache contracts. Do not encode
  unsupported catalog/credits behavior; Plan 032 will align the remaining
  prompt prose with the same variant facts.
- **Library Health** is collection-read-only and network-free, but its report
  can recommend several conditional manual Rekordbox actions. Those handoffs
  do not turn the scan itself into a write.
- **Set Building, Pool Building, and Chapter Set Planning** create no staged
  metadata. Their playlist export can flush unrelated changes already in
  `ChangeManager`; expose `flushesExistingOnExport: true`. Pool/chapter scoring
  can automatically persist timbral-normalization statistics, and preset
  persistence is optional.
- **Library Cleanup** is `mixed` and has an ordered handoff: Reload Tag after
  direct file fixes, then metadata XML imports after staged-metadata sessions.
- Every XML-producing workflow records backup as `on-export` and conditional on
  the integrated backup result. At this plan's base a configured missing custom
  script may still produce a skipped status; do not call backup guaranteed.
  Plan 029 must update the records when it makes backup fail closed.

## Commands you will need

| Purpose                | Command                                                                                                                                                                                                                                                                                                                                                                             | Expected on success                         |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| Count records          | `node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { if (workflows.length !== 11) process.exit(1); console.log(workflows.map(x => x.id).join('\\n')) })"`                                                                                                                                                                                                      | exit 0; eleven unique IDs in intended order |
| Validate DJ variants   | `node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { const x=workflows.find(w => w.id==='dj-prompts'); const expected=['gig-prep','collection-gap-analysis','dig-session-partner','post-gig-debrief','harmonic-journey-planning','practice-session-design']; if (JSON.stringify(x?.variants.map(v => v.id)) !== JSON.stringify(expected)) process.exit(1) })"` | exit 0; six exact ordered recipes           |
| Validate runtime help  | `node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { const menu=workflows.filter(w => w.runtimeHelp).sort((a,b) => a.runtimeHelp.menuOrder-b.runtimeHelp.menuOrder); const recommended=menu.filter(w => w.runtimeHelp.recommendedOrder != null).sort((a,b) => a.runtimeHelp.recommendedOrder-b.runtimeHelp.recommendedOrder); if (menu.length!==9              |                                             |
| Validate routes        | `node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { for (const x of workflows) if (!x.route.startsWith('/workflows/')) throw Error(x.id) })"`                                                                                                                                                                                                                 | exit 0                                      |
| Remove false universal | `rg -n -e "every workflow" -e "all workflows.*read.*enrich.*stage.*export" site/src/content/docs/workflows site/src/content/docs/agent/index.mdx`                                                                                                                                                                                                                                   | exit 1 for the stale universal claim        |
| Format                 | `dprint check`                                                                                                                                                                                                                                                                                                                                                                      | exit 0                                      |
| Build                  | `cd site && npm run build`                                                                                                                                                                                                                                                                                                                                                          | exit 0                                      |

## Scope

**In scope**:

- `site/src/data/workflows.mjs` — create
- `site/src/components/WorkflowCatalog.astro` — create
- `site/src/components/WorkflowContract.astro` — create
- `site/src/content/docs/workflows/index.mdx`
- the eleven `site/src/content/docs/workflows/*.mdx` pages, but only to render
  the shared contract and remove immediately duplicated/contradictory facts
- `site/src/content/docs/agent/index.mdx`
- `site/astro.config.mjs`
- `site/src/styles/custom.css` only if the existing Starlight styles cannot
  express an accessible layout
- `plans/README.md` for the status row only

**Out of scope**:

- Editing any SOP partial, Rust source, tool parameter, or runtime help.
- Rewriting the full human workflow prose; Plan 031 owns audience separation.
- Implementing filtering, saved choices, analytics, or client-side state.
- Promising batch convergence before Plan 028.
- Adding a second YAML/JSON mirror of the same records.
- Hiding risk or prerequisites to make a card shorter.

## Git workflow

- Branch: `codex/026-publish-workflow-contract-catalog`
- Preferred commit: `docs(workflows): publish canonical workflow contracts`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Inventory and classify all eleven workflows

Read each human page, matching SOP, tool schema, and relevant handler before
writing its record. For every workflow, answer these questions with source
evidence:

1. What user outcome does it produce?
2. Does it use the network or optional local audio analysis?
3. What can it write: nothing, staged metadata, audio tags/artwork, organized
   files, internal state, backups, metadata XML, or playlist XML?
4. Is each write automatic, conditional, optional, or only performed on
   export? Can playlist export flush metadata staged by an earlier workflow?
5. What must exist first?
6. Is the scope bounded, and how does continuation work today?
7. Where must the user approve or use Rekordbox, and in what order?
8. How can the user recover or retry?
9. Is this one workflow or a catalog of variants with materially different
   network and state contracts?

Do not infer a duration from code size. Use `scope-dependent` when the site has
no measured basis. If the human page and embedded SOP disagree, follow the
integrated dependency plans and code-backed behavior; STOP if neither is
authoritative. For Batch Import, use the local Rekordbox manual to make the
file-import handoff concrete. For DJ Prompts, model current recipes as variants
and flag stale prompt prose for Plan 032 rather than copying it into the
canonical record.

**Verify**:

```bash
test "$(rg --files site/src/content/docs/workflows | rg '\.mdx$' | rg -v '/index\.mdx$' | wc -l | tr -d ' ')" -eq 11
rg -n -e "write_xml" -e "write_file_tags" -e "cache" site/src/content/docs/workflows site/src/partials/sops
```

Expected: the first command confirms the eleven-page inventory; the second
provides source locations for XML, direct-file, and local-state classifications
that are reflected in the record draft. If the count differs, STOP.

### Step 2: Create and self-validate the canonical data module

Add the ordered `workflows` export and freeze the vocabulary in a header
comment/JSDoc. Add a tiny import-time validator or exported `validateWorkflows`
function that rejects duplicate IDs/routes, missing fields, unknown
`kind`, `libraryImpact`, network levels, modes, side-effect/handoff tokens,
conditional entries without conditions, contradictory impact/effect
combinations, invalid staged-metadata flush combinations, catalog records
without variants, the exact runtime-help subset/topics/contiguous positions,
and non-absolute routes. The validator must be dependency-free and must not
read source files.

Keep facts concise enough for cards but complete enough that Plans 027, 030,
and 031 can consume them without scraping prose.

**Verify**:

```bash
node -e "import('./site/src/data/workflows.mjs').then(m => { m.validateWorkflows(m.workflows); console.log(m.workflows.length) })"
node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { const x=workflows.find(w => w.id==='dj-prompts'); const expected=['gig-prep','collection-gap-analysis','dig-session-partner','post-gig-debrief','harmonic-journey-planning','practice-session-design']; if (JSON.stringify(x?.variants.map(v => v.id)) !== JSON.stringify(expected)) process.exit(1) })"
node -e "import('./site/src/data/workflows.mjs').then(({workflows}) => { const menu=workflows.filter(w => w.runtimeHelp).sort((a,b) => a.runtimeHelp.menuOrder-b.runtimeHelp.menuOrder); const recommended=menu.filter(w => w.runtimeHelp.recommendedOrder != null).sort((a,b) => a.runtimeHelp.recommendedOrder-b.runtimeHelp.recommendedOrder); if (menu.length!==9 || recommended.length!==7) process.exit(1) })"
```

Expected: the first prints `11`; all commands exit 0, the second proves the
exact six DJ recipe records, and the third proves the separate nine-menu/seven-
recommended runtime subsets.

### Step 3: Build accessible shared components

Create:

- `WorkflowCatalog.astro`: renders all records as links/cards with outcome,
  collection impact, direct-file/local-state/output effects, prerequisites,
  duration/scope, conditionality, and handoff visible before click; a catalog
  record such as DJ Prompts must visibly say that behavior varies by recipe;
- `WorkflowContract.astro`: renders one record consistently at the top of an
  individual workflow page, with sections for what changes, prerequisites,
  local state and created files, scope/resumability, approvals,
  recovery/output, Rekordbox handoff, and any existing-staged-change flush
  warning.

Both must be server-rendered, work without JavaScript, use semantic headings or
definition lists, and expose visible link focus. Do not encode essential facts
only as color or icon. Reuse Starlight tokens before adding custom CSS.

**Verify**:

```bash
rg -n -e "libraryImpact" -e "directUserFiles" -e "localStateWrites" -e "outputs" -e "flushesExistingOnExport" -e "mode" -e "variants" -e "prerequisites" -e "rekordboxHandoff" site/src/components/WorkflowCatalog.astro site/src/components/WorkflowContract.astro
! rg -n -e "client:load" -e "client:only" site/src/components/WorkflowCatalog.astro site/src/components/WorkflowContract.astro
```

Expected: both components render every safety-critical field and neither needs
client-side hydration.

### Step 4: Replace the false workflow overview

Rewrite `/workflows/` as a choice surface:

- start with the user's goal, not implementation internals;
- explain that workflows have different side effects and prerequisites;
- render the full catalog;
- link to Safety and the provider capability reference;
- identify Library Health as the lowest-commitment exploration path without
  calling it universally complete or bounded;
- retain direct access to every current workflow.

Remove the universal read/enrich/stage/export claim from the workflow and agent
overview. The agent index may summarize the same catalog order but must not
duplicate the full records.

**Verify**:

```bash
rg -n "WorkflowCatalog" site/src/content/docs/workflows/index.mdx
! rg -n -e "every workflow" -e "all workflows.*read.*enrich.*stage.*export" site/src/content/docs/workflows/index.mdx site/src/content/docs/agent/index.mdx
```

Expected: the catalog is rendered once and the universal lifecycle claims are
absent.

### Step 5: Render the shared contract on each workflow page

Add one `WorkflowContract` invocation to each of the eleven pages. Remove only
nearby prerequisite/side-effect blurbs that now directly contradict or
duplicate the component. Preserve workflow-specific explanations, prompts, and
SOP renders for Plan 031.

Use record lookup by stable ID and make an unknown ID a build-time error. Do not
pass hand-written overrides from individual pages.

**Verify**:

```bash
test "$(rg -l "WorkflowContract" site/src/content/docs/workflows/*.mdx | rg -v '/index\.mdx$' | wc -l | tr -d ' ')" -eq 11
! rg -n "WorkflowContract.*(mutation|network|output)=" site/src/content/docs/workflows
```

Expected: exactly eleven workflow pages render the component and none passes a
hand-written factual override.

### Step 6: Make the catalog discoverable in navigation

Update `site/astro.config.mjs` so the sidebar includes:

- **Choose a workflow** linking `/workflows/` before individual workflows;
- the existing individual workflow links in the same canonical order;
- **Reference overview** linking `/reference/` before reference children, if
  that route exists on the integrated head.

Do not duplicate routes in autogenerated and explicit groups.

**Verify**:

```bash
rg -n -e "Choose a workflow" -e "Reference overview" site/astro.config.mjs
```

Expected: both overview links are explicit. Manually confirm their sidebar
order matches the canonical workflow module before proceeding.

### Step 7: Build and perform production-preview UX checks

Run the validator, formatter, and site build. In a production preview inspect:

- `/workflows/` at desktop and 390px mobile;
- one read-only workflow;
- one direct-write workflow;
- one staged/playlist-export workflow.

Confirm every card can be understood without hover, the difference between
read-only and direct writes is visible, prerequisite lists do not overflow,
keyboard focus follows the displayed order, and every destination resolves.

**Verify**:

```bash
node -e "import('./site/src/data/workflows.mjs').then(m => m.validateWorkflows(m.workflows))"
dprint check
cd site && npm run build
test -f dist/workflows/index.html
```

Expected: every command exits 0 and the catalog route builds. Record desktop,
390px mobile, and keyboard results for the five representative side-effect
cases in the review handoff.

## Test plan

- Dependency-free validation covers record completeness, enums/tokens/modes,
  conditional-entry requirements, staged-metadata flush consistency, catalog
  variants, uniqueness, ordered IDs, and absolute routes.
- Astro build proves every component lookup resolves.
- Static checks remove the false universal lifecycle.
- Browser QA covers three library-impact classes plus a cache-writing and an
  XML/backup-writing workflow on desktop/mobile.
- Plan 027 will add cross-source/runtime enforcement; do not pre-empt it with a
  second checker architecture here.

## Done criteria

- [ ] Exactly eleven workflow records exist in one ordered canonical module.
- [ ] Every record exposes collection impact, staged creation versus existing-change flush, conditional direct user files/local-state writes/filesystem outputs, network, prerequisites, scope/duration, resumability, approval, recovery, result, and ordered/conditional Rekordbox handoffs.
- [ ] DJ Prompts variants and Batch Import's concrete library-file handoff are represented without unsupported capabilities.
- [ ] The 11-page catalog, 9 runtime-help menu records, and 7 recommended-order records are explicit and separately validated.
- [ ] `/workflows/` lets users compare those facts before choosing.
- [ ] Every workflow page renders one shared contract from its ID.
- [ ] Workflows and Reference overviews are discoverable in the sidebar.
- [ ] The false universal lifecycle is absent.
- [ ] Validator, `dprint check`, and Astro production build pass.
- [ ] Desktop, mobile, and keyboard walkthroughs pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- Any prerequisite accuracy plan is missing or conflicts with live behavior.
- The live workflow set differs from the eleven listed and ownership is unclear.
- A side effect, provider requirement, recovery path, or final handoff cannot be
  established from code/SOP evidence.
- Rendering accurate contracts requires changing runtime behavior.
- The project already gained a materially different canonical workflow model.
- A layout needs client-side state or a broad theme rewrite to remain usable.

## Maintenance notes

- A new workflow is incomplete until one canonical record, page, navigation
  entry, and later contract-gate coverage land together.
- Collection impact, local/output side effects, network, handoff, and numeric duration claims deserve stricter
  review than marketing copy.
- Plans 027, 030, and 031 must consume this module rather than create mirrors.
- Plan 029 must update the backup entries from the integrated fail-closed
  behavior and keep the checker/catalog aligned.
- Keep the catalog server-rendered until measured scale justifies filtering.
