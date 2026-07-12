# Plan 007: Include Rekordbox beat-grid input in Stratum cache freshness

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update the status row for this plan
> in `plans/README.md` — unless a reviewer dispatched you and told you they
> maintain the index.
>
> **Drift check (run first)**: `git diff --stat e6eb382..HEAD -- src/audio.rs src/store.rs src/tools/analysis.rs src/tools/audio_handlers.rs src/tools/classify_handler.rs src/tools/resolve_handlers.rs src/tools/scoring.rs src/tools/tests.rs src/cli/mod.rs src/cli/analyze.rs src/cli/hydrate.rs README.md site/src/content/docs/cli/index.mdx site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
> Changes made by DONE Plans 004, 005, 006, 009, 010, 011, and 013 are expected,
> especially `STRATUM_SCHEMA_VERSION`, `STORE_SCHEMA_VERSION`, Stratum tests,
> classification identities, scoring/store/audit schemas, and CLI writer
> messages. Plan 010 is included transitively through Plan 011.
> If the orchestrator starts this final-wave plan from the integrated Wave 4
> result, additive `src/tools/tests.rs` changes, the CLI-reference edits from
> DONE Plan 013 and Plans 019–020, plus the MCP-reference summary edits from
> DONE Plan 012 are also expected; preserve them and edit only the
> cache-identity paragraphs.
> Reconcile those committed results with this plan and continue when the
> cache-identity intent still matches. Any unrelated drift or contradictory
> cache redesign is a STOP.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/004-preserve-key-mode-evidence.md`, `plans/005-infer-meter-and-downbeat-phase.md`, `plans/006-stabilize-variable-tempo-beat-grids.md`, `plans/009-preserve-classification-missingness.md`, `plans/011-make-audit-freshness-complete.md` (transitively includes Plan 010), `plans/013-propagate-cli-batch-failures.md`
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Stratum analysis uses a Rekordbox ANLZ/PQTZ beat grid when available, and multiple
serialized features depend on its beat and bar positions. Cache freshness currently
compares only the audio file's size/mtime and the analyzer schema. Editing the beat
grid in Rekordbox without changing the audio file therefore leaves stale sections,
dub-stab, kick-pattern, grid stability, and grid-source results indefinitely.

This plan gives the actual Stratum grid input a stable fingerprint, stores it alongside
each cache row, and requires a match on every single and batch Stratum cache read. It
keeps Essentia freshness independent of Rekordbox grids, migrates existing local cache
databases safely, and ensures the fingerprint written is the same input snapshot that
was analyzed.

## Current state

- `src/audio.rs` — resolves a track to a read-only Rekordbox DB row, loads PQTZ, and
  passes the optional grid into Stratum.
- `src/store.rs` — local writable cache schema and all single/batch freshness helpers.
- `src/tools/analysis.rs` — MCP-side audio identity and cache lookup helpers.
- `src/tools/audio_handlers.rs` — single/batch analysis cache reads and writes.
- `src/tools/classify_handler.rs`, `resolve_handlers.rs`, `scoring.rs` — consumers of
  cached Stratum/Essentia data, including batch paths.
- `src/cli/mod.rs`, `analyze.rs`, `hydrate.rs` — CLI preflight, analysis, and queued
  cache writes.
- `src/tools/tests.rs` plus module-local tests — cache behavior coverage.
- `README.md`, the CLI reference, and the MCP enrichment/analysis reference currently
  describe cache freshness as schema-only (or file path plus schema), so all three
  claims become incomplete when the auxiliary Stratum grid identity lands.

Grid lookup is explicitly an analysis input (`src/audio.rs:396-410,490-517`):

```rust
/// Look up a track's Rekordbox-tagged beat grid by its file path.
///
/// Opens the master.db read-only, finds the track's `AnalysisDataPath`,
/// resolves it under the USBANLZ root, and parses the PQTZ tag.
pub fn load_rekordbox_grid_for_path(file_path: &str) -> Option<stratum_dsp::BeatGrid> {
    // ... read-only lookup and ANLZ parse ...
}

pub fn analyze_with_stratum(
    samples: &[f32],
    sample_rate: u32,
    external_beat_grid: Option<stratum_dsp::BeatGrid>,
) -> Result<StratumResult, AudioError> {
    let config = stratum_dsp::AnalysisConfig {
        external_beat_grid,
        ..stratum_dsp::AnalysisConfig::default()
    };
    let result = stratum_dsp::analyze_audio(samples, sample_rate, config)?;
```

The local table has no auxiliary-input identity (`src/store.rs:68-76`):

```sql
CREATE TABLE IF NOT EXISTS audio_analysis_cache (
    file_path TEXT NOT NULL,
    analyzer TEXT NOT NULL,
    file_size INTEGER NOT NULL,
    file_mtime INTEGER NOT NULL,
    analysis_version TEXT NOT NULL,
    features_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (file_path, analyzer)
);
```

Freshness compares only three values (`src/store.rs:449-461`):

```rust
pub fn is_audio_analysis_fresh(
    cached: Option<&CachedAudioAnalysis>,
    analysis_version: &str,
    file_size: i64,
    file_mtime: i64,
) -> bool {
    matches!(
        cached,
        Some(entry)
            if entry.analysis_version == analysis_version
                && entry.file_size == file_size
                && entry.file_mtime == file_mtime
    )
}
```

The MCP identity likewise contains audio metadata only
(`src/tools/analysis.rs:5-18,34-41`):

```rust
pub(super) struct AudioCacheIdentity {
    pub(super) cache_key: String,
    pub(super) file_size: i64,
    pub(super) file_mtime: i64,
}

pub(super) fn audio_cache_identity(raw_file_path: &str) -> Option<AudioCacheIdentity> {
    let cache_key = super::resolve_file_path(raw_file_path).ok()?;
    let metadata = std::fs::metadata(&cache_key).ok()?;
    Some(AudioCacheIdentity {
        cache_key,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
    })
}
```

CLI preflight repeats the same identity (`src/cli/mod.rs:239-264`). Actual CLI analysis
loads the grid only after the cache decision (`src/cli/analyze.rs:436-453`):

```rust
let path_for_grid = file_path.clone();
let stratum_result = tokio::task::spawn_blocking(move || {
    let grid = audio::load_rekordbox_grid_for_path(&path_for_grid);
    audio::analyze_with_stratum(&samples, sample_rate, grid)
})
.await?;

CliCacheWriteMsg {
    file_path: file_path.clone(),
    analyzer: audio::ANALYZER_STRATUM.to_string(),
    file_size,
    file_mtime,
    analyzer_version: audio::STRATUM_SCHEMA_VERSION.to_string(),
    features_json,
}
```

The local store is allowed to write, but the Rekordbox SQLCipher database must remain
read-only. `sha2` is already a root dependency; no new hashing dependency is needed.

The published claims that must be replaced are:

```text
README: Cache entries are keyed by schema version...
CLI: Cache is invalidated when the analysis schema version changes.
MCP: Cache is keyed by file path and analysis schema version.
```

## Commands you will need

| Purpose               | Command                                                                      | Expected on success    |
| --------------------- | ---------------------------------------------------------------------------- | ---------------------- |
| Store tests           | `cargo test -p reklawdbox store::tests -- --nocapture`                       | exit 0                 |
| Audio tests           | `cargo test -p reklawdbox audio::tests -- --nocapture`                       | exit 0                 |
| CLI cache tests       | `cargo test -p reklawdbox cli::tests -- --nocapture`                         | exit 0                 |
| Tool cache tests      | `cargo test -p reklawdbox tools::tests -- --nocapture`                       | exit 0                 |
| Root suite            | `cargo test -p reklawdbox --no-fail-fast`                                    | exit 0                 |
| Workspace suite       | `cargo test -p stratum-dsp --no-fail-fast`                                   | exit 0                 |
| Format/lint           | `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings` | exit 0                 |
| Repository formatting | `dprint check`                                                               | exit 0                 |
| Docs build            | `(cd site && npm ci && npm run build)`                                       | exit 0; locked build   |
| Release build         | `cargo build --release && ./target/release/reklawdbox --version`             | exit 0; version prints |

## Scope

**In scope** (the only source/docs files you should modify):

- `src/audio.rs`
- `src/store.rs`
- `src/tools/analysis.rs`
- `src/tools/audio_handlers.rs`
- `src/tools/classify_handler.rs`
- `src/tools/resolve_handlers.rs`
- `src/tools/scoring.rs` only if its cache helper call signature changes
- `src/tools/tests.rs`
- `src/cli/mod.rs`
- `src/cli/analyze.rs`
- `src/cli/hydrate.rs`
- `README.md`
- `site/src/content/docs/cli/index.mdx`
- `site/src/content/docs/mcp-tools/enrichment-analysis.mdx`
- `plans/README.md` only for the status-row update

**Out of scope** (do NOT touch):

- The serialized `StratumResult` shape or DSP algorithms completed by Plans 004–006.
- Timbral-statistics identity and audit-freshness semantics completed by Plans 010–011;
  preserve them while extending the shared store schema.
- Essentia cache dependence on Rekordbox grids; it must remain audio/schema-only.
- Direct writes, locks, or migrations against Rekordbox `master.db`.
- Changing SQLCipher open flags or resolving grids from a writable connection.
- Hashing audio contents; audio file identity remains size/mtime under the existing
  cache policy.
- Broad cache redesign, eviction policy, or remote cache.
- Adding a new dependency; use existing `sha2`.
- Documentation outside the three stale cache-identity claims; run doc-drift and STOP
  before expanding scope if another concrete public claim is found.

## Git workflow

- Branch: `codex/007-invalidate-stratum-cache-on-grid-change`
- Use Conventional Commits. Suggested logical commits:
  1. `test(cache): cover Stratum grid input identity`
  2. `fix(cache): invalidate Stratum analysis on grid changes`
  3. `docs(cache): describe analyzer input freshness`
- Do not push or open a PR unless instructed.
- Do **not** reset `STRATUM_SCHEMA_VERSION` or `STORE_SCHEMA_VERSION` to a planned-at
  value. Plans 004–006 will have moved Stratum's version, while Plans 010–011 may have
  moved the store version/schema. This plan changes cache identity/storage rather than
  output semantics, so increment the post-Plan-011 **live** `STORE_SCHEMA_VERSION`
  exactly once for the new fingerprint-column migration. If that exact migration
  already exists or implementation changes valid output, STOP for reconciliation or a
  separate schema decision.

## Steps

### Step 1: Define and test a stable Stratum grid-input fingerprint

In `src/audio.rs`, introduce a small owned value representing the exact input selected
for analysis, for example:

```rust
pub struct RekordboxGridInput {
    pub grid: Option<stratum_dsp::BeatGrid>,
    pub fingerprint: String,
}
```

The type/name may differ, but the grid and fingerprint must travel together. Define
fingerprint domains:

- `grid:v1:<sha256>` when a PQTZ grid is available;
- `hmm:v1` whenever analysis receives `None` and therefore uses the generated HMM grid.

Hash the **semantic grid input**, not filesystem metadata: include a domain prefix,
vector labels/lengths, and every `beats`, `downbeats`, and `bars` `f32::to_bits()` value
in a fixed byte order. This makes a beat/bar edit invalidate even if file size and
timestamp are preserved, while irrelevant ANLZ tags do not. Use `sha2::Sha256`, already
in `Cargo.toml`. Never use `DefaultHasher`, debug formatting, locale-sensitive floats,
or an absolute path in the fingerprint.

Refactor `load_rekordbox_grid_for_path` through a new function returning the paired
input. Retain the old function as a compatibility wrapper if current callers/tests need
it. Every failure that actually supplies `None` to Stratum gets `hmm:v1`; if lookup
succeeds later, it changes to `grid:v1:...` automatically.

Unit tests must prove identical grids hash identically and that changing one beat, one
bar/downbeat, vector length, or grid-vs-HMM source changes the fingerprint. Assert
domain prefixes and equality/inequality, not a hardcoded SHA string.

**Verify**:
`cargo test -p reklawdbox audio::tests -- --nocapture` → exit 0; fingerprint tests pass
without a real Rekordbox library.

### Step 2: Migrate the local cache schema

In `src/store.rs`:

After reconciling the live schema and store helpers from reviewed Plans 010–011,
perform only the migration work not already present:

1. Add `input_fingerprint TEXT NOT NULL DEFAULT ''` to new
   `audio_analysis_cache` tables.
2. Add an idempotent migration using the existing `table_has_column` pattern and
   `ALTER TABLE ... ADD COLUMN ... NOT NULL DEFAULT ''` for existing stores.
3. Increment `STORE_SCHEMA_VERSION` by one from its live value; if Plan 010 already
   reserved/implemented an equivalent version step, extend its migration sequence
   monotonically rather than reusing or decrementing a version.
4. Add `input_fingerprint` to `CachedAudioAnalysis`, `AudioAnalysisIdentity`, every
   SELECT mapping, and `set_audio_analysis` insert/upsert.
5. Require fingerprint equality in `is_audio_analysis_fresh`,
   `batch_get_fresh_audio_analysis`, and
   `batch_fresh_audio_analysis_existence`.

Treat `''` as the identity for analyzers with no auxiliary input, including Essentia.
All Stratum probes must use a non-empty `grid:v1:...` or `hmm:v1`, which intentionally
makes migrated legacy Stratum rows stale. Existing Essentia rows default to `''` and
remain fresh when audio/schema identity matches.

Add migration tests that create the legacy table manually, insert one Stratum and one
Essentia row, run `migrate`, and assert the column exists/defaults to empty. Then assert
the old Stratum row is stale against `hmm:v1`, while the Essentia row is fresh against
`''`. Cover single, batch-map, and batch-existence freshness with changed fingerprints.

**Verify**:
`cargo test -p reklawdbox store::tests -- --nocapture` → exit 0; migration is
idempotent and all fingerprint freshness variants pass.

### Step 3: Carry the analyzed fingerprint through every write path

Change Stratum analysis helpers to return the result plus the exact fingerprint loaded
for that run. Do not recompute the fingerprint after analysis from a second lookup;
the paired grid/fingerprint snapshot passed to `analyze_with_stratum` is authoritative
for the cache write.

Update:

- MCP single and batch analysis in `audio_handlers.rs`;
- `CacheWriteMsg::Audio`;
- CLI `CliCacheWriteMsg` and its writer in `cli/analyze.rs`;
- `HydrateCacheMsg::AudioAnalysis` and its writer/callers in `cli/hydrate.rs`;
- all direct `store::set_audio_analysis` test/setup calls.

For Essentia writes, always store `input_fingerprint = ""`. For Stratum writes, reject
an empty fingerprint as an internal error in tests/debug assertions; production code
must always receive one of the versioned domains.

Add a unit test with an injected/synthetic paired grid input proving the cached
fingerprint is the one analyzed, even if a second synthetic "current" value differs.

**Verify**:

```bash
cargo test -p reklawdbox audio::tests -- --nocapture
cargo test -p reklawdbox cli::tests -- --nocapture
rg -n 'set_audio_analysis\(' src | wc -l
```

Run test filters separately if needed. Expected: tests exit 0; inspect every reported
call and confirm it supplies the new fingerprint argument deliberately (never a blind
empty value for Stratum).

### Step 4: Make single cache reads analyzer-aware

Extend `AudioCacheIdentity` in `src/tools/analysis.rs` and `CacheProbe` in
`src/cli/mod.rs` with the current Stratum input fingerprint. Add an explicit conversion
method taking the analyzer (or two clearly named conversions):

- Stratum store identity uses the current grid/HMM fingerprint.
- Essentia store identity uses `""`.

Update `get_fresh_analysis_entry`, `check_analysis_cache`, CLI
`has_fresh_cache_entry`, and `cache_status_for_track` so analyzer choice cannot
accidentally apply the grid fingerprint to Essentia. Prefer typed/named helper methods
over repeated string conditionals at call sites.

Cache probing may load/parse the current grid before deciding freshness. On a miss,
actual analysis must write the paired fingerprint from Step 3. A grid edit between the
probe and analysis can cause one extra miss but must never label a result with the
wrong fingerprint.

Add tests for same audio/schema with same grid (hit), changed grid (Stratum miss), and
changed grid (Essentia still hit).

**Verify**:

```bash
cargo test -p reklawdbox cli::tests -- --nocapture
cargo test -p reklawdbox tools::tests -- --nocapture
```

Expected: both suites exit 0; analyzer-specific hit/miss tests pass.

### Step 5: Update every batch cache consumer

The same base audio identity is currently reused for Stratum and Essentia batch reads.
Build analyzer-specific identity vectors instead:

- `stratum_identities` with `grid:v1:...`/`hmm:v1`;
- `essentia_identities` with `""`.

Update every `batch_get_fresh_audio_analysis` and
`batch_fresh_audio_analysis_existence` call in `classify_handler.rs` and
`resolve_handlers.rs`, including repeated classification modes. Update any scoring
helper that goes through the single read API.

Use the existing read-only lookup path to obtain each current fingerprint. Do not add
connection pooling or a new public batch DB API in this correctness plan; the existing
per-track connection cost is explicitly deferred until benchmarked. You may deduplicate
repeated canonical paths within the handler before calling the existing lookup, as a
local mechanical optimization that does not change connection ownership. Add a
synthetic batch test for duplicate paths and mixed grid/HMM inputs without requiring a
real encrypted DB.

**Verify**:

```bash
rg -n 'batch_get_fresh_audio_analysis|batch_fresh_audio_analysis_existence' src/tools
cargo test -p reklawdbox tools::tests -- --nocapture
```

Expected: every listed call uses the correctly named analyzer-specific identity vector;
tests exit 0.

### Step 6: Update CLI preflight without changing work selection semantics

Make CLI cache probes carry the Stratum fingerprint and ensure both `analyze` and
`hydrate` use it when deciding `needs_stratum`. Essentia preflight remains fingerprint
`""`. If convenient, batch-resolve fingerprints for the track list before the current
preflight loop; do not mix the local cache connection with the Rekordbox connection.

After a Stratum miss, load one paired grid input for the actual analysis and send its
fingerprint in the cache message. Preserve scheduling, concurrency, skip-cache flags,
and writer ownership.

Add CLI tests proving a changed grid marks only Stratum pending and a subsequent write
with the new fingerprint becomes fresh.

**Verify**:
`cargo test -p reklawdbox cli::tests -- --nocapture` → exit 0; existing audio file
size/mtime invalidation tests and new grid-only invalidation tests all pass.

### Step 7: Decide schema-version handling explicitly

Inspect the live `STRATUM_SCHEMA_VERSION` after Plans 004–006. Do not decrement or
hardcode it. This plan does not change serialized `StratumResult` values for a given
audio+grid input, and migrated legacy rows are invalidated by their empty fingerprint,
so no additional Stratum output-schema bump is required.

Keep the fingerprint domain version (`grid:v1`/`hmm:v1`) explicit. A future change to
fingerprint semantics should change that domain version, automatically invalidating
affected rows. Only increment `STRATUM_SCHEMA_VERSION` if implementation unexpectedly
changes actual output values; if so, STOP and report rather than combining concerns.

**Verify**:
`git diff -- src/audio.rs | sed -n '/STRATUM_SCHEMA_VERSION/,+3p'` → no change to the
live constant/test from this plan; fingerprint-domain additions may appear elsewhere
in the file.

### Step 8: Update every published cache-identity claim

Update the three in-scope documentation surfaces with one consistent contract:

- cache freshness always includes analyzer schema plus canonical audio identity
  (current size/mtime policy);
- Stratum freshness additionally includes the current Rekordbox beat-grid fingerprint,
  or the versioned HMM/no-grid fingerprint;
- Essentia remains independent of Rekordbox grid edits;
- a schema change, audio identity change, or analyzer-specific auxiliary-input change
  invalidates only the affected row.

Do not expose internal SQL column names or imply that Rekordbox `master.db` is writable.
Run the doc-drift workflow in `docs/workflows/doc-drift/README.md`; if it finds another
concrete stale cache-identity claim, STOP and report it before changing an out-of-scope
file.

**Verify**:

```bash
rg -n 'schema|fingerprint|beat.grid|Essentia' \
  README.md \
  site/src/content/docs/cli/index.mdx \
  site/src/content/docs/mcp-tools/enrichment-analysis.mdx
(cd site && npm ci && npm run build)
```

Expected: all three surfaces state the same analyzer-specific contract; the locked
Starlight install/build exits 0.

### Step 9: Run all gates and smoke the release binary

**Verify**:

```bash
cargo fmt --check
dprint check
(cd site && npm ci && npm run build)
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo test -p stratum-dsp --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
git diff --name-only
```

Expected: all commands exit 0; version/help print normally; only in-scope files changed.

## Test plan

- Pure fingerprint tests: deterministic same-grid hash; beat/bar/source mutations.
- Store tests: fresh/stale single reads, batch maps, batch existence, legacy migration,
  idempotent migration, Essentia independence.
- Analysis-write tests: paired grid and fingerprint cannot diverge.
- MCP/CLI tests: grid-only change misses Stratum, leaves Essentia fresh, writes new
  identity, then hits.
- Batch tests: mixed grid/HMM, duplicate canonical paths, analyzer-specific vectors.
- Existing audio size/mtime and schema-version invalidation tests remain green.
- Plan 010 timbral-statistics and Plan 011 audit-freshness regressions remain green.
- Docs: README, CLI, and MCP references agree on schema + audio identity +
  analyzer-specific auxiliary input; doc-drift and locked site build pass.
- No test requires the user's real Rekordbox database or private audio.

## Done criteria

All must hold:

- [ ] Semantic changes to any beat/bar/downbeat alter the `grid:v1` fingerprint.
- [ ] HMM fallback has a stable non-empty versioned fingerprint distinct from any grid.
- [ ] `audio_analysis_cache.input_fingerprint` exists on fresh and migrated stores.
- [ ] `STORE_SCHEMA_VERSION` is incremented once and migration is idempotent.
- [ ] Existing Stratum rows with empty fingerprints are stale; existing Essentia rows
      remain eligible when audio/schema identity matches.
- [ ] Single, batch-map, and batch-existence freshness require the appropriate
      analyzer-specific fingerprint.
- [ ] Every Stratum writer stores the fingerprint paired with the grid actually analyzed.
- [ ] A grid-only edit produces a Stratum miss and an Essentia hit in MCP and CLI tests.
- [ ] Batch consumers do not reuse one fingerprint identity vector for both analyzers.
- [ ] Rekordbox DB access remains read-only; no write SQL/path is introduced.
- [ ] `STRATUM_SCHEMA_VERSION` is not reset or bumped solely for this identity migration.
- [ ] Plan 010 timbral-statistics and Plan 011 audit-freshness behavior/tests are
      preserved.
- [ ] README, CLI, and MCP docs describe grid-aware Stratum freshness and
      grid-independent Essentia freshness consistently.
- [ ] The doc-drift workflow is reviewed and `(cd site && npm ci && npm run build)`
      exits 0.
- [ ] Format, dprint, workspace clippy, both crate suites, and release build/smokes exit 0.
- [ ] `git diff --check` exits 0 and no out-of-scope files changed.
- [ ] `plans/README.md` status row is updated if the executor owns the index.

## STOP conditions

Stop and report back without improvising if:

- Plans 004–006, 009, 011 (including 010), or 013 are incomplete, or their required
  tests are red.
- Dependency work replaced the cache schema/identity model so the excerpts no longer
  describe live behavior.
- The live store already contains an equivalent `input_fingerprint` column/migration;
  do not silently skip or reuse a migration number—STOP and reconcile ownership.
- Computing the current grid fingerprint would require writing to or locking
  Rekordbox `master.db`.
- The only proposed fingerprint is file mtime/size, debug-formatted floats, an unstable
  hasher, or a value recomputed separately from the analyzed grid snapshot.
- Keeping Essentia fresh would require storing a Rekordbox fingerprint on Essentia rows.
- Doc-drift finds another stale cache-identity claim outside the three in-scope
  documentation files; report it before broadening scope.
- Correctness appears to require connection pooling or a new public Rekordbox batch DB
  API; keep the existing read-only lookup behavior and report the performance follow-up.
- The change alters serialized Stratum values and therefore requires an unplanned
  `STRATUM_SCHEMA_VERSION` bump.
- A step fails twice or requires an out-of-scope file.

## Maintenance notes

- Cache identity is the full set of inputs that can change output: analyzer schema,
  audio identity, and analyzer-specific auxiliary input. Add future external inputs to
  this model at the time they are introduced.
- Bump the fingerprint domain when hashing semantics change. Bump
  `STRATUM_SCHEMA_VERSION` when output semantics/shape changes; these are related but
  distinct invalidation mechanisms.
- Keep grid and fingerprint paired through analysis and write to avoid TOCTOU
  mislabelling.
- Preserve analyzer-specific identities in new batch consumers. Essentia must not be
  invalidated by Rekordbox-only edits.
- The local SQLite store is writable by design; Rekordbox `master.db` remains strictly
  read-only.
