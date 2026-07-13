# Plan 032: Reconcile remaining public references and examples

> **Executor instructions**: This is the portfolio's final integrated accuracy
> sweep. Start only from a reviewed base containing every dependency, refresh
> all evidence from that head, and follow the steps in order. Stop on any STOP
> condition rather than changing product behavior to preserve old prose. Update
> this plan's row in `plans/README.md` only if the orchestrator/reviewer does not
> own the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat 3451803..HEAD -- README.md src/audio.rs src/audio_profile.rs src/beatport.rs src/classify.rs src/color.rs src/discogs.rs src/genre.rs src/main.rs src/store.rs src/xml.rs src/tools/analysis.rs src/tools/audio_handlers.rs src/tools/help_handler.rs src/tools/mod.rs src/tools/params.rs src/tools/staging_handlers.rs src/tools/tests.rs src/tools/classify_handler.rs src/tools/pool_handlers.rs src/tools/resolve.rs src/tools/resolve_handlers.rs src/tools/sequencing_handlers.rs scripts/mcp-smoke.mjs scripts/release.sh .github/workflows/docs-pages.yml site/astro.config.mjs site/package.json site/package-lock.json site/src/data/workflows.mjs site/src/data/tool-reference.mjs site/vendor/starlight-llms-txt site/src/content/docs site/src/partials/sops/batch-import.mdx site/src/partials/sops/metadata-backfill.mdx scripts/check-doc-contract.mjs scripts/check-doc-contract.test.mjs
> ```
>
> Plans 027–031 and all of their dependencies must be integrated. Treat the
> excerpts below as findings to recheck, not text to apply blindly. If an
> example requires a new API/filter or a narrative correction requires changing
> scoring/classification behavior, stop and report it separately.

## Status

- **Priority**: P2
- **Effort**: M
- **Risk**: LOW
- **Depends on**: 027, 028, 029, 030, 031, 033
- **Category**: docs / final integration
- **Planned at**: commit `3451803`, 2026-07-12

## Why this matters

After the targeted safety, workflow, runtime, and publishing plans land, a set
of smaller but high-trust errors remains across copy-paste DJ prompts, MCP
reference pages, scoring explanations, XML instructions, environment variables,
audio capability summaries, batch-import commands, and runtime help. Each can
send a user toward a nonexistent filter, the wrong value scale, unsupported
catalog research, or a dead/incorrect workflow instruction.

This final plan reconciles those surfaces against the integrated shipped
interfaces and extends the automated gate where the fact is mechanically
stable. It must not add capabilities merely to make stale documentation true.

## Current state

### Checklist to re-verify

At planning commit `3451803`:

1. **Tool/taxonomy inventory**
   - `site/src/content/docs/mcp-tools/index.mdx:10,35-38` says 51 total and 11
     Classification & Staging tools; the release binary reports 53 total and
     that group has 13. Plan 027 should already canonicalize this—verify rather
     than re-hardcode it.
   - `site/src/content/docs/mcp-tools/classification-staging.mdx:14-16` says 54
     canonical genres; `src/genre.rs:6-62` contains 55.
2. **Resolved/cache data**
   - `site/src/content/docs/mcp-tools/library-data.mdx:123-165` says “all cached
     enrichment” and that coverage includes error rows.
   - `src/tools/resolve_handlers.rs:30-58` resolves Discogs, Beatport, current
     Stratum, current Essentia, plus staged changes—not Bandcamp/MusicBrainz.
   - `src/store.rs:342-345` excludes `match_quality='error'` from batch coverage.
   - Plan 027 exposes `format` on `cache_coverage` because the tool reuses
     `ResolveTracksDataParams`, but `handle_cache_coverage` never reads that
     field. The final public schema must not advertise an ignored response
     format.
   - `site/src/content/docs/mcp-tools/enrichment-analysis.mdx` overstates
     lookup outputs: `DiscogsResult` has title, year, label, genres, styles,
     URL, cover image, and fuzzy-match state—not catalog number/credits/general
     release metadata—and `BeatportResult` has one genre plus BPM, key, track,
     artists, date, label, and fuzzy-match state—not a separate sub-genre.
3. **DJ prompt capabilities**
   - `site/src/content/docs/workflows/dj-prompts.mdx:39` asks
     `search_tracks` for an energy range, but `SearchTracksParams` has no energy
     filter.
   - Lines 74 and 94-104 use `lookup_discogs` for recent catalog gaps, credits,
     collaborators, and release discovery. `src/discogs.rs:127-138` returns a
     matched release result, not a general catalog/credits browser.
   - Lines 196-198 call a transition score in the 60–80 range; the public score
     axes/composite use 0–1.
4. **Scoring/taxonomy prose**
   - `site/src/content/docs/reference/transition-scoring.md:10-13` calls
     `build_set` greedy; live `BuildSetParams` defaults `beam_width` to 3 and
     width 1 is greedy.
   - Its House family table omits `Italodance`, which the source family mapping
     includes.
5. **XML inputs**
   - `site/src/content/docs/reference/xml-export.mdx:77-90` tells callers to use
     hex colors. `src/tools/staging_handlers.rs:41-50` accepts the canonical
     color names and rejects other strings; XML serialization produces hex.
6. **Audio analysis**
   - `README.md:105-106`, environment, architecture, and MCP docs reduce
     Stratum to BPM/key or BPM/key/rhythm. `src/audio.rs:30-105` exposes
     confidence/grid provenance, decay evidence, dub-stab, kick pattern, and
     structural sections in addition to BPM/key.
   - Docs must distinguish returned evidence from fields actually consumed by
     classification, transition scoring, pool scoring, audit, calibration, and
     audio-profile summaries. Essentia does not return a field named `energy`;
     scoring derives energy from danceability, integrated loudness, and onset
     rate.
7. **Environment surface**
   - `site/src/content/docs/reference/environment-variables.md:66-68`
     advertises `REKLAWDBOX_CORPUS_PATH`; `src/main.rs:13-18` compiles the corpus
     module only for tests, so this is not a public production variable.
8. **Batch Import file coverage**
   - `site/src/partials/sops/batch-import.mdx:265-268` moves only WAV, FLAC,
     and MP3; `src/audio.rs` supports FLAC, WAV, MP3, M4A, AAC, and AIFF.
9. **Runtime help/label gate**
   - Baseline help lists a stale workflow order, universal hydration
     prerequisite, dead `/reference/tools/` link, and omits `album` from its
     visible topic tip even though the handler accepts it.
   - `src/tools/mod.rs`, `src/tools/params.rs`, and
     `src/tools/staging_handlers.rs` call label research “step 3”; the embedded
     Metadata Backfill SOP names it Step 1c.
   - The public help-topic vocabulary is the twelve values advertised by
     `HelpParams` schema—`genre`, `genre audit`, `set`, `pool`, `chapter`,
     `audit`, `import`, `metadata`, `label`, `year`, `album`, and `health`—not
     every private substring alias accepted by handler matching.

Plans 023, 027, 028, and 029 intentionally change several integrated facts.
Document the final behavior, not both the old and new versions.

## Commands you will need

| Purpose            | Command                                                                                                                                                                                                                                                | Expected on success                                           |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------- |
| Runtime evidence   | `cargo build --release && node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000`                                                                                                                                   | exit 0; integrated tool/help summary captured                 |
| Old public phrases | `rg -n -e "score 60-80" -e "54 genres" -e "REKLAWDBOX_CORPUS_PATH" -e "Use these hex values" -e "during greedy sequencing" -e "all cached enrichment" -e "including .*error results" -e "/reference/tools/" README.md site/src/content/docs src/tools` | exit 1 after corrections, subject to exact integrated wording |
| Help tests         | `cargo test -p reklawdbox help_public_contract -- --nocapture`                                                                                                                                                                                         | exit 0; route/public topics/checkpoints/shape pass            |
| Docs contract      | `node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist`                                                                                                                                                             | exit 0                                                        |
| Full gates         | `cargo fmt --check && dprint check && cargo clippy -p reklawdbox --all-targets -- -D warnings && cargo test -p reklawdbox --no-fail-fast`                                                                                                              | exit 0                                                        |
| Site               | `cd site && npm run build`                                                                                                                                                                                                                             | exit 0                                                        |

## Scope

**In scope — runtime user-visible text/tests only**:

- `src/tools/help_handler.rs`
- `src/tools/mod.rs`
- `src/tools/params.rs`
- `src/tools/staging_handlers.rs`
- `src/tools/resolve.rs` only to share selector/scope formatting between the
  dedicated resolve and cache-coverage parameter types without copying it
- `src/tools/resolve_handlers.rs` only for the dedicated cache-coverage input
  type wiring described in Step 2
- `src/tools/tests.rs`
- `scripts/mcp-smoke.mjs`
- `scripts/check-doc-contract.mjs`
- `scripts/check-doc-contract.test.mjs`
- `.github/workflows/docs-pages.yml` and `scripts/release.sh` only to add the
  new source-backed oracle files to the existing docs-contract trigger set

**In scope — public docs/examples**:

- `README.md`
- `site/src/content/docs/mcp-tools/index.mdx`
- `site/src/content/docs/mcp-tools/library-data.mdx`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `site/src/content/docs/mcp-tools/classification-staging.mdx`
- `site/src/content/docs/mcp-tools/mixing.mdx` only if integrated help/schema
  wording requires alignment
- `site/src/content/docs/reference/transition-scoring.md`
- `site/src/content/docs/reference/xml-export.mdx`
- `site/src/content/docs/reference/environment-variables.md`
- `site/src/content/docs/concepts/architecture.mdx`
- `site/src/content/docs/concepts/index.mdx` only for the audio/provider summary
- `site/src/content/docs/workflows/dj-prompts.mdx`
- `site/src/partials/sops/batch-import.mdx`
- `plans/README.md` for the status row only

**Evidence only; out of scope**:

- Runtime/source evidence: `src/audio.rs`, `src/audio_profile.rs`,
  `src/beatport.rs`, `src/classify.rs`, `src/color.rs`, `src/discogs.rs`,
  `src/genre.rs`, `src/main.rs`, `src/store.rs`, `src/xml.rs`,
  `src/tools/analysis.rs`, `src/tools/audio_handlers.rs`,
  `src/tools/classify_handler.rs`, `src/tools/pool_handlers.rs`,
  `src/tools/scoring.rs`, and `src/tools/sequencing_handlers.rs` unless a
  separate reviewed defect is discovered.
- Integrated contract/publishing evidence: `site/src/data/workflows.mjs`,
  `site/src/data/tool-reference.mjs`, `site/astro.config.mjs`,
  `site/package.json`, `site/package-lock.json`,
  `site/vendor/starlight-llms-txt/**`, and
  `site/src/partials/sops/metadata-backfill.mdx`. Re-read these surfaces but do
  not change Plan 026, 030, or 031 contracts in this sweep.

**Also out of scope**:

- New search, catalog, credits, or music-discovery APIs.
- Scoring/taxonomy/audio algorithm changes or new file-format support.
- Re-enabling the test-only corpus in production.
- Navigation/audience/vendor work owned by Plans 030–031.
- Changing runtime behavior solely to retain a stale example.

## Git workflow

- Branch: `codex/032-refresh-public-reference-contracts`
- Preferred commit: `docs(reference): reconcile public surfaces with runtime`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Capture integrated runtime evidence

Build the reviewed dependency head and collect, without a private DB:

- complete `tools/list` names, descriptions, input schemas, and any integrated
  output schemas; derive groups/routes from Plan 027's
  `site/src/data/tool-reference.mjs`, and record defaults only where JSON Schema
  or zero-exit CLI help structurally exposes them;
- root CLI and every subcommand help;
- `help()` menu and each of the twelve schema-advertised public topics,
  including `album`; private substring aliases are evidence only, not part of
  the promised vocabulary;
- Plan 026 workflow order and Plan 028 continuation fields;
- Plan 029 backup/path behavior from tests;
- canonical taxonomy count through `get_genre_taxonomy` if it is DB-free;
- built routes, Pagefind policy, sitemap, and generic/custom LLM outputs.

Recheck every item in the current-state checklist. Mark an item no-op if an
earlier plan already fixed it, but preserve/extend its regression assertion.
Unexpected behavior differences are STOP conditions, not copy-edit prompts.

**Verify**:

```bash
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
./target/release/reklawdbox --help
cd site && npm run build && cd ..
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: every command exits 0 on the integrated dependency head. Save the
tool count, help topics/routes/order, CLI inventory, and generated-output
presence in the review notes before editing; any unexpected failure is a STOP.

### Step 2: Reconcile inventories and resolved-data claims

Ensure the MCP overview obtains group/total counts from Plan 027's canonical
mapping rather than hard-coded prose. For taxonomy, either derive the displayed
count from a canonical import/build component or omit the volatile number and
let `get_genre_taxonomy` remain authoritative; do not maintain “55” in a second
unverified place.

Describe resolved data exactly:

- Rekordbox metadata plus cached Discogs/Beatport;
- current Stratum and optional current Essentia results;
- staged changes;
- no implied Bandcamp/MusicBrainz payload unless integrated code now returns it.

If `cache_coverage` still shares `ResolveTracksDataParams`, introduce a
dedicated `CacheCoverageParams` with the same live filters, `track_ids`,
`playlist_id`, and `max_tracks`, but no ignored `format`. Wire only that tool
and handler to the dedicated type; factor the existing scope-description helper
over those shared selector fields rather than copying its logic. Preserve
selection, result shape, and
`resolve_tracks_data`'s real `format` behavior. Add a DB-free schema assertion
that compares both property sets and proves their only difference is
`format`: `cache_coverage` omits it while `resolve_tracks_data` retains it.
Remove the no-op row from the reference table. Do not implement format
behavior merely to keep the accidental schema surface.

Align coverage semantics with Plan 023 and live store queries: searched
no-match may count as completed search, error rows do not count as successful
coverage, and result/match quality is separate.

Also correct the lookup result tables: Discogs exposes title, year, label,
genres, styles, URL, cover image, and fuzzy-match state; Beatport exposes one
genre plus BPM, key, track, artists, date, label, and fuzzy-match state. Do not
promise Discogs catalog number/credits/general release browsing or a distinct
Beatport sub-genre.

These provider/output/error semantics are not derivable from current DB-free
`tools/list` because the affected tools have no output schema. Keep them in the
semantic audit with targeted one-time searches and direct source evidence; do
not add phrase-presence markers or Node snapshots merely to make the checker
green. Only group/total/route facts from `tool-reference.mjs` remain mechanical.

**Verify**:

```bash
rg -n -e "Discogs" -e "Beatport" -e "Stratum" -e "Essentia" -e "staged" -e "no.match" -e "error" site/src/content/docs/mcp-tools/library-data.mdx site/src/content/docs/mcp-tools/enrichment-analysis.mdx site/src/content/docs/mcp-tools/index.mdx
! rg -n -e "all cached enrichment" -e "including .*error results" site/src/content/docs/mcp-tools
! rg -n -e "catalog number, and release metadata" -e "genre, sub-genre" site/src/content/docs/mcp-tools/enrichment-analysis.mdx
cargo test -p reklawdbox cache_coverage_public_schema -- --nocapture
```

Expected: the positive search shows the exact integrated providers/states, old
overbroad lookup/cache claims have no matches, and mechanical group/total/route
checks remain owned by Plan 027 without narrative snapshots.

### Step 3: Correct scoring and genre-family reference prose

Update Transition Scoring to state:

- `build_set` uses beam search by default;
- `beam_width=1` is the greedy specialization;
- standalone `score_transition` and sequence-context modifiers have their
  distinct scopes;
- Italodance belongs in the live House family;
- all axes and composites use the actual 0–1 scale.

Cross-check formulas, defaults, family members, and fallback behavior directly
against integrated source/tests. Do not opportunistically rewrite correct Pool
Discovery greedy-expansion prose; that is a different algorithm.

Add focused Rust or checker assertions for the default beam width/family
membership only if they can consume canonical source/runtime facts without
copying the algorithm.

**Verify**:

```bash
rg -n -e "beam search" -e "beam_width.*1" -e "Italodance" -e "0 and 1" site/src/content/docs/reference/transition-scoring.md
! rg -n "during greedy sequencing" site/src/content/docs/reference/transition-scoring.md
node --test scripts/check-doc-contract.test.mjs
```

Expected: beam/default/greedy specialization, House membership, and 0–1 scale
are present; stale Build Set greedy prose is absent; canonical-source fixtures
pass without changing scoring code.

### Step 4: Rewrite unsupported DJ prompt promises

For each prompt, keep the Plan 026/030 DJ contract intact:

- select by supported `search_tracks` filters, then resolve audio/scoring data
  to assess energy; do not claim an energy-range search filter;
- use `lookup_discogs` only for a specific track/release match and its returned
  fields;
- remove claims that it browses recent label catalogs, credits, remixers,
  collaborators, compilations, or missing releases;
- make local collection directions the default; provider lookup is conditional
  and only evaluates a concrete track or release candidate supplied by the
  user;
- defer open-ended catalog discovery entirely. Do not direct the recipe to an
  external search, browse catalogs, or introduce a new network prerequisite;
- replace “score 60–80” with qualitative practice selection grounded in the
  0–1 composite and axis breakdown. Do not invent another rigid band without
  product evidence.

Run the SOP/tool-call parser against every copy-paste block. A useful prompt
that requires a new filter/API must be reduced or deferred, not implemented in
this plan. The parser proves tool/argument validity only; it cannot prove
natural-language claims about catalog browsing, energy filters, or score bands.
Keep those claims in this semantic review and targeted search instead of adding
phrase snapshots to the automated checker.

**Verify**:

```bash
! rg -n -e "search_tracks.*energy range" -e "Discogs credits" -e "recent releases you might be missing" -e "score 60-80" site/src/content/docs/workflows/dj-prompts.mdx
rg -n -e "user.supplied" -e "0.*1" -e "axis" site/src/content/docs/workflows/dj-prompts.mdx
! rg -n -e "browse.*catalog" -e "discover.*release" -e "external.*web" site/src/content/docs/workflows/dj-prompts.mdx
cd site && npm run build && cd ..
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: unsupported promises and open-ended discovery instructions have no
matches, user-supplied candidate and actual score semantics are explicit, the
rebuilt route is current, and every copy-paste tool call/argument passes the
live checker without narrative snapshots.

### Step 5: Correct XML color input and preserve prior handoffs

Rewrite the color section so callers pass the eight canonical color **names**
accepted by `update_tracks`; show hex only as the serialized Rekordbox XML
representation. The live input schema describes `color` only as a string, so it
is not the vocabulary oracle. Bracket the color name/hex table with one narrow
`doc-contract:color-palette source=src/color.rs` marker and extend the checker
with a bounded parser for the `COLORS` constant. Compare the exact eight
name/code pairs to the marked table; use `src/xml.rs` and a focused Rust test as
the evidence that those integers serialize as `0xRRGGBB`. Add negative fixtures
for a missing name, wrong hex value, and duplicate row.

Preserve Plan 025's metadata-versus-playlist import split and Plan 029's
fail-closed backup/path wording. Add a focused tool-validation test proving a
canonical name succeeds validation and a hex input is rejected with the valid
names, without requiring a real DB.

**Verify**:

```bash
cargo test -p reklawdbox -- --list | rg "color_input_public_contract"
cargo test -p reklawdbox color_input_public_contract -- --nocapture
rg -n -e "color names" -e "XML.*hex" -e "metadata" -e "playlist" -e "backup" site/src/content/docs/reference/xml-export.mdx
node --test scripts/check-doc-contract.test.mjs
```

Expected: the named DB-free validation tests exist and pass, docs distinguish
name input from serialized hex, the source-constant palette fixtures pass, and
Plans 025/029 handoff/backup sections remain.

### Step 6: Describe Stratum outputs and consumers accurately

Update README, architecture, environment, and enrichment/analysis reference so
they distinguish:

- **returned/cached Stratum evidence**: BPM/key and confidence, grid
  provenance/stability, decay, dub-stab timing/template evidence, kick pattern,
  and structural sections when available;
- **optional Essentia evidence**: integrated loudness, danceability, brightness,
  onset/rhythm, and other spectral/timbral features; it has no returned
  `energy` field;
- **derived energy**: scoring combines Essentia danceability, integrated
  loudness, and onset rate; label it as derived rather than returned analysis;
- **consumers**: only the subset each classifier, transition scorer, pool
  scorer, audit, calibration path, or audio-profile summary actually reads.

The canonical detailed table must account for every public field family in
`StratumResult` and `EssentiaOutput`, including analysis metadata/timing/schema
versions, modulation-centroid and harmonic-proportion evidence,
histograms/rate bases, and flags/warnings. Group related fields for readable UX;
if a row intentionally summarizes rather than enumerates a family, label it
non-exhaustive and link the live schema instead of presenting a partial list as
complete.

Do not say every Stratum field affects every downstream score. Mark optional
fields/failure modes, schema/file/grid freshness, and no-grid fallback
consistently with integrated code. Keep the overview concise and link to one
canonical detailed table. Inspect `src/audio_profile.rs`, `src/classify.rs`, and
the classification, sequencing, and pool handlers as well as `src/audio.rs`;
do not infer consumers from similarly named output fields.

**Verify**:

```bash
rg -n -e "grid" -e "confidence" -e "decay" -e "dub.stab" -e "kick pattern" -e "sections" -e "Essentia" -e "derived energy" README.md site/src/content/docs/concepts/architecture.mdx site/src/content/docs/reference/environment-variables.md site/src/content/docs/mcp-tools/enrichment-analysis.mdx
rg -n -e "classification" -e "transition" -e "pool" -e "calibration" -e "audio profile" site/src/content/docs/mcp-tools/enrichment-analysis.mdx
cd site && npm run build && cd ..
```

Expected: returned Stratum evidence, optional Essentia evidence, and consumer
subsets—including derived energy and calibration/audio-profile use—are explicit
in the canonical detailed page; overview pages link or summarize without
claiming universal consumption; the site builds.

### Step 7: Remove nonproduction environment claims and complete Batch Import

Remove `REKLAWDBOX_CORPUS_PATH` from public production environment docs. Do not
relocate or document the test-only variable elsewhere in this plan. Verify no
production code reads it.

Update the Batch Import non-overwriting move command to include all live
`AUDIO_EXTENSIONS`: FLAC, WAV, MP3, M4A, AAC, and AIFF. Preserve `-maxdepth 1`,
quoting, `mv -n`, cover handling, verification, and empty-directory behavior.
Bracket exactly one shell block with a narrow
`doc-contract:batch-audio-extensions source=src/audio.rs` marker. Extend the
Node checker to parse its `-iname` extension set and compare it exactly with
`AUDIO_EXTENSIONS` in `src/audio.rs`; reject missing, extra, and duplicate
extensions with dedicated fixtures. Do not add a runtime export solely for the
docs gate.

Because the SOP is embedded in runtime help, rebuild before smoke validation.

**Verify**:

```bash
! rg -n "REKLAWDBOX_CORPUS_PATH" site/src/content/docs/reference/environment-variables.md
rg -n -e '\*\.flac' -e '\*\.wav' -e '\*\.mp3' -e '\*\.m4a' -e '\*\.aac' -e '\*\.aiff' -e 'mv -n' site/src/partials/sops/batch-import.mdx
node --test scripts/check-doc-contract.test.mjs
cargo build --release
```

Expected: the public production table has no corpus variable, all six canonical
extensions plus non-overwrite behavior are present, the source-set fixture
passes, and embedded help rebuilds.

### Step 8: Finalize runtime help and label-gate wording

Align `help()` with the integrated canonical workflow module/order and public
routes:

- no universal “hydrate everything first” prerequisite;
- provider/workflow readiness wording from Plan 023;
- Reload Tag and XML handoffs at their real checkpoints;
- `/mcp-tools/` as the reference route;
- exactly the twelve topics advertised by `HelpParams`—including `album`—in
  the visible topic tip; private substring aliases remain unadvertised;
- label research consistently named **Step 1c** (or a stable section title if
  the integrated SOP has moved), not step 3.

Update the schema descriptions and label-gate error without changing gate
behavior. Plan 027's Node checker remains the only test that compares the
canonical 11-workflow order and 9/7 subsets to `workflows.mjs`; do not duplicate
that sequence in Rust. Add `help_public_contract` Rust tests for the public
route, exact schema-advertised topic vocabulary/tip, checkpoint wording, and
DB-free response shape. MCP smoke should assert more than one help topic and
remain DB-free.

**Verify**:

```bash
cargo test -p reklawdbox help_public_contract -- --nocapture
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
! rg -n -e "/reference/tools/" -e "step 3.*label research" src/tools site/src/content/docs
rg -n -e "album" -e "/mcp-tools/" -e "Step 1c" src/tools/help_handler.rs src/tools/mod.rs src/tools/params.rs src/tools/staging_handlers.rs
```

Expected: help tests and multi-topic smoke pass without a DB; dead route and old
checkpoint have no matches; album, live route, and stable label section appear.

### Step 9: Extend stable automated sentinels

Use Plan 027's gate for facts that can be derived:

- complete tool counts and groups/routes from `tool-reference.mjs`;
- taxonomy count from a DB-free source/runtime oracle or count-free rendering;
- exact schema-advertised public help topics and public routes;
- copy-paste tool/argument validity;
- the marked Batch Import extension block against `AUDIO_EXTENSIONS` in
  `src/audio.rs`;
- the marked color table against `COLORS` in `src/color.rs`;
- Plan 031's generated audience rules.

Add `src/audio.rs`, `src/color.rs`, and `src/genre.rs` to both the docs CI path
filter and release docs-sensitive predicate. These files are now direct
oracles for the extension, color, and taxonomy checks; a source-only change
must execute the gate.

Extend Plan 030's pure trigger-inventory assertion rather than using text
search. Each of the three new paths must appear in all three parsed inventories:
CI `on.push.paths`, CI `on.pull_request.paths`, and release
`docs_contract_changed()`. Add a negative fixture for each path/inventory class
and preserve the earlier required trigger set, including `Cargo.toml` and
`Cargo.lock`.

Keep unsupported energy-filter/catalog language, old score scale, and
provider/error/coverage semantics in the targeted semantic searches from the
earlier steps: current live schemas do not expose authoritative output
contracts for them. Do not encode full narrative snapshots. Tests must fail
with file/line and the expected canonical source.

**Verify**:

```bash
node --test scripts/check-doc-contract.test.mjs
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: unit/live gates exit 0 and each new sentinel has a negative fixture
whose diagnostic names its canonical source and public file/line.

### Step 10: Run the final portfolio gate and visual review

Run focused help/color tests, formatter, clippy, full crate tests, release build,
MCP smoke, site build, and live docs checker. Then inspect in production preview
at desktop and mobile widths:

- MCP overview and resolved-data sections;
- DJ Prompts;
- Transition Scoring;
- XML Export;
- Environment Variables;
- Architecture/audio reference;
- one Batch Import agent endpoint and its dedicated plaintext output.

Confirm Plan 031's audience separation remains intact and no human search result
regains raw agent SOP duplication.

**Verify**:

```bash
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
cd site && npm run build && cd ..
node --test scripts/check-doc-contract.test.mjs
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: every command exits 0. Record desktop/mobile checks for all named
pages plus Pagefind/sitemap/generic/custom LLM assertions; no raw agent SOP
returns to human search or generic bundles.

## Test plan

- Integrated live schemas/help and source constants are the evidence oracles.
- The Node checker alone protects workflow order/subsets from `workflows.mjs`;
  focused Rust tests protect the help route/public topics/checkpoint/shape and
  color validation.
- Contract fixtures check stable copy-paste and source-set facts without prose
  snapshots.
- Release build protects the embedded Batch Import SOP.
- Generated Pagefind/sitemap/LLM checks protect Plan 031 during the final sweep.
- Desktop/mobile rendering covers every materially edited reference family.

## Done criteria

- [ ] Every checklist item is corrected or proven already fixed on integrated head.
- [ ] Tool/taxonomy counts cannot drift as independent hard-coded prose.
- [ ] Resolved-data and cache-coverage claims match the shipped providers/error semantics.
- [ ] `cache_coverage` no longer advertises the ignored `format` parameter; `resolve_tracks_data` retains its real format contract.
- [ ] No prompt promises an unsupported energy filter, catalog/credits browser, or 60–80 score scale.
- [ ] Beam search, genre family, and 0–1 scoring docs match source.
- [ ] XML accepts documented color names and preserves prior import/backup guidance.
- [ ] Audio docs distinguish returned, optional, and consumed evidence.
- [ ] Production env docs omit the test-only corpus variable.
- [ ] Batch Import handles every supported extension with non-overwriting moves.
- [ ] Runtime help order, URLs, topics, readiness, and label checkpoint are correct.
- [ ] Source-only audio-extension, color-palette, or taxonomy changes trigger the docs contract.
- [ ] Focused/full Rust, release/MCP, site, checker, format, generated-output, and visual gates pass.
- [ ] No behavior code was changed to preserve stale documentation.
- [ ] No files outside Scope are modified, except `plans/README.md` status.

## STOP conditions

Stop and report back if:

- A dependency is incomplete or integrated behavior differs from both its plan
  and tests.
- Making an example useful/accurate requires a new API or search filter.
- A scoring, taxonomy, or audio correction requires behavior/algorithm changes.
- Current supported Rekordbox instructions conflict with prior handoff plans.
- The checker can pass only through broad exceptions or narrative snapshots.
- A required change belongs in vendored publishing code already owned by Plan
  031.
- Any verification would require private audio, DB data, credentials, or
  external provider calls.

## Maintenance notes

- This is the last narrative sweep; future structural drift should fail Plan
  027's checker incrementally.
- Review advanced audio claims whenever Stratum/Essentia schema versions change.
- Review runtime help whenever workflow order, SOP topics, or public routes
  change.
- Keep volatile numeric inventories derived or machine-checked.
- Treat copy-paste prompts as executable product surface, not illustrative
  marketing text.
