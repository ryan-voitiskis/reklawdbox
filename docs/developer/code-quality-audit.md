# Code Quality Audit — AI Agent Perspective

Conducted 2026-02-26. Covers all 18 Rust files in `src/` (~21k lines).

Evaluates how easy this codebase is for an LLM coding agent to correctly read,
modify, extend, and avoid introducing bugs in. Not concerned with human
readability or style.

---

## Findings by Severity

### Critical

These will likely cause agent-introduced bugs on the next non-trivial change.

#### C1. ~~`TrackChange` field set enumerated in 8+ locations with no compile-time enforcement~~ ✅ Resolved

`EditableField` enum in `types.rs` with `ALL` constant and `from_str()`/`as_str()`
methods. `clear_fields` uses `EditableField::from_str()` dispatch. `VALID_FIELDS`
replaced with `EditableField::from_str()` + `all_names_csv()`. Compile-time
field-count assertion test catches divergence between `TrackChange` fields and
`EditableField` variants.

#### C2. ~~`SearchParams` construction copy-pasted 6 times~~ ✅ Resolved

`SearchParams` now derives `Default`. All 6 tool-site constructions replaced
with `SearchFilterParams::into_search_params()`. All 18 test constructions use
`..Default::default()`. Adding a new filter field only requires updating
`SearchFilterParams` and `into_search_params`.

#### C3. ~~`tools.rs` is 8138 lines — exceeds agent context windows~~ ✅ Resolved

Split into `src/tools/` module directory: `mod.rs` (2899 lines, tool methods only),
`params.rs`, `scoring.rs`, `corpus_helpers.rs`, `enrichment.rs`, `essentia.rs`,
`resolve.rs`, `audio_scan.rs`, `tests.rs`.

#### C4. ~~`IssueType` enum: 7 update sites, only `as_str()` is compiler-enforced~~ ✅ Resolved

`strum::EnumString` + `strum::Display` derives replace manual `as_str()`/`from_str()`.
`safety_tier()` catch-all removed — new variants get a compile error until
assigned a tier.

#### C5. ~~`OnceLock` caches credential errors permanently~~ ✅ Resolved

`OnceLock<Result<T, E>>` replaced with `OnceLock<T>` in both `discogs.rs` and
`corpus.rs`. Errors are returned without caching; only successful values are
stored via `get_or_init`. Dead `CorpusError::Load` variant removed.

---

### High

Significant friction; partial bugs likely on non-trivial changes.

#### H1. ~~4+ duplicate parameter structs with identical filter fields~~ ✅ Resolved

Extracted `SearchFilterParams` with 12 shared filter fields. All 4 param
structs now use `#[serde(flatten)] pub filters: SearchFilterParams`.
`SearchFilterParams::into_search_params()` provides the conversion to
`db::SearchParams`. MCP tool JSON schemas are unchanged (`schemars` inlines
flattened fields).

#### H2. ~~Track resolution pattern duplicated 4 times with subtle differences~~ ✅ Resolved

Extracted `resolve_tracks()` in `tools/resolve.rs` with `ResolveTracksOpts`
struct handling bounded/unbounded queries and sampler filtering. All 4 call
sites (`enrich_tracks`, `analyze_audio_batch`, `resolve_tracks_data`,
`cache_coverage`) use the shared helper.

#### H3. ~~`AUDIO_EXTENSIONS` defined identically in 3 files~~ ✅ Resolved

Single `pub(crate) const AUDIO_EXTENSIONS` in `audio.rs`, referenced by
`cli.rs`, `audit.rs`, and `tools.rs`.

#### H4. ~~Audio analysis cache flow duplicated between single and batch tools~~ ✅ Resolved

Extracted `check_analysis_cache()`, `analyze_stratum()`, `analyze_essentia()`,
and `cache_analysis()` into `tools/analysis.rs`. Both `analyze_track_audio` and
`analyze_audio_batch` use the shared helpers.

#### H5. ~~Provider dispatch via raw strings instead of enum~~ ✅ Resolved

`enum Provider { Discogs, Beatport }` in `types.rs` with `Serialize`/`Deserialize`.
String dispatch replaced with enum match in tool handlers.

#### H6. ~~Genre-to-family mapping disconnected from genre taxonomy~~ ✅ Resolved

`GenreFamily` enum and `genre_family()` moved from `tools/scoring.rs` to
`genre.rs`. Uses exact canonical names from `GENRES` (no lowercasing). Added
missing genres (`Gospel House`, `Progressive House`, `Drone Techno`,
`Dub Reggae`, `Disco`). Removed stale aliases (`"drum and bass"`, `"rnb"`,
`"synth pop"`). Test verifies all `GENRES` entries are covered.

#### H7. ~~`TRACK_SELECT` string replacement silently fragile~~ ✅ Resolved

`debug_assert_ne!` added after the replacement to catch silent no-ops during
development and testing. No behavior change in release builds.

#### H8. ~~Audit status/resolution strings cross module boundaries untyped~~ ✅ Resolved

`enum AuditStatus` and `enum Resolution` in `audit.rs` with `strum` derives.
Bare string literals and catch-all mappings replaced with enum variants.

#### H9. ~~Dynamic SQL parameter numbering in `get_audit_issues`~~ ✅ Resolved

Replaced manual `?4`/`?5` numbering and 4-arm match with dynamic
`Vec<Box<dyn ToSql>>` builder and `params_from_iter`. Adding new filters
requires only a new `if let` block.

#### H10. ~~Embedded 185-line Python script with no schema contract~~ ✅ Resolved

`EssentiaOutput` struct with `Serialize`/`Deserialize` + `#[serde(default)]`
in `audio.rs`. `run_essentia()` and `parse_essentia_stdout()` return typed
struct. All downstream consumers use typed field access.

#### H11. ~~Beatport HTML parsing silently fragile~~ ✅ Resolved

`parse_beatport_html()` now returns `Err(String)` with descriptive messages
for parse failures (missing `__NEXT_DATA__`, malformed JSON, missing path,
non-array queries). `Ok(None)` reserved for legitimate "no matching track".

#### H12. ~~String-typed closed sets: `FieldDiff.field`, `NormalizationSuggestion.confidence`~~ ✅ Resolved

`enum Confidence { Alias, Unknown, Canonical }` in `types.rs` with
`Serialize`/`Deserialize`. `EditableField` enum added (see C1). `FieldDiff.field`
remains `String` for JSON serialization but values are produced via
`EditableField::as_str()`.

#### H13. ~~Blocking subprocess in async tool handler~~ ✅ Resolved

Backup script call wrapped in `tokio::task::spawn_blocking()`, matching the
existing pattern used for essentia validation.

---

### Medium

Notable issues. Manageable with awareness but easy to get wrong.

#### Duplication

| ID | Issue | Files |
|----|-------|-------|
| M1 | ~~Duplicate `escape_like`~~ ✅ Resolved — `store.rs` now imports `crate::db::escape_like` | |
| M2 | ~~Duplicate `urlencoding`~~ ✅ Resolved — `beatport.rs` now imports `crate::discogs::urlencoding` | |
| M3 | ~~Essentia storage code duplicated in two `cli.rs` branches~~ ✅ Resolved — `run_and_cache_essentia()` helper | |
| M4 | ~~Status aggregation duplicated between `scan()` and `get_summary()`~~ ✅ Resolved — `aggregate_status_counts()` helper | |
| M5 | ~~Disc-subdir detection duplicated~~ ✅ Resolved — `is_disc_subdir()` helper | |

#### Module Boundaries & Coupling

| ID | Issue | Files |
|----|-------|-------|
| M6 | `normalize()` in `discogs.rs` used universally (28+ call sites in tools.rs) | `discogs.rs:151` |
| M7 | ~~Inconsistent whitespace: `genre::canonical_casing` trims, `color::canonical_casing` doesn't~~ ✅ Resolved — `color::canonical_casing` now trims | |
| M8 | Inconsistent normalization: Discogs strips punctuation, Beatport preserves it | `discogs.rs:151`, `beatport.rs:195` |
| M9 | Inconsistent matching: Discogs uses `contains`, Beatport uses exact equality | `discogs.rs:583`, `beatport.rs:207` |
| M10 | `color_code == 0` means "no color" — black (`0x000000`) unrepresentable | `xml.rs:126`, `changes.rs:293` |
| M11 | Rekordbox XML attribute names differ from struct fields (`Name`/`title`, `Tonality`/`key`) | `xml.rs:80-106` |

#### Error Handling & Silent Failures

| ID | Issue | Files |
|----|-------|-------|
| M12 | ~~Reopened audit issues keep stale `resolution`/`resolved_at`/`note`~~ ✅ Resolved — ON CONFLICT clears stale fields | |
| M13 | ~~`get_tracks_by_ids` doesn't preserve caller order, deduplicates~~ ✅ Resolved — returns tracks in caller order with dedup | |
| M14 | Dry-run tag diff can claim RIFF changes that write path will skip | `tags.rs:864,874,748` |
| M15 | Beatport title matching: permissive bidirectional substring, false-positive prone | `beatport.rs:220` |
| M16 | Discogs short artist names (<3 chars) auto-match first result | `discogs.rs:580,547` |
| M17 | ~~Audio decode tolerates frame errors with only stderr logging~~ ✅ Resolved — single summary line after decode loop | |
| M18 | ~~Corrupt cache JSON becomes `null` while response still says "cache hit"~~ ✅ Resolved — parse errors surfaced in output JSON | |
| M19 | ~~`DJPlayCount` dual-type parse failures collapse to 0~~ ✅ Resolved — logs non-empty parse failures via eprintln | |
| M20 | ~~`TECH_SPEC_PATTERNS` lists `[FLAC]`/`[flac]` but misses mixed-case~~ ✅ Resolved — case-insensitive matching | |
| M21 | ~~`date` field in `check_tags` not in `tags::ALL_FIELDS` — no-op for non-WAV~~ ✅ Resolved — removed no-op `date` check | |
| M22 | Corpus manifest path is cwd-relative, creating non-local behavior | `corpus.rs:9,180` |

#### Type Safety

| ID | Issue | Files |
|----|-------|-------|
| M23 | `Track` struct is flat primitives with no newtypes (`rating: u8`, `file_type: i32`) | `types.rs:4-30` |
| M24 | ~~Priority weights returned as anonymous 6-tuple~~ ✅ Resolved — `PriorityWeights` named struct | |
| M25 | ~~Energy computation uses undocumented magic numbers~~ ✅ Resolved — named constants extracted | |
| M26 | `write_track` two-phase attribute writing (main write + conditional appends + close) | `xml.rs:75-131` |
| M27 | ~~Migration mixes unconditional `CREATE TABLE IF NOT EXISTS` with version-gated blocks~~ ✅ Resolved — all DDL unconditional | |
| M28 | ~~Migration logic assumes `user_version` implies audit tables exist~~ ✅ Resolved — version gate removed | |
| M29 | All errors in `audio.rs`, `tags.rs`, `beatport.rs` are `Result<_, String>` | Multiple |
| M30 | `serde(untagged)` enums in `tags.rs` make deser errors opaque | `tags.rs:117-217` |
| M31 | ~~Analyzer name strings (`"stratum-dsp"`, `"essentia"`) used as DB keys with no constant~~ ✅ Resolved — `ANALYZER_STRATUM`/`ANALYZER_ESSENTIA` constants | |

---

### Low

Minor issues. Documented for completeness.

| ID | Issue | Files |
|----|-------|-------|
| L1 | ~~Manual SQL `BEGIN`/`COMMIT` with `?` exits risks partial transactions~~ ✅ Resolved — `unchecked_transaction()` with auto-rollback | |
| L2 | Unreadable directories in CLI expansion silently ignored | `cli.rs:546` |
| L3 | Malformed broker URL treated as missing config | `discogs.rs:24` |
| L4 | Legacy Discogs HTTP errors drop response-body diagnostics | `discogs.rs:530` |
| L5 | Issue detail JSON parse failure silently dropped | `audit.rs:1057` |
| L6 | Poisoned mutex silently recovered in staged changes | `changes.rs:20` |
| L7 | `file_type_to_kind` catch-all `_ => "Audio File"` | `types.rs:99-108` |
| L8 | Hardcoded Rekordbox version `"7.2.10"` in XML | `xml.rs:146-148` |
| L9 | Hardcoded User-Agent with Chrome 91 (2021) in Beatport scraper | `beatport.rs:4-5` |
| L10 | Double-negative CLI flag `--no-skip-cached` | `cli.rs:68` |
| L11 | `SAMPLER_PATH_PREFIX` hardcoded to `/Users/vz/...` | `db.rs:110` |
| L12 | Backup script discovery is cwd-relative | `tools.rs:1541-1558` |
| L13 | ~~`stars_to_rating(6)` silently returns 255; `rating_to_stars(300)` returns 0~~ ✅ Resolved — out-of-range values saturate to 5 stars | |
| L14 | ~~No-op `touch_cached_*` functions (dead scaffolding)~~ ✅ Resolved — dead functions removed | |
| L15 | No-op writes still trigger file rewrites in tags | `tags.rs:768` |

---

### Context-Dependent (Partially Confirmed)

These are technically present but mitigated by other code paths today. Worth
monitoring because the mitigation is in a different module — an agent modifying
one side could break the contract.

| ID | Issue | Mitigation | Risk if mitigation changes |
|----|-------|-----------|--------------------------|
| P1 | Unknown resolution coerces to `"resolved"` in store layer | Tool layer validates first (`audit.rs:1094`) | Direct store calls bypass validation |
| P2 | Duplicate track IDs overwrite in XML mapping | `write_xml` deduplicates first | Direct `generate_xml_with_playlists` calls don't |
| P3 | Hard-coded CLI/server dispatch string list | Currently aligned with declared commands | Adding CLI command without updating `main.rs:29` |
| P4 | Raw string date comparisons in SQL | Safe if ISO-8601 format invariant holds | User-supplied dates could violate |
| P5 | `mtime` fallbacks use epoch 0 | Only risky if `modified()` fails frequently | Platform-specific edge cases |

---

## Systemic Themes

### 1. String-typed closed sets instead of enums

Largely resolved. Remaining: field diff names (still `String` for JSON compat
but produced via `EditableField::as_str()`).

**Affected findings**: ~~C1~~, ~~C4~~, ~~H5~~, ~~H8~~, ~~H12~~, ~~M31~~

### 2. Copy-paste with subtle variation

Fully resolved.

**Affected findings**: ~~C2~~, ~~H1~~, ~~H2~~, ~~H4~~, ~~H3~~, ~~M1~~, ~~M2~~, ~~M3~~, ~~M4~~, ~~M5~~

### 3. Cross-module implicit contracts

Color `0 = unset` (M10), XML attribute names vs struct fields (M11). Modifying
one side of these contracts gives no signal about the other.

**Affected findings**: ~~H6~~, ~~H7~~, ~~H8~~, ~~H10~~, M10, M11

### 4. ~~Monolith file~~ ✅ Resolved

`tools.rs` split into 9-file module directory. `mod.rs` is 2899 lines
(tool methods only). Submodules handle params, scoring, enrichment, etc.

**Affected findings**: ~~C3~~

### 5. Silent error swallowing

Multiple sites return `Ok(None)` or `Ok(0)` where an error occurred. This is
particularly dangerous for agents because there's no signal that something
went wrong — the agent assumes success and moves on.

**Affected findings**: ~~H11~~, ~~M12~~, M15, M16, ~~M17~~, ~~M18~~, ~~M19~~, L3-L6

---

## Suggested Fix Priority

Highest-impact changes ordered by (bug prevention * effort ratio):

1. ~~**Derive `Default` for `SearchParams`**~~ ✅ Done — eliminates C2
2. ~~**`enum EditableField`**~~ ✅ Done — eliminates C1 + H12
3. ~~**Split `tools.rs`**~~ ✅ Done — unlocks all other refactors (C3)
4. ~~**`enum AuditStatus` + `enum Resolution`**~~ ✅ Done — eliminates H8
5. ~~**`strum` derives for `IssueType`**~~ ✅ Done — eliminates C4
6. ~~**Extract `SearchFilterParams`**~~ ✅ Done — eliminates H1
7. ~~**Extract `resolve_tracks()` helper**~~ ✅ Done — eliminates H2
8. ~~**Shared `AUDIO_EXTENSIONS` constant**~~ ✅ Done — eliminates H3
9. ~~**`enum Provider`**~~ ✅ Done — eliminates H5
10. ~~**`EssentiaOutput` struct**~~ ✅ Done — eliminates H10
