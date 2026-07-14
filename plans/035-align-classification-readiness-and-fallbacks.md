# Plan 035: Align classification readiness, profile freshness, and label fallbacks

> **Executor instructions**: Follow this plan step by step. Run every
> verification command and confirm the expected result before moving to the
> next step. If anything in the "STOP conditions" section occurs, stop and
> report — do not improvise. When done, update this plan's row in
> `plans/README.md` only if the reviewer has not told you that they own the
> index.
>
> **Required base and drift check (run first)**:
>
> This plan was written against commit `4e031ca` plus the uncommitted
> post-Beatport-removal working-tree snapshot present on 2026-07-14. It depends
> on reviewed Plan 034 and must start from a clean composite containing both the
> Beatport removal and Plan 034.
>
> ```bash
> test -z "$(git status --porcelain)"
> ! rg -n -i "beatport" src --glob '*.rs'
> git log -1 --oneline
> git diff --check
> ```
>
> Expected: the worktree is clean; no active Rust Beatport reference exists;
> the current branch contains Plan 034's source-aware evidence model and
> `review_required` contract. Record the resulting commit as `PLAN_034_BASE`.
> Compare every live path below with the excerpts. STOP on a semantic mismatch
> in cache keys, readiness counts, calibration storage, or label precedence.

## Status

- **Priority**: P1
- **Effort**: L
- **Risk**: MED
- **Depends on**: 034 and the reviewed post-Beatport removal base; extends
  completed Plans 009 and 023
- **Category**: readiness correctness / cache schema / calibration safety /
  metadata fallback
- **Planned at**: commit `4e031ca` plus post-Beatport working-tree snapshot,
  2026-07-14

## Why this matters

After Beatport removal, the application needs honest answers to three different
questions:

1. Was Discogs searched for the exact track key the classifier will load?
2. Did the cached match contain a genre the taxonomy can actually use?
3. Are calibrated audio profiles both scorable and compatible with the current
   classifier/analyzer versions and verified training playlist?

The current code collapses each distinction too early.

`TrackEvidence.has_discogs` is set from `discogs_cache.is_some()`. A cached
no-match therefore suppresses the engine's no-enrichment signal even though it
contains no genre. `cache_coverage` separately calls broad artist/title
existence queries that discard `query_album`, while classification reads the
exact `(provider, normalized artist, normalized title, normalized album)` key.
Coverage can say a track has Discogs data that the classifier will never load.

For calibration, `extract_audio_features` returns `Some(AudioFeatures)` for any
fresh Stratum or Essentia row, including a BPM-only record. Coverage counts that
as ready, but profile scoring explicitly rejects BPM-only prototypes:

```rust
if observed_optional_features < required_optional_features
    || !optional_weight_coverage.is_finite()
    || optional_weight_coverage < MIN_CALIBRATED_WEIGHT_COVERAGE
{
    return None;
}
```

Profile persistence then replaces all stored rows transactionally but records
no classifier schema, analyzer versions, playlist, or training fingerprint.
Any non-empty profile table is loaded, even after inputs or algorithms drift.

Finally, label backfill already consumes cached values in the precedence order
Discogs → MusicBrainz → Bandcamp, and the MusicBrainz adapter returns a label.
Yet `backfill_labels(auto_enrich=true)` hydrates only Bandcamp. The analogous
year workflow already dispatches both providers. On the 2026-07-14 local cache,
358 Beatport-derived label results had no cached Discogs, MusicBrainz, or
Bandcamp label; that aggregate makes the asymmetry worth fixing after removal.

This plan makes readiness reflect usable evidence, makes calibrated profiles
versioned local state, and closes the existing MusicBrainz label-hydration gap.
It deliberately does not add Bandcamp tags as classifier votes.

## Current state

### Coverage and classification use different Discogs identities

`src/application/classification/classify.rs` builds and loads an album-aware
key:

```rust
let discogs_key = (
    "discogs".to_string(),
    norm_artist.clone(),
    norm_title.clone(),
    album.to_string(),
);
let discogs_cache = enrich_map.get(&discogs_key);
```

`src/mcp/analysis/coverage.rs` instead loads artist/title sets:

```rust
let discogs_set = store::batch_enrichment_existence(&store, "discogs", &unique_artists)?;
let discogs_result_set =
    store::batch_enrichment_with_results(&store, "discogs", &unique_artists)?;
```

The helper query selects only `query_artist, query_title`, so album-specific
rows collapse together. It also calls every `exact`/`fuzzy` response a result
without applying `extract_discogs_genres`; an exact response whose styles are
all unmapped is not classifier-ready.

The store already supports exact batch keys through
`batch_get_enrichment(keys: &[(&str, &str, &str, &str)])`. Reuse that boundary
and Plan 034's typed Discogs status instead of adding another SQL interpretation.

### Searched, matched, and usable are currently conflated

`EnrichmentCacheEntry` retains both `match_quality` and `response_json`.
`persist_discogs_result` writes `exact`, `fuzzy`, or a durable no-match. But the
classification model currently has only:

```rust
pub(crate) has_discogs: bool,
pub(crate) has_audio: bool,
```

Plan 034 will add source/match provenance. This plan extends that model with a
shared readiness state, not a second parallel enum. The required Discogs states
are:

- `not_searched` — no exact cache row;
- `no_match` — a completed non-error search with no provider result;
- `matched_unmapped` — exact/fuzzy payload exists but produces no canonical
  taxonomy mapping; and
- `usable_genre` — at least one canonical mapped genre exists.

Invalid JSON or unknown match quality must be represented as unusable/error
detail, not promoted to `no_match` or `usable_genre`. Existing broad
`searched`/`has_result` fields may remain for compatibility, but the classifier
and readiness decisions must use the exact detailed state.

### Calibration counts records that cannot form a useful profile

`src/application/classification/calibrate.rs` currently accepts every
`Some(features)` sample:

```rust
match extract_audio_features(track, stratum_cache, essentia_cache) {
    Some(features) => samples.push((canonical, features)),
    None => skipped_no_audio += 1,
}
```

Coverage repeats the same test and declares readiness from the count:

```rust
if extract_audio_features(track, stratum, essentia).is_some() {
    stats.tracks_with_audio_features += 1;
}
// ...
let prototype_ready = stats.tracks_with_audio_features >= profiles::MIN_TRACKS;
```

But `profiles::score_all` has an explicit test proving a BPM-only prototype has
insufficient optional coverage and emits no affinity. Readiness must share
scoring's finite/coverage semantics or dry-run the candidate prototype rather
than counting cache-record presence.

### Stored profile rows have no compatibility metadata

`src/adapters/state/migrations.rs` is currently schema version 9 and creates
`genre_audio_profiles`, `genre_timbral_centroids`, and `genre_global_stats`
without model metadata. `src/adapters/state/classification.rs::load_from_db`
returns any non-empty, parseable registry:

```rust
let count: i64 = conn.query_row(
    "SELECT COUNT(*) FROM genre_audio_profiles",
    [],
    |row| row.get(0),
)?;
if count == 0 {
    return Ok(None);
}
```

Audio cache compatibility is already explicit:

```rust
pub(crate) const STRATUM_SCHEMA_VERSION: &str = "21";
pub(crate) const ESSENTIA_SCHEMA_VERSION: &str = "2";
```

Profile state needs an analogous classifier-profile schema and training
identity. Beatport removal alone does not invalidate audio profiles because it
does not change their feature extraction; do not force recalibration merely
because the provider was removed.

### Label scan reads MusicBrainz but auto-enrichment does not fetch it

`scan_labels` batch-loads all three caches and preserves this precedence:

```rust
let enrichment_label = discogs_label.or(mb_label).or(bc_label);
```

Its gap queue records only `uncached_bandcamp`. The handler dispatches only
`lookup_bandcamp_remote`. In contrast, `src/mcp/metadata/years.rs` already uses
one bounded writer and concurrent Bandcamp/MusicBrainz dispatch paths. Reuse
that proven pattern without changing label precedence.

## Commands you will need

| Purpose                  | Command                                                                                     | Expected on success                                                 |
| ------------------------ | ------------------------------------------------------------------------------------------- | ------------------------------------------------------------------- |
| Evidence/readiness tests | `cargo test -p reklawdbox classification::evidence -- --nocapture`                          | exit 0; exact-key status cases pass                                 |
| Coverage tests           | `cargo test -p reklawdbox cache_coverage -- --nocapture`                                    | exit 0; searched/matched/mapped gaps agree with classifier reads    |
| Calibration tests        | `cargo test -p reklawdbox calibration -- --nocapture`                                       | exit 0; raw vs scorable counts and safe replacement pass            |
| Profile tests            | `cargo test -p reklawdbox classification::profiles -- --nocapture`                          | exit 0; BPM-only and compatibility cases pass                       |
| State migration tests    | `cargo test -p reklawdbox adapters::state -- --nocapture`                                   | exit 0; v9 migration and metadata round-trip pass                   |
| Label tests              | `cargo test -p reklawdbox backfill_labels -- --nocapture`                                   | exit 0; dual-provider hydration and precedence pass                 |
| Rust format              | `cargo fmt --check`                                                                         | exit 0                                                              |
| Docs/config format       | `dprint check`                                                                              | exit 0                                                              |
| Lint                     | `cargo clippy -p reklawdbox --all-targets -- -D warnings`                                   | exit 0                                                              |
| Crate tests              | `cargo test -p reklawdbox --no-fail-fast`                                                   | exit 0                                                              |
| Workspace DSP tests      | `cargo test -p stratum-dsp --no-fail-fast`                                                  | exit 0                                                              |
| Release build            | `cargo build --release`                                                                     | exit 0                                                              |
| MCP smoke                | `node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000` | exit 0; no protocol violations                                      |
| Documentation contract   | commands in `docs/workflows/doc-drift/README.md`                                            | tests and site build exit 0; output schemas and embedded SOPs agree |

## Scope

**In scope — classification evidence and coverage**:

- `src/domain/classification/model.rs`
- `src/application/classification/evidence.rs`
- `src/application/classification/classify.rs`
- `src/mcp/analysis/coverage.rs`
- `src/adapters/state/enrichment.rs` only to remove or narrow broad helpers no
  longer used by coverage, or to expose an exact-key batch primitive without
  duplicating parsing semantics

**In scope — calibration and local profile state**:

- `src/application/classification/calibrate.rs`
- `src/domain/classification/profiles.rs`
- `src/adapters/state/migrations.rs`
- `src/adapters/state/classification.rs`
- `src/adapters/state/tests.rs`

**In scope — label fallback**:

- `src/application/metadata/backfill.rs`
- `src/mcp/metadata/labels.rs`
- `src/mcp/tests/metadata.rs`
- `src/mcp/metadata/years.rs` only if a small private dual-provider hydration
  helper can be extracted without changing year behavior

**In scope — runtime tests and descriptions**:

- `src/mcp/tests/analysis.rs`
- `src/mcp/tests/classification.rs`
- `src/mcp/server.rs`

**In scope — public documentation**:

- `site/src/content/docs/mcp-tools/library-data.mdx`
- `site/src/content/docs/mcp-tools/classification-staging.mdx`
- `site/src/content/docs/workflows/genre-classification.mdx`
- `site/src/content/docs/workflows/genre-audit.mdx`
- `site/src/content/docs/workflows/metadata-backfill.mdx`
- `site/src/partials/sops/genre-classification.mdx`
- `site/src/partials/sops/genre-audit.mdx`
- `site/src/partials/sops/metadata-backfill.mdx`
- `site/src/data/tool-reference.mjs` only when required by the existing
  documentation contract

**Reviewer-owned plan artifact**:

- `plans/README.md`

**Out of scope**:

- Reintroducing Beatport or deleting legacy Beatport cache rows.
- Adding Bandcamp or MusicBrainz as genre-classifier evidence. The MusicBrainz
  adapter has label/date fields, not a genre field; Bandcamp-tag weighting must
  wait for Plan 034's benchmark evidence.
- Taxonomy expansion, label-map changes, BPM-range changes, classifier weight
  tuning, or confidence-policy changes beyond consuming Plan 034's provenance.
- Audio analyzer output/schema changes. Profile metadata references current
  analyzer versions; it does not bump them.
- Album/year backfill precedence or auto-enrichment behavior.
- A generic provider orchestration rewrite. Extract a narrow shared helper only
  if it reduces duplicated label/year dispatch without broadening Scope.
- Destructive cache migration, automatic profile recalibration, release
  automation, direct `master.db` writes, pushing, or opening a PR.

## Git workflow

- Branch: `codex/035-align-classification-readiness-and-fallbacks`
- Preferred commits:
  - `fix(classify): align readiness with usable evidence`
  - `fix(classify): version calibrated profile state`
  - `fix(metadata): hydrate MusicBrainz labels`
  - `docs(classify): document evidence and profile readiness`
- Fewer cohesive Conventional Commits are acceptable; do not mix unrelated
  cleanup.
- Do not push, open a pull request, deploy locally, release, or purge caches
  without separate operator authorization.
- The reviewer owns `plans/README.md`; do not edit the tracker in an executor
  worktree.

## Steps

### Step 1: Characterize the four evidence states and exact-key mismatch

Add red unit/integration tests before changing implementation. Build temporary
store rows for two tracks sharing normalized artist/title but with different
albums, then cover:

1. no exact row → `not_searched`;
2. exact row with durable `none` → `no_match`;
3. exact/fuzzy payload whose styles are absent, invalid, or all unmapped →
   `matched_unmapped`;
4. exact/fuzzy payload with at least one canonical mapping → `usable_genre`;
5. a row for the other album must not change the first track's state; and
6. malformed JSON/unknown match quality is surfaced as unusable diagnostic
   state, never as a usable match.

Exercise both `build_track_evidence` and `cache_coverage` with the same rows.
The tests must initially demonstrate the current mismatch: broad coverage sees
artist/title data while the classifier's album-aware key does not.

Preserve compatibility meanings from Plan 023: `searched` includes completed
no-match and exact/fuzzy rows but excludes retryable errors; `has_result`
includes exact/fuzzy payloads. Add mapped-usability fields rather than silently
renaming these existing fields.

**Verify**:

```bash
cargo test -p reklawdbox classification::evidence -- --nocapture
cargo test -p reklawdbox cache_coverage -- --nocapture
```

Expected before Step 2: new assertions fail only on exact-album identity or
searched/matched/mapped conflation. Setup, migration, or unrelated coverage
failures are STOP conditions.

### Step 2: Use one typed Discogs readiness interpretation everywhere

Extend Plan 034's Discogs provenance type rather than introducing a parallel
boolean. Implement one pure interpreter over `Option<&EnrichmentCacheEntry>`
plus the existing taxonomy parser that yields the detailed status, mapped
genres, match quality, and parse diagnostics in one pass.

Use it in `build_track_evidence`. Replace internal decisions based on
`has_discogs` with the detailed state:

- the engine's missing-evidence flag is cleared only for `usable_genre` when
  discussing genre evidence;
- `no_match` is a completed search but still no usable enrichment;
- `matched_unmapped` is explicitly reviewable/mappable work; and
- invalid/error-like data cannot vote.

In `cache_coverage`, build the exact album-aware Discogs keys exactly as
`classify_batch` does, call `batch_get_enrichment`, and pass every row through
the same interpreter. Preserve existing `coverage.discogs.searched` and
`has_result`, then add:

- `usable_genre` and `usable_genre_percent`;
- `matched_unmapped`; and
- gap counts `not_searched`, `searched_no_match`, and `matched_unmapped`.

Retain `gaps.no_enrichment` only as a compatibility aggregate and define it as
"no usable mapped genre," or add a new unambiguous field and preserve the old
definition if changing it would break callers. Whichever choice is made must be
documented and locked by schema tests. Do not make broad artist/title store
helpers the classifier source of truth.

Add tests proving classification evidence and coverage agree for every exact
key and state. If `batch_enrichment_existence`,
`batch_enrichment_with_results`, or `batch_enrichment_with_label` remains for
other callers, update comments to say that it is album-collapsing and must not
be used for classification readiness.

**Verify**:

```bash
cargo test -p reklawdbox classification::evidence -- --nocapture
cargo test -p reklawdbox cache_coverage -- --nocapture
cargo test -p reklawdbox classification -- --nocapture
```

Expected: all exact-key/status tests pass; coverage never claims usable genre
evidence that `classify_batch` cannot load and map.

### Step 3: Make calibration readiness mean "candidate can score"

Create one reusable calibration-sample inspection path using the same finite
feature extraction and optional-coverage rules as `profiles::score_all`.
Readiness must distinguish:

- a fresh audio cache row exists;
- `extract_audio_features` produced a record;
- the record has scorable optional features; and
- the genre has enough scorable samples to build a candidate prototype that can
  score at least one representative sample.

Do not infer the last two from `Option<AudioFeatures>` alone. Prefer a pure
`profiles` helper that describes feature coverage plus a candidate-registry
dry run, so calibration and coverage cannot drift from the scorer.

In `calibration_coverage`, preserve raw fields such as
`tracks_with_audio_features` for compatibility and add explicit
`tracks_with_scorable_features`, `missing_scorable_features`, and per-genre
candidate readiness. `prototype_ready` and `ready_to_calibrate` must use
scorable samples/candidate validation, not raw cache presence.

In `calibrate_audio_profiles`:

1. report raw and scorable sample counts separately;
2. omit unusable samples/prototypes with explicit reasons;
3. build the candidate registry in memory;
4. dry-run scoring against representative input from each candidate genre;
5. return a structured error/summary if no candidate prototype is usable; and
6. never replace a non-empty compatible stored registry with zero usable
   prototypes.

A BPM-only cache row must remain valid cached analysis, but it is not sufficient
calibration evidence. Missing optional data must remain unknown per Plan 009;
do not create numeric sentinels.

**Verify**:

```bash
cargo test -p reklawdbox calibration -- --nocapture
cargo test -p reklawdbox classification::profiles -- --nocapture
```

Expected: BPM-only rows count as cached/raw but not scorable; coverage and
calibration agree; an unusable candidate cannot erase a good stored registry.

### Step 4: Version profile state and validate it on load

Bump `STORE_SCHEMA_VERSION` and add a single-row profile metadata table through
the existing additive migration pattern. Do not rebuild or delete existing
profile tables. The metadata row must contain at least:

- `classifier_profile_schema_version` — a new constant owned by the
  classification/profile module;
- `stratum_schema_version` and `essentia_schema_version` copied from the audio
  adapter constants;
- resolved verified-playlist name;
- stable training fingerprint and canonical/scorable sample count;
- calibration timestamp; and
- optionally the application version for diagnostics, but never use app
  version alone as compatibility.

Define the stable training fingerprint over sorted, privacy-safe inputs needed
to detect a changed corpus (for example track IDs plus canonical genre and the
fresh audio cache identity/version). Do not hash absolute file paths, cache
payload JSON, timestamps, or iteration order. Document the exact fingerprint
contract in code and tests.

Change the state API from `Option<ProfileRegistry>` to a typed load result that
can represent:

- `Missing` — no profile rows;
- `Fresh` — schema/analyzer metadata compatible and training fingerprint
  matches when a current playlist context is available;
- `TrainingChanged` — compatible model/analyzers but the verified corpus
  fingerprint differs; and
- `Incompatible` — missing legacy metadata or classifier/analyzer schema
  mismatch.

Classification must use only `Fresh` or, by an explicit conservative product
decision, `TrainingChanged` registries. The recommended decision is to allow
`TrainingChanged` for inference but flag/report recalibration due, because the
features remain compatible; suppress `Incompatible` registries entirely.
`calibration_coverage` must expose the status and reasons. Legacy v9 profile
rows migrate as `Incompatible`/metadata-missing rather than being falsely
stamped current.

Save registry rows and metadata in the same transaction. If any insert fails,
preserve the previous complete registry and metadata. Add migration tests from
v9, fresh round-trip tests, each mismatch case, deterministic fingerprint tests,
and rollback tests.

Do not bump `STRATUM_SCHEMA_VERSION` or `ESSENTIA_SCHEMA_VERSION`: this plan
does not change analyzer output. Do not invalidate profiles solely because
Beatport was removed.

**Verify**:

```bash
cargo test -p reklawdbox adapters::state -- --nocapture
cargo test -p reklawdbox calibration -- --nocapture
cargo test -p reklawdbox classification -- --nocapture
```

Expected: legacy state is preserved but not silently consumed; fresh metadata
round-trips; incompatible profiles do not vote; failed saves are atomic.

### Step 5: Hydrate both MusicBrainz and Bandcamp for label gaps

Extend `BackfillLabelsScanResult` with an `uncached_musicbrainz` queue using the
same normalized/raw tuple shape as `uncached_bandcamp`. Queue a provider only
when its exact key is absent and the track still lacks a usable label after the
higher-precedence cached sources have been considered. Preserve label
precedence exactly:

```text
Discogs > MusicBrainz > Bandcamp
```

Update `backfill_labels(auto_enrich=true)` to dispatch bounded MusicBrainz and
Bandcamp lookup work, following the existing year handler's pattern:

- provider-specific concurrency stays bounded;
- MusicBrainz's adapter rate limiter remains authoritative;
- one bounded channel feeds one blocking SQLite writer;
- exact/fuzzy/no-match semantics are persisted for both providers;
- every task releases permits and every writer is joined on all normal paths;
- provider failures remain retryable and do not become durable no-match rows;
  and
- after both providers finish, re-scan once and stage through `ChangeManager`.

Preserve the existing additive `auto_enriched` total for compatibility. Add a
provider breakdown such as `auto_enriched_by_provider.musicbrainz` and
`.bandcamp`; make clear whether the counts represent matches or attempted
lookups and lock that definition in tests/docs. Do not change the label research
gate or conflict precedence.

Add deterministic handler tests with mock/local provider responses for:

- MusicBrainz fills a label when Discogs is absent;
- Discogs still wins when both are cached;
- MusicBrainz wins over Bandcamp;
- Bandcamp remains fallback when MusicBrainz has no label/no match;
- per-provider and total counts reconcile;
- errors remain retryable; and
- `dry_run` controls staging, not cache hydration, consistently with the current
  tool contract.

Do not call the public network in normal tests.

**Verify**:

```bash
cargo test -p reklawdbox backfill_labels -- --nocapture
cargo test -p reklawdbox metadata -- --nocapture
```

Expected: both providers hydrate missing label evidence, precedence is
unchanged, writes use the internal store and staging still uses ChangeManager.

### Step 6: Update public readiness and fallback contracts

Update live tool descriptions, MCP schema docs, human workflows, and embedded
SOPs to distinguish:

- Discogs searched, matched, mapped, and usable states;
- exact album-aware classification readiness;
- raw cached audio versus scorable calibration samples;
- profile status `missing`, `fresh`, `training_changed`, or `incompatible` and
  the required operator action;
- MusicBrainz plus Bandcamp behavior for
  `backfill_labels(auto_enrich=true)`; and
- the unchanged Discogs → MusicBrainz → Bandcamp label precedence.

Preserve Plan 023's rule that workflow readiness is capability-specific and
that a completed no-match is not a retryable error. Preserve Plan 034's rule
that incomplete evidence is reviewable rather than silently confident. Do not
claim every provider must match every track.

Because SOP partials are embedded with `include_str!`, rebuild the release
binary before documentation contract checks.

**Verify**:

```bash
cargo build --release
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
```

Expected: all commands exit 0; docs match live output schemas and embedded SOP
text.

### Step 7: Run the integrated gate and inspect scope

Run:

```bash
cargo fmt --check
dprint check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p reklawdbox --no-fail-fast
cargo test -p stratum-dsp --no-fail-fast
cargo build --release
./target/release/reklawdbox --version
./target/release/reklawdbox --help
node scripts/mcp-smoke.mjs --bin ./target/release/reklawdbox --skip-db --timeout-ms 60000
node --test scripts/check-doc-contract.test.mjs
(cd site && npm ci && npm run build)
node scripts/check-doc-contract.mjs --bin ./target/release/reklawdbox --dist ./site/dist
git diff --check
git status --short
```

Expected: every command exits 0; state migration and rollback tests pass; MCP
smoke reports no protocol violations; docs match runtime; only files in Scope
are modified against `PLAN_034_BASE`.

## Test plan

- Temporary-store tests cover every Discogs readiness state with exact
  album-aware keys and prove coverage/classifier agreement.
- Existing broad searched/has-result fields keep compatibility tests; new
  mapped-usability and gap fields have exact schema assertions.
- Calibration tests distinguish cached, extracted, scorable, candidate-ready,
  and persisted states; BPM-only inputs never become false readiness.
- Migration tests start from a v9 fixture, preserve profile rows, mark missing
  metadata incompatible, and verify atomic v10-style save/round-trip behavior.
- Fingerprint tests prove stable ordering and sensitivity to track/genre/audio
  identity changes without hashing absolute paths.
- Label tests use local fakes, cover provider precedence and error/no-match
  semantics, and reconcile per-provider/total counts.
- Full runtime, MCP smoke, site build, and doc-drift gates validate public
  contracts and embedded SOPs.

## Implementation results (2026-07-14)

Implemented immediately after Plan 034 in the same exact user-authorized
uncommitted post-Beatport-removal worktree. This preserved semantic ordering
but did not satisfy the planned clean-composite prerequisite.

The resulting implementation uses one album-aware Discogs interpreter for
classification and coverage, stores compatible profile metadata atomically,
suppresses legacy/incompatible profile rows without deleting them, rejects
BPM-only and otherwise unscorable calibration candidates before replacement,
and hydrates both MusicBrainz and Bandcamp label fallbacks while retaining
Discogs → MusicBrainz → Bandcamp precedence. Full runtime, migration, DSP,
release, MCP, site, and documentation gates passed. No analyzer version bump,
provider genre vote, cache purge, direct Rekordbox write, deploy, or release
was added.

## Done criteria

- [ ] The plan starts from reviewed Plan 034 on a clean Beatport-free base.
- [x] One typed interpreter defines Discogs not-searched, no-match,
      matched-unmapped, and usable-genre states.
- [x] Classification and coverage use the same exact album-aware cache key and
      taxonomy mapping.
- [x] Existing searched/has-result compatibility fields remain defined and
      additive usability/gap fields are documented.
- [x] Calibration readiness counts scorable samples/candidate prototypes, not
      merely `Some(AudioFeatures)`.
- [x] BPM-only inputs cannot make a genre ready to calibrate or erase a good
      registry.
- [x] Stored profiles have atomic metadata containing classifier/analyzer
      compatibility and stable training identity.
- [x] Legacy/mismatched profiles are reported and cannot silently vote.
- [x] Beatport removal alone does not force audio-profile recalibration.
- [x] Label auto-enrichment hydrates MusicBrainz and Bandcamp with unchanged
      Discogs → MusicBrainz → Bandcamp precedence.
- [x] Provider totals are additive and reconcile with `auto_enriched`.
- [x] No Bandcamp/MusicBrainz genre vote, cache purge, analyzer version bump,
      direct Rekordbox write, deploy, or release was added.
- [x] Targeted tests, migration tests, full gates, release build, MCP smoke,
      site build, and documentation contract all pass.

## STOP conditions

Stop and report rather than improvising if:

- the base is dirty, active Beatport Rust code remains, or Plan 034 is absent;
- coverage and classification no longer use the key/parser paths described
  above;
- implementing exact-key readiness would require changing Discogs cache primary
  keys or destructively rewriting enrichment rows;
- the scorer's optional-coverage rules cannot be reused or dry-run without
  broad classifier redesign;
- candidate validation would replace a compatible non-empty registry before
  the new candidate is proven usable;
- a migration would stamp legacy profile rows as current without evidence,
  delete them, or fail to save rows and metadata atomically;
- profile compatibility would require bumping analyzer cache schemas despite no
  analyzer-output change;
- label hydration requires changing provider precedence, making MusicBrainz a
  genre source, or calling real networks in tests;
- cancellation/error handling can leave a writer task or semaphore permit
  stranded;
- any user-visible metadata path bypasses `ChangeManager` or writes to
  `master.db`;
- files outside Scope require semantic edits; or
- two reasonable attempts cannot make a required verification command pass.

## Maintenance notes

Readiness is a state machine, not a percentage alias. Future coverage work must
use the same cache identity and parser as the consuming capability. A completed
provider no-match is operationally complete but musically unusable; an
exact/fuzzy provider payload can still be taxonomy-unmapped; a cached audio row
can still be unscorable. Preserve those distinctions in internal types and
public additive fields.

Profile metadata is local model provenance. Bump the classifier-profile schema
when calibration features, scoring coverage, prototype representation, or
confidence use changes. Reference analyzer schema constants rather than copying
their values into logic. Training changes should request recalibration without
pretending the stored bytes are structurally incompatible.

MusicBrainz label hydration closes an existing asymmetry; it is not a new genre
classifier. Defer Bandcamp tag weighting until Plan 034's real benchmark can
measure it, preserve legacy Beatport rows for rollback/A/B work, and leave
album/year cascades untouched.
