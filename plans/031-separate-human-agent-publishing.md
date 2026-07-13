# Plan 031: Separate human and agent publishing surfaces

> **Executor instructions**: Follow this plan in order and test generated
> search, sitemap, and LLM artifacts—not only source imports. Stop on any STOP
> condition rather than duplicating SOP content or hiding human pages. Update
> this plan's row in `plans/README.md` only if the orchestrator/reviewer does not
> own the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- site/src/content/docs/workflows site/src/content/docs/agent site/src/partials/sops site/astro.config.mjs site/package.json site/package-lock.json site/vendor/starlight-llms-txt scripts/check-doc-contract.mjs
> ```
>
> Plans 026, 027, and 030 must be integrated. Rebuild once before editing and inspect
> current Pagefind markers, sitemap entries, generic LLM bundles, and custom
> agent bundles. If excluding agent routes also removes their dedicated custom
> outputs, or accurate human pages require a second SOP copy, stop.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: 026, 027, 030
- **Category**: direction / information architecture / UX
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

Nine human workflow pages inline the same imperative SOP partials separately
published under `/agent/`. Both copies enter Pagefind, search-engine sitemaps,
and generic LLM bundles. Human discovery therefore returns model-control text,
generic machine-readable outputs duplicate entire workflows, and readers cannot
easily tell explanation from agent instructions.

Agent pages should remain public, stable, and available to MCP hosts/dedicated
plaintext sets. Human search, search engines, and generic LLM corpora should
carry the human explanation once; the canonical SOP should appear once in the
dedicated agent surface.

## Current state

The duplicated pairs are:

| Human route                          | Agent route / canonical partial                                    |
| ------------------------------------ | ------------------------------------------------------------------ |
| `workflows/batch-import.mdx`         | `agent/batch-import.mdx` / `sops/batch-import.mdx`                 |
| `workflows/collection-audit.mdx`     | `agent/collection-audit.mdx` / `sops/collection-audit.mdx`         |
| `workflows/metadata-backfill.mdx`    | `agent/metadata-backfill.mdx` / `sops/metadata-backfill.mdx`       |
| `workflows/genre-classification.mdx` | `agent/genre-classification.mdx` / `sops/genre-classification.mdx` |
| `workflows/genre-audit.mdx`          | `agent/genre-audit.mdx` / `sops/genre-audit.mdx`                   |
| `workflows/set-building.mdx`         | `agent/set-building.mdx` / `sops/set-building.mdx`                 |
| `workflows/pool-building.mdx`        | `agent/pool-building.mdx` / `sops/pool-building.mdx`               |
| `workflows/chapter-set-planning.mdx` | `agent/chapter-set-planning.mdx` / `sops/chapter-set-planning.mdx` |
| `workflows/library-health.mdx`       | `agent/library-health.mdx` / `sops/library-health.mdx`             |

- Human pages currently import/render the raw SOP partial; agent pages import
  the same file.
- `site/astro.config.mjs:48-101` already defines dedicated `Agent SOPs` custom
  LLM sets using `paths: ['agent/**']`; preserve them.
- `site/vendor/starlight-llms-txt/llms-full.txt.ts` passes no exclusion to
  generic full output.
- `site/vendor/starlight-llms-txt/llms-small.txt.ts` applies the existing
  `exclude` option only to generic small output, as documented in `types.ts`.
- Agent pages have no `pagefind: false` or `robots` directive.
- Starlight supports page-level Pagefind exclusion. At the planning baseline it
  adds its default sitemap only when no explicit integration named
  `@astrojs/sitemap` is present; the lockfile already resolves a compatible
  `@astrojs/sitemap` version transitively. Reconfirm this before promoting it to
  a direct dependency.
- A baseline build should show each paired workflow twice in
  `site/dist/llms-full.txt`, `/agent/**` in the sitemap, and
  `data-pagefind-body` on agent HTML while dedicated agent text still builds.

## Audience policy to enforce

Agent routes must:

- remain built and directly addressable at the same paths;
- keep exactly one canonical SOP import;
- remain included in dedicated agent plaintext sets;
- be labeled as agent/model-facing;
- use `pagefind: false`;
- emit `robots: noindex, nofollow`;
- be absent from the public sitemap and generic `llms-full.txt`/`llms-small.txt`.

This noindex guarantee applies to `/agent/**` HTML. Dedicated
`/_llms-txt/**` plaintext outputs intentionally remain public and linked from
`/llms.txt`; GitHub Pages cannot attach per-file `X-Robots-Tag` headers. This
plan preserves those model-facing files but does not promise that an external
search engine can never index their URLs.

Human workflow routes must:

- remain Pagefind-indexed, sitemap-visible, and in generic LLM output;
- explain outcome, risks, prerequisites, phases, decisions, recovery, output,
  and Rekordbox handoff using Plan 026's contract;
- not inline raw imperative SOPs or maintain a second operational copy;
- link to the agent route/plaintext only as an advanced automation resource.

Derive the paired inventory once from the canonical module:

```js
const agentPairs = workflows.filter((workflow) => workflow.runtimeHelp !== null)
```

Require exactly nine. Derive each human route from `workflow.route`, agent route
as `/agent/${workflow.id}/`, source paths from those routes, and the dedicated
per-SOP artifact as `dist/_llms-txt/${workflow.id}-sop.txt`. Do not copy the
nine-row table, IDs, or custom-set inventory into the checker.

## Commands you will need

| Purpose       | Command                                                                                                                                                                                                                                                                                                                                                            | Expected on success                                                                           |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------- |
| Build         | `cd site && npm run build`                                                                                                                                                                                                                                                                                                                                         | exit 0; all human/agent/plaintext routes build                                                |
| Human imports | `rg -n -e "partials/sops/batch-import" -e "partials/sops/collection-audit" -e "partials/sops/metadata-backfill" -e "partials/sops/genre-classification" -e "partials/sops/genre-audit" -e "partials/sops/set-building" -e "partials/sops/pool-building" -e "partials/sops/chapter-set-planning" -e "partials/sops/library-health" site/src/content/docs/workflows` | exit 1; all nine operational SOP imports are absent while unrelated human partials may remain |
| Agent imports | `rg -l "partials/sops/" site/src/content/docs/agent/*.mdx`                                                                                                                                                                                                                                                                                                         | exit 0; exactly the nine expected workflow files are listed, excluding index                  |
| Sitemap       | `! rg -q "https://reklawdbox.com/agent/" site/dist/sitemap-*.xml`                                                                                                                                                                                                                                                                                                  | exit 0                                                                                        |
| Pagefind      | `! rg -q "data-pagefind-body" site/dist/agent/genre-classification/index.html`                                                                                                                                                                                                                                                                                     | exit 0                                                                                        |
| Contract      | `node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist`                                                                                                                                                                                                                                                                         | exit 0; all nine audience assertions pass                                                     |
| Format        | `dprint check`                                                                                                                                                                                                                                                                                                                                                     | exit 0                                                                                        |

## Scope

**In scope — human workflow pages**:

- `site/src/content/docs/workflows/batch-import.mdx`
- `site/src/content/docs/workflows/collection-audit.mdx`
- `site/src/content/docs/workflows/metadata-backfill.mdx`
- `site/src/content/docs/workflows/genre-classification.mdx`
- `site/src/content/docs/workflows/genre-audit.mdx`
- `site/src/content/docs/workflows/set-building.mdx`
- `site/src/content/docs/workflows/pool-building.mdx`
- `site/src/content/docs/workflows/chapter-set-planning.mdx`
- `site/src/content/docs/workflows/library-health.mdx`

**In scope — agent and publishing surfaces**:

- `site/src/content/docs/agent/index.mdx`
- the nine matching `site/src/content/docs/agent/*.mdx` files above
- `site/astro.config.mjs`
- `site/package.json`
- `site/package-lock.json`
- `site/vendor/starlight-llms-txt/types.ts`
- `site/vendor/starlight-llms-txt/index.ts`
- `site/vendor/starlight-llms-txt/llms-full.txt.ts`
- `site/vendor/starlight-llms-txt/REKLAWDBOX.md`
- `scripts/check-doc-contract.mjs`
- `scripts/check-doc-contract.test.mjs`
- `plans/README.md` for the status row only

**Out of scope**:

- Editing any `site/src/partials/sops/*.mdx`; they remain canonical agent and
  compile-time runtime-help content.
- Changing Rust/MCP behavior or `src/tools/help_handler.rs`.
- Removing or renaming `/agent/**` routes or dedicated plaintext endpoints.
- Hiding human workflow pages from search/indexing.
- Replacing the vendored plugin wholesale.
- Recreating the SOP operational sequence in human prose.

## Git workflow

- Branch: `codex/031-separate-human-agent-publishing`
- Preferred commit: `docs(site): separate human and agent surfaces`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Capture the generated baseline and add red assertions

Build the integrated site and record, for all nine pairs:

- occurrence count in `llms-full.txt` and `llms-small.txt`;
- presence in the dedicated `/_llms-txt/agent-sops.txt` and per-SOP outputs;
- Pagefind body marker in human versus agent HTML;
- sitemap presence;
- direct route existence.

Extend Plan 027's fixture/live checker with the audience policy before the
rewrite. Build `agentPairs` exactly as above; assertions iterate that derived
array rather than hard-coding one example or a second nine-ID list. Keep
generated-output checks separate from source-import checks.

Add a pure audience-policy checker seam that receives source/built artifacts
and fails explicitly when an expected file is missing. It must enforce:

- all ten agent HTML routes (index plus nine pairs) exist;
- each agent HTML file has exactly one `robots` directive and no Pagefind body
  marker, while every paired human HTML file exists and has the Pagefind body
  marker;
- at least one sitemap XML exists; parsed `<loc>` values include every paired
  human route plus representative `/workflows/`, `/mcp-tools/`, and
  `/getting-started/` families, and exclude `/agent/` plus all nine agent routes;
- `llms-full.txt` and `llms-small.txt` both exist, contain each paired human
  heading exactly once, and contain every `Agent SOP: <title>` heading zero
  times;
- `_llms-txt/agent-sops.txt` exists and contains each derived agent heading
  exactly once;
- every derived `${id}-sop.txt` exists and contains exactly its own agent
  heading;
- each agent source imports/renders its matching SOP exactly once and paired
  human sources import/render none.

Fixture every missing-file class, a duplicate/missing robots directive, agent
Pagefind leakage, missing human Pagefind, one agent sitemap URL, leakage into
each generic bundle, a missing/duplicate custom SOP, and a wrong/duplicate
source import. A negated text search alone is never evidence that an expected
artifact existed.

**Verify**:

```bash
cd site && npm run build && cd ..
if node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist; then
  printf 'expected the new audience assertions to fail on the duplicated baseline\n' >&2
  exit 1
fi
```

Expected at this step: the checker exits nonzero with all-nine Pagefind,
sitemap, generic-LLM, and human-SOP duplication diagnostics. A build failure or
missing custom agent bundle is a STOP condition, not the expected red result.

### Step 2: Mark all agent pages for their intended audience

Add to the agent index and nine agent pages:

```yaml
pagefind: false
head:
  - tag: meta
    attrs:
      name: robots
      content: noindex, nofollow
```

Make the page title or introductory label unambiguous, such as
`Agent SOP: Genre Classification`, while preserving slugs, prompts, and one
canonical partial import. Explain on the agent index that these routes and
`/_llms-txt/**` are model-facing operational surfaces, with a link back to the
human workflow catalog.

Verify built HTML has the robots meta and no Pagefind body marker, while every
route still returns 200.

**Verify**:

```bash
cd site && npm run build
for f in dist/agent/index.html dist/agent/*/index.html; do
  test -f "$f"
  rg -q 'name="robots"' "$f"
  rg -q 'content="noindex, nofollow"' "$f"
  ! rg -q 'data-pagefind-body' "$f"
done
test "$(find dist/agent -name index.html | wc -l | tr -d ' ')" -eq 10
```

Expected: the agent index plus nine SOP routes build, all ten contain the robots
policy, and none has a Pagefind body marker.

### Step 3: Exclude agent routes from the sitemap

Promote the already-resolved `@astrojs/sitemap@3.7.3` to a direct dependency;
do not request an unqualified/current version. The lockfile delta should only
record that existing resolution at the package root. Register one explicit
sitemap integration in `site/astro.config.mjs`. Its filter must parse the
absolute URL string with `new URL(page)` and reject only pathnames beginning
`/agent/`.

Reconfirm Starlight does not also add an unfiltered default integration. Check
the built sitemap contains representative `/workflows/**`, `/mcp-tools/**`, and
`/getting-started/**` routes and contains no `/agent/` route.

**Verify**:

```bash
cd site && npm run build
test -n "$(find dist -maxdepth 1 -name 'sitemap-*.xml' -print -quit)"
! rg -n "https://reklawdbox.com/agent/" dist/sitemap-*.xml
rg -n -e "/workflows/" -e "/mcp-tools/" -e "/getting-started/" dist/sitemap-*.xml
```

Expected: agent routes have no matches and all three representative human route
families remain in the sitemap.

### Step 4: Add generic-full exclusion to the vendored LLM plugin

Add a backward-compatible option `excludeFull?: string[]`:

- declare/document it in `StarlightLllmsTextOptions`;
- pass it through the plugin/project context in `index.ts`;
- apply it only to generic full generation in `llms-full.txt.ts`;
- do not change the existing small-only `exclude` meaning;
- do not apply either generic exclusion to custom sets.

Configure:

```js
exclude: ['agent/**'],
excludeFull: ['agent/**'],
```

Preserve `customSets` with `paths: ['agent/**']`. Update
`site/vendor/starlight-llms-txt/REKLAWDBOX.md`: removing the vendor copy requires
upstream Astro compatibility **and** an equivalent generic-full exclusion, or a
tested migration preserving these outputs.

Verify both absence and presence: agent SOPs absent from generic full/small,
human workflows present once, agent workflows present once in dedicated sets.

**Verify**:

```bash
cd site && npm run build
test -f dist/llms-full.txt
test -f dist/llms-small.txt
test -f dist/_llms-txt/agent-sops.txt
! rg -n "^# Agent SOP:" dist/llms-full.txt dist/llms-small.txt
rg -n "^# Agent SOP:" dist/_llms-txt/agent-sops.txt
test "$(rg -c '^# Agent SOP:' dist/_llms-txt/agent-sops.txt)" -eq 9
```

Expected: generic bundles contain no agent heading, the dedicated bundle
contains exactly nine, and every per-SOP plaintext route still builds. If the
actual generated heading normalization differs, encode the equivalent all-nine
assertion in the checker rather than weakening it to one example.

### Step 5: Rewrite the nine human pages around human decisions

Remove raw SOP imports/renders. Organize each human page consistently:

1. Outcome;
2. `WorkflowContract` / before-you-start facts;
3. three to six human-readable phases;
4. decisions and approval checkpoints;
5. what changes and how to recover;
6. finish in Rekordbox;
7. one copyable start prompt;
8. next steps and an advanced Agent SOP link.

Preserve accurate corrections from Plans 021–025 and the Plan 026 contract.
Do not reproduce detailed batch loops, complete tool schemas, or model-control
phrases. Shared human UI partials such as the correct XML import handoff may
remain; only the nine operational agent SOP partials are prohibited.

**Verify**:

```bash
! rg -n -e "partials/sops/batch-import" -e "partials/sops/collection-audit" -e "partials/sops/metadata-backfill" -e "partials/sops/genre-classification" -e "partials/sops/genre-audit" -e "partials/sops/set-building" -e "partials/sops/pool-building" -e "partials/sops/chapter-set-planning" -e "partials/sops/library-health" site/src/content/docs/workflows
for id in batch-import collection-audit metadata-backfill genre-classification genre-audit set-building pool-building chapter-set-planning library-health; do
  rg -q "WorkflowContract" "site/src/content/docs/workflows/${id}.mdx"
  rg -q "/agent/${id}/" "site/src/content/docs/workflows/${id}.mdx"
done
```

Expected: no human page imports a canonical agent SOP, and all nine rewritten
pages retain the shared contract plus an advanced agent link.

### Step 6: Validate search and generated outputs end to end

In a production preview:

- query Pagefind for “genre classification”, “collection audit”, and “library
  health”; confirm human results appear and agent routes do not;
- open a human page and its direct agent counterpart;
- inspect `/llms-full.txt`, `/llms-small.txt`,
  `/_llms-txt/agent-sops.txt`, and one per-SOP plaintext route;
- inspect the sitemap.

At desktop and mobile widths confirm the human page reads as an explanation,
the agent page is clearly labeled, and advanced links do not become the primary
human CTA. Test keyboard focus on the human-to-agent link.

**Verify**:

```bash
node --test scripts/check-doc-contract.test.mjs
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: all-nine source/generated audience assertions pass. Record Pagefind
queries and desktop/mobile/keyboard observations for one human/agent pair in
the review handoff.

### Step 7: Run complete docs and source-boundary checks

Run formatter, site build, live docs checker, source import greps, and generated
artifact assertions. `git diff --name-only` must contain no SOP partial or Rust
source. If `npm install` updates unrelated dependency versions, revert those
unrelated lockfile changes without destructive worktree commands.

**Verify**:

```bash
dprint check
cd site && npm run build && cd ..
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
test -z "$(git status --short -- src site/src/partials/sops)"
git status --short
```

Expected: formatter, build, and checker exit 0; the source-boundary check is
empty; status contains only the listed Scope files and orchestrator-owned plan
status.

## Test plan

- Contract fixtures iterate all nine pair IDs and catch one missing policy.
- Generated-output tests cover both presence and absence in generic/custom
  bundles.
- Sitemap tests prove non-agent routes remain.
- Pagefind checks inspect built markers and real search results.
- Browser QA covers representative human/agent pages on desktop/mobile.
- Source checks ensure SOP partials remain unchanged and canonical.

## Done criteria

- [ ] Human pages no longer render raw agent SOP partials.
- [ ] Agent pages remain built with exactly one canonical SOP import.
- [ ] All `/agent/**` HTML routes are excluded from Pagefind, carry noindex/nofollow, and are absent from the sitemap.
- [ ] Dedicated `/_llms-txt/**` agent plaintext remains public and complete without being included in generic LLM bundles.
- [ ] Public sitemap contains no `/agent/**` route and retains human routes.
- [ ] Generic LLM outputs contain each paired human workflow once and no agent copy.
- [ ] Dedicated agent bundles retain every intended SOP once.
- [ ] Human search results do not return agent routes for tested queries.
- [ ] Site build, checker, generated assertions, and `dprint check` pass.
- [ ] Desktop/mobile/keyboard QA passes.
- [ ] No SOP partial or Rust source changed; no files outside Scope changed except index status.

## STOP conditions

Stop and report back if:

- Prerequisite factual plans are incomplete or Plan 026 contracts are unresolved.
- Pagefind/sitemap/generic exclusion removes an agent page from custom LLM output.
- Sitemap filtering removes any non-agent route.
- The vendored plugin already gained a different supported full-exclusion API.
- A human page cannot be accurate without duplicating/editing the canonical SOP.
- Generated output contains agent content through a second unaccounted source.
- Lockfile changes cannot be limited to the direct sitemap dependency.

## Maintenance notes

- `site/src/partials/sops/*.mdx` remains the sole operational source for agent
  pages and runtime embedded help.
- New agent pages require Pagefind/noindex/sitemap/generic-LLM treatment and a
  dedicated-output assertion.
- Dedicated plaintext is intentionally public on GitHub Pages; stronger crawler
  exclusion would require a separate hosting/header or robots policy design.
- Never remove the vendor copy solely because upstream supports Astro 7;
  preserve `excludeFull` behavior or migrate with equivalent tests.
- Review all generic and custom text outputs after plugin upgrades.
