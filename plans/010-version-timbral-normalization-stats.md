# Plan 010: Version timbral normalization statistics by their exact source set

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` unless the orchestrator/reviewer owns the index.
>
> **Drift check (run first)**:
>
> ```bash
> git diff --stat e6eb382..HEAD -- src/store.rs src/tools/scoring.rs src/tools/eval_scoring.rs
> ```
>
> If any file changed, compare the schema and freshness excerpts below with
> live code. Any independently added normalization provenance or store-schema
> migration is a STOP condition requiring reconciliation by the orchestrator.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: bug
- **Planned at**: commit `e6eb382`, 2026-07-10

## Why this matters

Pool timbral scores are z-normalized against statistics cached for the whole
Essentia corpus. Those statistics are reused solely because the usable row
count changed by at most ten percent, so same-count upserts, removals plus
insertions, feature changes, and vector-layout changes can retain an obsolete
mean and standard deviation indefinitely. The persisted statistics must name
the exact fresh source set, analyzer schema, and vector schema from which they
were computed; stale file identities must not influence the population.

## Current state

- `src/store.rs` owns the writable internal SQLite store. These writes are
  allowed; Rekordbox `master.db` remains read-only.
- `src/tools/scoring.rs` assembles Essentia timbral vectors, computes Welford
  statistics, lazily ensures them, and consumes them in pool scoring.
- `src/tools/eval_scoring.rs` constructs `TimbralNormStats` fixtures directly;
  every literal must gain explicit coherent test provenance when the struct grows.
- `sha2 = "0.10"` is already a root dependency (`Cargo.toml:32`); no new hash
  dependency is needed.
- Fresh audio cache identity elsewhere is schema version + file size +
  second-resolution file mtime; match `store::is_audio_analysis_fresh` and the
  metadata pattern in `src/tools/analysis.rs:34-41` rather than inventing an
  incompatible definition.

Current `src/store.rs:105-110` stores only per-dimension values and a count:

```sql
CREATE TABLE IF NOT EXISTS timbral_norm_stats (
    dimension_index INTEGER PRIMARY KEY,
    mean REAL NOT NULL,
    stddev REAL NOT NULL,
    sample_count INTEGER NOT NULL,
    computed_at TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Current `src/store.rs:683-709` exposes no provenance:

```rust
pub struct TimbralNormStats {
    pub dims: Vec<(f64, f64)>, // (mean, stddev) per dimension
    pub sample_count: i64,
}

let sample_count = rows[0].3;
let dims: Vec<(f64, f64)> = rows.iter().map(|r| (r.1, r.2)).collect();
Ok(Some(TimbralNormStats { dims, sample_count }))
```

Current `src/tools/scoring.rs:1550-1584` computes over every row with the
current Essentia version but neither validates its file identity nor records
which rows were consumed:

```rust
let mut stmt = store_conn.prepare(
    "SELECT features_json FROM audio_analysis_cache \
     WHERE analyzer = ?1 AND analysis_version = ?2",
)?;

for row in rows {
    let essentia: crate::audio::EssentiaOutput = match serde_json::from_str(&json_str) {
        Ok(e) => e,
        Err(_) => continue,
    };
    let Some(vec) = build_timbral_vector_from_essentia(&essentia) else {
        continue;
    };
    // Welford update...
}
```

Current `src/tools/scoring.rs:1635-1674` uses count drift as the entire
invalidation policy:

```rust
/// Recomputes if missing or cache has grown >10% since last computation.
pub(super) fn ensure_timbral_norm_stats(...) -> Result<Option<...>, String> {
    let current_count: i64 = store_conn.query_row("SELECT COUNT(*) ...", ...)?;
    // ...
    if let Some(ref stats) = existing {
        let drift = (current_count - stats.sample_count).abs() as f64
            / stats.sample_count as f64;
        if drift <= 0.10 {
            return Ok(existing);
        }
    }
```

Applicable conventions:

- Store migrations are idempotent and column-detected with
  `table_has_column`; see `migrate_enrichment_cache` in
  `src/store.rs:152-201`.
- Store writes that replace a coherent multi-row value use one transaction;
  preserve the delete/insert transaction in `save_timbral_norm_stats`.
- `EssentiaOutput` is `#[serde(default)]`, allowing compact synthetic JSON
  fixtures.
- Do not silently disable the timbral axis on a vector-dimension mismatch. An
  old vector schema must be rejected and recomputed before scoring.

## Commands you will need

| Purpose            | Command                                                          | Expected on success         |
| ------------------ | ---------------------------------------------------------------- | --------------------------- |
| Store tests        | `cargo test -p reklawdbox store::tests::test_timbral_norm_stats` | exit 0; matching tests pass |
| Scoring tests      | `cargo test -p reklawdbox timbral_norm`                          | exit 0; matching tests pass |
| Pool tests         | `cargo test -p reklawdbox pool`                                  | exit 0; matching tests pass |
| Eval scoring tests | `cargo test -p reklawdbox eval_scoring`                          | exit 0; fixture tests pass  |
| Format             | `cargo fmt --check`                                              | exit 0, no diff             |
| Docs/config format | `dprint check`                                                   | exit 0                      |
| Lint               | `cargo clippy -p reklawdbox --all-targets -- -D warnings`        | exit 0, no warnings         |
| Full crate tests   | `cargo test -p reklawdbox --no-fail-fast`                        | exit 0; all tests pass      |

## Scope

**In scope** (the only source files you may modify):

- `src/store.rs`
- `src/tools/scoring.rs`
- `src/tools/eval_scoring.rs`
- `plans/README.md` for the status row only

**Out of scope**:

- Changing the mathematical composition or weights of the timbral axis.
- Changing `EssentiaOutput`, its analyzer script, or
  `ESSENTIA_SCHEMA_VERSION` merely to force invalidation.
- Adding a new hashing crate; use the existing `sha2` dependency.
- Changing generic audio-cache freshness semantics; follow the existing
  `is_audio_analysis_fresh` contract in this plan.
- Direct Rekordbox database writes or user-visible metadata staging.
- Purging valid audio-analysis cache rows.

## Git workflow

- Branch: `codex/010-version-timbral-normalization-stats`
- Use Conventional Commits; preferred final message:
  `fix(scoring): version timbral normalization stats`.
- Do not push or open a PR unless explicitly instructed.

## Steps

### Step 1: Add provenance and migration tests to the internal store

Extend `TimbralNormStats` with all of:

- `source_fingerprint: String` — lowercase SHA-256 hex of the usable, fresh,
  deterministically ordered source rows;
- `analysis_version: String` — the Essentia schema used;
- `vector_schema_version: String` — an explicit local version for
  `assemble_timbral_vector` layout/ordering;
- existing `dims` and `sample_count`.

Update the fresh-install `timbral_norm_stats` DDL with those provenance
columns. Add an idempotent `migrate_timbral_norm_stats` using
`table_has_column`, and call it from `migrate`. At the planned base SHA,
increment `STORE_SCHEMA_VERSION` from 7 to 8. Migration defaults must be empty
strings, which are deliberately invalid and force one recomputation; do not
claim legacy stats came from the new source definition.

Update `get_timbral_norm_stats` and `save_timbral_norm_stats` so every dimension
row round-trips identical provenance. When persisted rows disagree on sample
count/provenance or dimension indices are not contiguous from zero, return
`Ok(None)` (or an explicit `Stale` outcome that `ensure_timbral_norm_stats`
handles identically) so the caller recomputes. Reserve `Err` for actual SQLite
read failures; do not construct a partially coherent stats object and do not
let malformed cached content abort the required rebuild.

Add tests in `src/store.rs` for:

- provenance round-trip and atomic replacement;
- a handcrafted legacy table receiving the new columns on `open`;
- idempotent second open;
- inconsistent provenance rows being rejected.
- incoherent persisted rows yielding a stale/missing result and then being
  recomputed by `ensure_timbral_norm_stats`.

**Verify**:
`cargo test -p reklawdbox store::tests::test_timbral_norm_stats` → all matching
round-trip and migration tests pass.

### Step 2: Define and load a deterministic fresh source snapshot

In `src/tools/scoring.rs`, add a private constant such as
`TIMBRAL_VECTOR_SCHEMA_VERSION: &str = "1"` adjacent to
`assemble_timbral_vector`. Its maintenance contract is: increment whenever
component order, inclusion, or dimension-selection semantics change.

Add a private `TimbralSourceSnapshot` containing `vectors` and
`source_fingerprint`. Load it with one ordered SQLite query selecting at least
`file_path`, `file_size`, `file_mtime`, and `features_json` for the current
Essentia analyzer/schema, ordered by `file_path`.

For every row:

1. `std::fs::metadata(file_path)` must succeed.
2. Actual size/mtime must match the cached values under the same rules as
   `store::is_audio_analysis_fresh`; otherwise exclude the row.
3. JSON must deserialize and all five timbral components required by
   `build_timbral_vector_from_essentia` must be present.
4. Every vector element must be finite and every included vector must have the
   same length; invalid rows are excluded rather than poisoning Welford.
5. Feed SHA-256 a length-delimited encoding of the vector schema version,
   Essentia analysis version, file path, cached identity, and exact
   `features_json`. Do not concatenate fields ambiguously and do not use
   `DefaultHasher`, whose stability is not a persistence contract.

Only rows that enter `vectors` enter the fingerprint and `sample_count`.
Sorted query order makes the digest independent of SQLite row order.

**Verify**: `cargo test -p reklawdbox timbral_source_snapshot` → new tests show
that row order does not change the digest, while changing JSON, identity, or
vector schema does.

### Step 3: Compute and persist stats from exactly one snapshot

Refactor `compute_timbral_norm_stats` so it accepts or internally produces one
`TimbralSourceSnapshot` and runs Welford over those exact vectors. Do not query
the source a second time between fingerprinting and computation. Populate all
provenance fields on the returned `TimbralNormStats`.

Retain the minimum of two usable samples and the `1e-10` standard-deviation
floor. If fewer than two fresh usable rows exist, return `None` through the
ensure path and delete any persisted normalization rows so callers cannot
accidentally reload a stale population.

**Verify**: `cargo test -p reklawdbox timbral_norm_compute` → exact synthetic
means/deviations, sample count, and provenance pass; a stale third row is
excluded.

### Step 4: Replace count drift with exact provenance matching

Rewrite `ensure_timbral_norm_stats` to:

1. load one current source snapshot;
2. return `None` and clear old stats when fewer than two usable rows exist;
3. reuse persisted stats only when `source_fingerprint`,
   `analysis_version`, and `vector_schema_version` all exactly match the
   snapshot/current constants and dimensions are coherent;
4. otherwise recompute from the already-loaded snapshot, save atomically, and
   return it.

Remove the ten-percent count-drift policy and its comment entirely. A source
set replacement with the same count must recompute. A dimension mismatch must
not fall through to `normalize_timbral_vector -> None` while stale stats remain
marked current.

Add regression tests that create real temporary files and cache rows with
matching identities:

- same-count JSON upsert changes fingerprint and recomputes means;
- remove one row/add another at the same count recomputes;
- mutate a file so its cached identity is stale; it is excluded and stats are
  recomputed/cleared as appropriate;
- unchanged source reuses persisted stats (the fingerprint and `computed_at`
  or a test-visible recomputation counter remains unchanged);
- analyzer or vector schema mismatch recomputes.

Use compact `EssentiaOutput` JSON containing `mfcc_mean`, `mfcc_std`,
`spectral_contrast_mean`, `spectral_centroid_cv`, and `dissonance_mean`; no
private audio is needed.

**Verify**: `cargo test -p reklawdbox timbral_norm` → every source-change and
reuse regression passes.

### Step 5: Run pool consumers and the full crate gate

Run pool/scoring tests to confirm the richer struct does not change score
formulas. Inspect every `TimbralNormStats` test literal and populate provenance
explicitly; do not hide missing fields behind a `Default` implementation that
could create apparently valid empty provenance. In `src/tools/eval_scoring.rs`,
update `dummy_norm_stats` and direct literals with consistent non-empty test
fingerprint, analysis-version, and vector-schema values.

**Verify**:

```bash
cargo test -p reklawdbox pool
cargo test -p reklawdbox eval_scoring
cargo fmt --check
dprint check
cargo clippy -p reklawdbox --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
git diff --check
```

Expected: all commands exit 0; existing pool score assertions remain unchanged;
only in-scope files are modified.

## Test plan

- `src/store.rs`:
  - new-schema/provenance round trip;
  - legacy migration and idempotence;
  - coherent replacement;
  - inconsistent dimension provenance rejected.
- `src/tools/scoring.rs`:
  - deterministic digest independent of query insertion order;
  - digest changes for same-count JSON replacement;
  - current source excludes missing/stale/invalid/non-finite rows;
  - Welford expected values for a small fixture;
  - exact provenance reuse and each mismatch-triggered recomputation;
  - fewer than two fresh samples clears/returns no stats.
- `src/tools/eval_scoring.rs`: every direct fixture has explicit non-empty coherent
  provenance and existing evaluation-score assertions are unchanged.
- Use temporary synthetic files plus `store::set_audio_analysis`; do not require
  Rekordbox, Essentia Python, network access, or private audio.

## Done criteria

- [ ] Persisted stats contain source fingerprint, Essentia schema, and vector
      schema version.
- [ ] `rg "drift <= 0\.10|grown >10%" src/tools/scoring.rs` returns no matches.
- [ ] Only fresh, finite, structurally complete current-schema rows contribute.
- [ ] Same-count feature replacement changes provenance and recomputes stats.
- [ ] Unchanged exact provenance reuses stats.
- [ ] Legacy stats are invalidated once, not falsely blessed.
- [ ] Incoherent cached rows are treated as stale and recomputed; only real
      database failures propagate as errors.
- [ ] Fewer than two fresh usable rows returns no normalization stats.
- [ ] Existing timbral score formulas and expected pool scores are unchanged.
- [ ] All `eval_scoring` `TimbralNormStats` fixtures compile with explicit provenance,
      and their existing score assertions pass.
- [ ] Store migration tests, targeted tests, format, dprint, clippy, full crate
      tests, release build, `--version`, and `--help` exit 0.
- [ ] `git diff --name-only` lists only `src/store.rs`, `src/tools/scoring.rs`,
      `src/tools/eval_scoring.rs`, and optionally `plans/README.md`.
- [ ] Rekordbox `master.db` remains read-only.

## STOP conditions

Stop and report back if:

- Another branch already changed `STORE_SCHEMA_VERSION` or the
  `timbral_norm_stats` schema; the orchestrator must assign a new migration
  number and reconcile both migrations explicitly.
- Current audio-cache identity has changed from size/mtime/schema; reuse the
  new canonical helper rather than maintaining two definitions.
- Correct fingerprinting would require hashing secret material. Only file
  paths, cache identities, analyzer versions, and feature JSON belong here.
- The source corpus cannot be made deterministic without reading Rekordbox or
  invoking Essentia; it should come solely from the internal cache and local
  file metadata.
- The proposed change would alter pool weights/formulas or silently accept a
  mismatched vector dimension.
- A verification command fails twice after one reasonable correction.

## Maintenance notes

- Increment `TIMBRAL_VECTOR_SCHEMA_VERSION` whenever
  `assemble_timbral_vector` changes order, membership, or dimensional rules.
- An `ESSENTIA_SCHEMA_VERSION` bump must automatically invalidate persisted
  normalization stats through exact provenance comparison.
- Reviewers should check that fingerprint input is length-delimited and that
  computation uses the exact snapshot that was fingerprinted.
- Snapshot loading is O(number of current Essentia rows) and stats computation
  is also O(n) only on a mismatch. If this becomes measurably expensive, add a
  trustworthy store generation index in a separate plan; do not weaken exact
  correctness back to row-count heuristics.
