# Plan 023: Define workflow readiness by the data each capability actually uses

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- src/cli/hydrate.rs src/store.rs src/tools/params.rs src/tools/resolve_handlers.rs src/tools/scoring.rs site/src/components/DataFlowDiagram.astro site/src/content/docs/cli/index.mdx site/src/content/docs/mcp-tools/library-data.mdx site/src/content/docs/mcp-tools/enrichment-analysis.mdx site/src/content/docs/workflows site/src/partials/sops
> ```
>
> Compare the live provider enums, `cache_coverage` response, and scoring cache
> reads with the excerpts below. If runtime support has expanded, stop and
> report the new matrix before editing prose.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 022
- **Category**: docs
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

The site currently uses "all providers at 100%" as a universal readiness gate.
No single command or response can satisfy that statement: CLI `hydrate` covers
Discogs, Beatport, and audio analysis; `cache_coverage` reports two audio
backends plus Discogs and Beatport; Bandcamp and MusicBrainz are used through
different enrichment/backfill paths. Transition and pool scoring do not read
enrichment caches at all. Readiness must be defined per workflow so users do
not repeatedly run an impossible check or spend hours hydrating data that a
creative tool never consumes.

## Current state

### Ground-truth capability matrix

- `src/cli/hydrate.rs:20-56` defines only `Discogs`, `Beatport`, and
  `Analysis`; the default is `discogs,beatport,analysis`.
- `src/tools/resolve_handlers.rs:422-443` returns coverage entries for
  `stratum_dsp`, `essentia`, `discogs`, and `beatport` only.
- `src/store.rs:325-384` defines provider coverage semantics: `searched`
  counts non-error cache rows (including legitimate no-match rows),
  `has_result` counts only exact/fuzzy rows, and error rows count as neither so
  they remain retryable.
- `src/tools/params.rs:272-290` allows MCP `enrich_tracks` providers
  `discogs`, `beatport`, and `bandcamp`; this is a different surface from CLI
  `hydrate`.
- `src/tools/scoring.rs:617-664` loads only fresh Stratum and Essentia cache
  rows. Lines 678-739 derive BPM/key/genre from Rekordbox metadata and those
  audio caches; no Discogs, Beatport, Bandcamp, or MusicBrainz cache is read.
- `site/src/content/docs/cli/index.mdx:44-100` already lists the three CLI
  hydrate providers accurately, but incorrectly calls hydration recommended
  before set building.

### Contradictory public workflow claims

Current `site/src/content/docs/workflows/library-cleanup.mdx:51-65` says one
CLI command fetches four metadata providers and that all must reach 100%:

```md
reklawdbox hydrate --cpu overnight -y

This fetches metadata from Discogs, Beatport, MusicBrainz, and Bandcamp, plus
runs audio analysis on every track.

All providers should be at 100% before moving on.
```

Current `site/src/partials/sops/metadata-backfill.mdx:15-49` asks
`cache_coverage` to report "all providers," then separately hydrates Bandcamp
and uses MusicBrainz later. The first statement cannot match the response.

Current `site/src/partials/sops/set-building.mdx:3-15` says transition scoring
uses cached audio analysis **and enrichment**, then blocks on all providers at
100%. `pool-building.mdx:3-15` and
`chapter-set-planning.mdx:13-17` repeat the same requirement.

`site/src/components/DataFlowDiagram.astro` currently draws every enrichment
output into Set Building, which visually reinforces the false prerequisite.

### Vocabulary to establish

Use these terms consistently:

1. **CLI hydration coverage** — Discogs, Beatport, and audio analysis work
   selected by `reklawdbox hydrate`.
2. **Reported cache coverage** — Stratum, Essentia, Discogs, and Beatport as
   returned by `cache_coverage` for a specific scope.
3. **Additional metadata sources** — Bandcamp and MusicBrainz, invoked by
   targeted enrichment/backfill/research paths and not represented in the
   current `cache_coverage` response.
4. **Workflow readiness** — the minimum evidence the specific workflow
   consumes; it is not synonymous with every cache being populated.
5. **Searched vs matched** — a provider can be searched completely while
   legitimately returning no match. `searched` excludes error rows;
   `has_result` includes only exact/fuzzy results; an error is incomplete and
   retryable. Do not require `has_result=100%`.

## Commands you will need

| Purpose                                | Command                                                                                                                                                                               | Expected on success                  |
| -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| Remove impossible gates                | `rg -n -i -e "all (enrichment )?providers.*100%" -e "all providers at 100%" -e "all providers should be at 100%" site/src/content/docs site/src/partials/sops`                        | exit 1; no matches                   |
| Remove scoring-enrichment prerequisite | `rg -n -e "scoring uses cached audio analysis and enrichment" -e "Enrichment caches should be populated" site/src/content/docs/workflows site/src/partials/sops`                      | exit 1; no matches                   |
| Remove wrong error-row claim           | `rg -n -i -e "including .*error results" -e "error results.*count.*searched" site/src/content/docs/mcp-tools/library-data.mdx site/src/content/docs/workflows site/src/partials/sops` | exit 1; no matches                   |
| Preserve actual hydrate list           | `rg -n -e "discogs,beatport,analysis" -e "Discogs \+ Beatport \+ audio analysis" site/src/content/docs/cli/index.mdx`                                                                 | exit 0                               |
| Format                                 | `dprint check`                                                                                                                                                                        | exit 0                               |
| Runtime build                          | `cargo build --release`                                                                                                                                                               | exit 0; embedded SOP changes compile |
| MCP smoke                              | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`                                                                                           | exit 0; no protocol violations       |
| Site build                             | `cd site && npm ci && npm run build`                                                                                                                                                  | exit 0                               |

## Suggested executor toolkit

- Use the Browser skill if available to inspect the rendered enrichment data
  flow and the prerequisite sections for cleanup, metadata backfill,
  classification, set building, and pool building.
- Use the repo's live release binary/tool schema as the source of truth. Do not
  infer provider support from brand names mentioned elsewhere in prose.

## Scope

**In scope** (the only source/documentation files you may modify):

- `site/src/components/DataFlowDiagram.astro`
- `site/src/content/docs/cli/index.mdx`
- `site/src/content/docs/mcp-tools/library-data.mdx`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `site/src/content/docs/workflows/library-cleanup.mdx`
- `site/src/content/docs/workflows/metadata-backfill.mdx`
- `site/src/content/docs/workflows/genre-classification.mdx`
- `site/src/content/docs/workflows/genre-audit.mdx`
- `site/src/content/docs/workflows/set-building.mdx`
- `site/src/content/docs/workflows/pool-building.mdx`
- `site/src/content/docs/workflows/chapter-set-planning.mdx`
- `site/src/partials/sops/metadata-backfill.mdx`
- `site/src/partials/sops/genre-classification.mdx`
- `site/src/partials/sops/genre-audit.mdx`
- `site/src/partials/sops/set-building.mdx`
- `site/src/partials/sops/pool-building.mdx`
- `site/src/partials/sops/chapter-set-planning.mdx`
- `plans/README.md` for the status row only

**Out of scope**:

- Adding Bandcamp or MusicBrainz to CLI `hydrate` or `cache_coverage`.
- Changing classification, backfill, scoring, or provider runtime behavior.
- Fixing batch pagination/completion loops; Plan 028 owns that behavior and the
  associated SOP iteration details.
- Requiring provider matches for every track; legitimate no-match entries are
  complete searches.
- Making Essentia mandatory. Stratum is built in; scoring degrades when
  optional Essentia fields are absent.
- Editing the canonical workflow catalog; Plan 026 will encode the corrected
  contracts after this plan lands.

## Git workflow

- Branch: `codex/023-define-capability-readiness-contracts`
- Use Conventional Commits; preferred final message:
  `docs(workflows): align readiness with data usage`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Document the three distinct coverage surfaces

In `site/src/content/docs/cli/index.mdx`, retain the accurate hydrate provider
table and examples, but remove the claim that full hydration is recommended
before set building. Add one short note that Bandcamp/MusicBrainz are not CLI
hydrate providers.

In `site/src/content/docs/mcp-tools/library-data.mdx`, state exactly which four
entries `cache_coverage` returns and distinguish `searched_percent` from
`has_result_percent`: searched counts non-error rows including no-match,
has-result counts exact/fuzzy only, and error rows count as neither and remain
retryable.

In `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`, distinguish CLI
hydrate from MCP `enrich_tracks`, including the latter's Bandcamp support.
Do not claim a MusicBrainz batch provider exists.

**Verify**:

```bash
rg -n "Stratum|Essentia|Discogs|Beatport|Bandcamp|MusicBrainz" site/src/content/docs/cli/index.mdx site/src/content/docs/mcp-tools/library-data.mdx site/src/content/docs/mcp-tools/enrichment-analysis.mdx
```

Expected: exit 0; each source is described only on the surfaces that support it.

### Step 2: Correct Library Cleanup and Metadata Backfill checkpoints

Update Session 2 in `library-cleanup.mdx` to say `hydrate` warms Discogs,
Beatport, Stratum, and optional Essentia caches. Replace the impossible
all-provider checkpoint with a concrete CLI completion condition: the selected
hydrate work finished without pending failures; cached no-match entries are
valid completed searches.

Update Session 3 and the skipping guidance to explain that Metadata Backfill
can use targeted Bandcamp/MusicBrainz work for remaining gaps. It must not claim
Session 2 already made those sources 100% complete.

In the human Metadata Backfill page and its SOP:

- describe `cache_coverage` as the Discogs/Beatport/audio baseline;
- use backfill result gaps, targeted `enrich_tracks`, `auto_enrich`, individual
  MusicBrainz lookup, and web research for additional sources;
- never ask the user to find Bandcamp/MusicBrainz percentages in a response
  that does not contain them;
- never require provider matches for genuinely unavailable releases.

Leave exact batch advancement to Plan 028; do not add another unchanged
"repeat until 100%" loop.

**Verify**:

```bash
rg -n -i "Bandcamp.*cache_coverage|MusicBrainz.*cache_coverage|Session 2.*100%" site/src/content/docs/workflows site/src/partials/sops/metadata-backfill.mdx
```

Expected: exit 1; no impossible relationship remains.

### Step 3: Define classification readiness as evidence quality

For Genre Classification and Genre Audit, in both human pages and SOPs:

- use scoped `cache_coverage` to report available Discogs/Beatport/audio
  evidence;
- recommend hydrating missing core searches/audio for the selected scope;
- allow the workflow to proceed when evidence is incomplete, with affected
  tracks classified as low/insufficient confidence and reviewed individually;
- state that Essentia improves evidence but is not a universal hard gate;
- use searched coverage rather than match coverage as the completeness signal.

Keep human approval requirements and the existing confidence decision tree.

**Verify**:

```bash
for file in \
  site/src/content/docs/workflows/genre-classification.mdx \
  site/src/content/docs/workflows/genre-audit.mdx \
  site/src/partials/sops/genre-classification.mdx \
  site/src/partials/sops/genre-audit.mdx; do
  rg -qi "insufficient" "$file"
  rg -qi "searched" "$file"
  rg -qi "Essentia" "$file"
done
```

Expected: every per-file assertion exits 0; each concept appears in every
human and agent surface rather than merely somewhere in the combined set.

### Step 4: Remove irrelevant enrichment gates from creative scoring

For Set Building, Pool Building, and Chapter Set Planning, in both human and
agent pages:

- remove Discogs/Beatport/Bandcamp/MusicBrainz coverage prerequisites;
- keep only the candidate-pool and tool-parameter requirements as hard gates;
- recommend sufficiently complete Rekordbox BPM/key/genre metadata for useful
  scoring, while stating that missing values degrade the corresponding axes
  rather than blocking the tool; each of the six files must contain the plain
  contract "Missing BPM, key, or genre degrades the corresponding scoring axis;
  it does not block the workflow" or a grammatically equivalent sentence that
  matches the verification expression below;
- describe Stratum as the built-in BPM/key fallback/evidence source;
- recommend Essentia for energy, brightness, rhythm, and timbral quality, while
  explaining graceful degradation;
- keep other real prerequisites, such as locked chapter playlists.

Do not imply that exporting a playlist is itself read-only; distinguish
read-only analysis from writing an XML output file.

**Verify**:

```bash
creative_files=(
  site/src/content/docs/workflows/set-building.mdx
  site/src/content/docs/workflows/pool-building.mdx
  site/src/content/docs/workflows/chapter-set-planning.mdx
  site/src/partials/sops/set-building.mdx
  site/src/partials/sops/pool-building.mdx
  site/src/partials/sops/chapter-set-planning.mdx
)
! rg -n -i "Discogs|Beatport|Bandcamp|MusicBrainz" "${creative_files[@]}"
for file in "${creative_files[@]}"; do
  rg -qi "Missing BPM, key, or genre degrades the corresponding scoring axis" "$file"
  rg -qi "does not block the workflow" "$file"
done
```

Expected: the provider grep has no matches and all six human/agent files carry
both clauses of the explicit graceful-degradation sentence. Do not retain
provider names merely to say they are unnecessary; the capability reference
already owns that fact.

### Step 5: Correct the visual data flow

Update `site/src/components/DataFlowDiagram.astro` so:

- Stratum and Essentia outputs can feed classification and scoring;
- Discogs and Beatport outputs feed classification/backfill, not Set Building;
- labels are accessible without relying on arrow color alone;
- the displayed Stratum/Essentia outputs agree with the adjacent reference
  prose after this plan.

Keep the component server-rendered and responsive. Do not add client JavaScript.

**Verify**:

```bash
cd site && npm run build
test -f dist/mcp-tools/enrichment-analysis/index.html
```

Expected: both commands exit 0.

### Step 6: Rebuild the embedded SOP and rendered site surfaces

Run the release build because six SOP partials are embedded by `help_handler`.
Run MCP smoke and inspect the rendered prerequisite sections at desktop and
mobile widths. Confirm the diagram remains legible and no workflow says "all
providers at 100%."

**Verify**:

```bash
dprint check
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
cd site && npm run build
```

Expected: every command exits 0; smoke reports no protocol violations.

## Test plan

- No Rust behavior is changed, so do not add provider mocks.
- Exact greps are regression oracles for impossible universal gates.
- The release build verifies every changed SOP remains valid `include_str!`
  input.
- The Astro build validates MDX and the data-flow component.
- Browser QA must cover the diagram plus at least one cleanup, one
  classification, and one creative workflow at desktop/mobile widths.

## Done criteria

- [ ] CLI hydrate is documented as Discogs, Beatport, and analysis only.
- [ ] `cache_coverage` is documented as Stratum, Essentia, Discogs, and Beatport only.
- [ ] Searched/matched/error cache semantics match the store queries exactly.
- [ ] Metadata Backfill explains the separate Bandcamp/MusicBrainz paths.
- [ ] Classification/audit use scoped evidence quality rather than an impossible universal gate.
- [ ] Set, pool, and chapter workflows have no enrichment-cache prerequisite and do not turn missing BPM/key/genre into a false hard gate.
- [ ] The data-flow diagram no longer routes metadata providers into Set Building.
- [ ] The zero-match universal-gate greps pass.
- [ ] `dprint check`, release build, MCP smoke, and site build all pass.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- The live CLI provider enum or `cache_coverage` response differs from the
  matrix above.
- Scoring now reads an enrichment provider cache.
- Classification runtime enforces a hard coverage threshold that the audit did
  not observe.
- The product owner wants one new universal hydration command instead of
  documentation alignment; that is a separate runtime feature decision.
- Correctness requires inventing a percentage for Bandcamp/MusicBrainz that no
  tool can currently compute.

## Maintenance notes

- Every new provider must be documented separately for CLI hydration, MCP
  enrichment, coverage reporting, backfill consumption, and scoring
  consumption; presence in one does not imply presence in all.
- Reviewers should search for "all providers" whenever provider surfaces change.
- Plan 026 must encode these corrected requirements in the workflow catalog.
- Plan 028 may change batch selection/completion mechanics but must preserve
  this capability matrix.
