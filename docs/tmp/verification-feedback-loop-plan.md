# Verification Feedback Loop & Calibration Coverage: Implementation Plan

**Date:** 2026-04-26
**Status:** Design proposal. No implementation yet.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) sections D3 and D4. Builds on the existing calibration entry point at `src/tools/classify_handler.rs:725` and the staged-XML write path at `src/tools/staging_handlers.rs:283`.

## 1. Goal

Close the verified-track feedback loop so that ear-verified classifications produced by the agent (or user, in conversation) flow into Fisher-prototype training without manual playlist curation in Rekordbox. Surface per-genre verification coverage so the user can prioritise verifying the genres closest to crossing the prototype-training threshold (`MIN_TRACKS = 5` at `src/audio_profile.rs:24`).

Two deliverables:

1. **D3 — `verify_track` MCP tool.** Stages a genre change (existing path) and marks the track as ear-verified for inclusion in the next calibration run.
2. **D4 — `calibration_coverage` MCP tool.** Per-genre report of verified-N, threshold gap, prototype state, and suggested next action.

Tonight's manual-tagging session produced four verified classifications (track IDs 170960182, 203991790, 113107185, 59342610 — all from the POM POM "Lost Tracks" comp) that did not enter prototype training because the user did not also add them to the `genre_verified` Rekordbox playlist. D3 prevents that loss going forward; D4 makes the gap visible.

## 2. Constraints

- **Read-only Rekordbox DB.** The encrypted `master.db` is opened read-only via SQLCipher (per `CLAUDE.md` and `src/tools/mod.rs:115`). No write path exists. The `verify_track` action cannot directly add a track to the Rekordbox `genre_verified` playlist nor write a Rekordbox MyTag.
- **Staged XML is the only outbound write path.** Genre/album/label/year/comment/rating edits flow through `update_tracks` → `preview_changes` → `write_xml` → user reimport. Playlists can be exported as part of `write_xml` via `xml::PlaylistDef` (`src/xml.rs:11`, wired in `staging_handlers.rs:303–376`), but the staged playlist only takes effect after the user manually reimports the XML in Rekordbox.
- **Calibration reads the playlist.** `handle_calibrate_audio_profiles` at `classify_handler.rs:729–750` resolves a playlist by name (default `genre_verified`) directly from `djmdContent` joined to the playlist tables. Anything that bypasses Rekordbox is invisible to the existing calibration entry until the calibration code is taught a new source.
- **No MyTag write path exists.** A grep for `MyTag` in `src/` returns no matches; the XML writer at `src/xml.rs:208` (`write_xml_with_playlists`) emits the standard Rekordbox 7.2.10 schema (TARGET_REKORDBOX_VERSION at `src/xml.rs:8`) which has no MyTag elements. Adding MyTag support would be a separate workstream and is not on the path of D3.
- **Compiled SOPs.** SOP `.mdx` files in `site/src/partials/sops/` are `include_str!`'d into `src/tools/help_handler.rs:8–20`. SOP changes ship with a release, not at runtime.

## 3. D3 — `verify_track` Design

### 3.1 Three options for marking verified

**Option A — Stage a `genre_verified` playlist add via XML.**
On `verify_track(track_id, genre)`, stage (a) a genre change in `ChangeManager` and (b) an entry in a new in-memory `verified_playlist_buffer` keyed off the `genre_verified` playlist name. On `write_xml`, the buffer is emitted as a `xml::PlaylistDef` (already supported, see `staging_handlers.rs:370–376`). When the user reimports the XML, Rekordbox merges the playlist add. Calibration then reads the playlist as it does today (`classify_handler.rs:729–750`).
- Pro: zero changes to calibration; reuses existing XML playlist support.
- Con: verification only takes effect after the user runs the import workflow. Until then, the in-memory buffer is the only record. If the agent crashes between stage and export, verifications are lost.
- Con: `xml::PlaylistDef` semantics overwrite the playlist contents on reimport — need to check whether the existing `genre_verified` playlist content is preserved when the staged XML's playlist with the same name is imported. Per `xml.rs:208` the writer emits the union of staged tracks plus playlist track_ids — staging only the *new* additions would replace the playlist with just the new ones unless the existing 574 tracks are also resnapshot into the staged playlist. This is a real footgun.

**Option B — Rekordbox MyTag.**
Use a "verified" MyTag. Calibration reads tracks with this MyTag instead of (or in addition to) the playlist.
- Pro: cleaner mental model (MyTag is per-track, doesn't touch playlists).
- Con: no MyTag write path exists in the XML writer or anywhere else. Adding one is its own workstream and out of scope for closing the verification loop. Reject.

**Option C — Local-only verified registry.**
New SQLite table in the cache store: `verified_tracks(track_id TEXT, genre TEXT, verified_at TEXT, source TEXT, PRIMARY KEY(track_id))`. `verify_track` writes the row immediately. Calibration reads from this table *in addition to* the legacy `genre_verified` playlist (union, deduped by `track_id`).
- Pro: instant. No XML round-trip required for verification to take effect on the next `calibrate_audio_profiles` call.
- Pro: stable — survives crashes, doesn't depend on user reimporting XML.
- Pro: the existing 574-track `genre_verified` playlist remains the authoritative bootstrap source; the registry just adds to the union.
- Con: lives outside the user's existing organization. The user can no longer just look at the Rekordbox playlist to see the full verified set.

**Option D (hybrid, recommended).**
Take Option C's instant local registry as the source of truth for calibration, *and* stage an XML playlist-add (Option A's mechanism) so the Rekordbox playlist eventually catches up on the next `write_xml`. Calibration reads `verified_tracks ∪ genre_verified` playlist (deduped). The registry is authoritative for "is this track verified for calibration"; the playlist mirrors it for user visibility in Rekordbox.

**Recommendation: Option D.** Verifications take effect immediately for the next calibration run (instant feedback loop, the primary goal of D3) while still mirroring into the user's existing organisational structure. Avoids the Option A footgun by treating the playlist as a *visibility* mirror rather than the calibration source — if the staged playlist replaces the existing one in Rekordbox on reimport, the user can rebuild from the registry; nothing's lost. The 574-track bootstrap set lives in the playlist today; the union read in calibration means we don't need to migrate it into the registry.

### 3.2 MCP tool surface

```
verify_track(track_id: String, genre: String, source: Option<String>) -> Result
verify_tracks(verifications: Vec<{track_id, genre}>, source: Option<String>) -> Result
```

- `source` is one of `"user"` | `"agent"` | `"sop"` (default `"agent"`). Stored in the registry for audit.
- Single-track and batch shapes coexist: tonight's per-track approval pattern needs single calls, but the SOP's medium-tier auto-approval flow (Step 3 of `genre-classification.mdx:62`) wants batched.
- Return shape:
  ```
  {
    "verified": <count>,
    "skipped": [{"track_id", "reason"}],
    "overwrote": [{"track_id", "old_genre", "new_genre"}],
    "staged_changes": <count>,        // genre changes added to ChangeManager
    "coverage_delta": {                // optional, just-crossed-threshold summary
      "newly_eligible": ["Deep Techno"],
      "still_blocked": ["Ambient Techno (3/5)"]
    }
  }
  ```

### 3.3 Behaviour

1. **Validation.** `genre` must resolve to a canonical name via `genre::resolve_genre` (`src/genre.rs:74`). Reject otherwise with the canonical list in the error message — same UX as `update_tracks` warning at `staging_handlers.rs:58–62`.
2. **Idempotency.** Re-verifying a track with the same genre is a no-op (no row update, no staged change). With a different genre, overwrites the registry row (`INSERT OR REPLACE`) and stages the new genre change. Logs an `overwrote` entry in the response.
3. **Staged-genre interaction.** `verify_track` always stages a `TrackChange { genre: Some(g), .. }` via the existing `ChangeManager::stage` API (`staging_handlers.rs:84`), even if the track already has that genre in the Rekordbox DB — verifying is a stronger statement than the genre alone, and the staged change is a no-op preview if values match. If the user later runs `clear_changes`, the staged genre is dropped but the registry entry persists.
4. **Disagreement with the existing playlist.** If the track is already in the `genre_verified` Rekordbox playlist with a different genre tag, overwrite the registry, stage the new genre, and emit an audit log entry: `{"track_id", "playlist_genre", "new_genre", "verified_at"}` written to a `verified_audit_log` table (append-only). Rationale: the user is the source of truth in conversation; the playlist is stale.
5. **Atomicity.** The registry write and the `ChangeManager::stage` call must succeed together. If either fails, neither is applied. Implement as: stage to ChangeManager first (in-memory, rollback-cheap), then write to SQLite; on SQLite error, rollback the stage via `ChangeManager::clear` for that single track_id.
6. **Playlist mirror.** A second in-memory buffer `verified_playlist_pending: Vec<String>` collects track_ids since the last `write_xml`. When `write_xml` runs, the buffer is appended to the user-supplied `playlists` array as a `PlaylistDef { name: "genre_verified", track_ids: <buffer> }` — but only if the buffer is non-empty *and* the user hasn't already passed a `genre_verified` playlist explicitly. Buffer clears on successful export. Document the playlist-overwrite behaviour clearly in the tool description so the user knows reimporting will replace the existing playlist with the staged additions only — they should re-export with all known verified IDs if they want a full mirror, or skip the playlist-mirror entirely (registry alone is sufficient for calibration).

### 3.4 Calibration source change

`handle_calibrate_audio_profiles` at `classify_handler.rs:729–750` currently reads only the playlist. Change to: union of `(playlist tracks, registry tracks)` deduped by `track_id`. Per-track resolution (genre, audio features) follows the existing flow at lines 766–800. The registry's `genre` field is treated as authoritative if a track appears in both with different genres (registry is newer).

Add a parameter `CalibrateAudioProfilesParams { include_registry: Option<bool>, ... }` defaulting to `true` so legacy callers can still scope to playlist-only.

### 3.5 Calibration auto-trigger

Do **not** auto-recalibrate on every `verify_track` call. Calibration is O(n_verified × feature_extraction) and reads through the entire cache store; per-call recalibration is expensive and pointless when most calls don't cross MIN_TRACKS for any genre.

Instead, in the `verify_track` response include a `coverage_delta` field: if this verification just crossed MIN_TRACKS for a genre, surface `"newly_eligible": ["Deep Techno"]` with a hint string `"Run calibrate_audio_profiles to incorporate."`. The user (or SOP) decides when to run.

A nightly scheduled recalibration is out of scope; the manual prompt is sufficient for current usage volumes (~1–10 verifications per session).

### 3.6 SOP integration

In `site/src/partials/sops/genre-classification.mdx`, Step 4.3 (verify staged results, line 157) is the natural place to introduce verification. Proposed wording (release-gated, since SOPs are `include_str!`'d):

```
### 4.4 Verify approved classifications

For tracks the user has explicitly confirmed during review (steps 3 and 4),
mark them ear-verified to feed the prototype training set:

verify_tracks(verifications=[{track_id, genre}, ...], source="sop")

This stages the genre change (no separate update_tracks call needed) and
records the verification for the next calibrate_audio_profiles run.

If the response includes `coverage_delta.newly_eligible`, mention to the
user that those genres now have enough verifications to build prototypes,
and suggest running calibrate_audio_profiles after import.
```

In Step 1 (Prerequisites), add a line:

```
calibration_coverage()
```

so the agent surfaces verified-N gaps before classifying. The output guides which genres to prioritise verifying during review.

## 4. D4 — `calibration_coverage` Design

### 4.1 Tool surface

New MCP tool registered in `src/tools/mod.rs` next to `calibrate_audio_profiles` (around line 597):

```
calibration_coverage(genre: Option<String>) -> Result
```

`genre` filter is optional; when set, returns only that genre's row. Cache-only (reads from the cache store at `cache_store_conn`, no Rekordbox DB writes, no external calls).

**Recommendation against extending `cache_coverage`:** that tool is scoped to per-track cache hit-rate and uses `ResolveTracksDataParams` filters (`resolve_handlers.rs:191`). Calibration coverage is per-genre and global — different scope, different output shape. A separate tool is the cleaner factoring. Keep `cache_coverage` for what it does today.

### 4.2 Output structure

Sorted to surface the most actionable items first. Three sections:

```json
{
  "summary": {
    "total_verified": 578,
    "genres_with_prototypes": 12,
    "genres_blocked": 5,
    "min_tracks_threshold": 5,
    "last_calibrated_at": "2026-04-10T09:14:33Z",
    "next_to_verify": "Deep Techno"
  },
  "ready_to_calibrate": [
    {"genre": "Deep Techno", "verified_n": 5, "delta": "+1 since last calibration", "prototype": null}
  ],
  "blocked": [
    {"genre": "Ambient Techno", "verified_n": 3, "needed": 2, "hint": "2 more verifications to build prototype"},
    {"genre": "Tech House", "verified_n": 0, "needed": 5, "hint": "no verified tracks — start tagging or skip in classification"}
  ],
  "trained": [
    {
      "genre": "Drum & Bass",
      "verified_n": 14,
      "prototype": {
        "last_calibrated_at": "2026-04-10T09:14:33Z",
        "n_features": 8,
        "has_timbral": true,
        "top_discriminators": ["onset_rate (38%)", "decay_high_tau (22%)"]
      }
    }
  ]
}
```

Sort order:
- `ready_to_calibrate` first — these are the highest-leverage actions (one click away from a new prototype).
- `blocked` next, sorted by `verified_n` descending (closest to crossing threshold first). Genres with `verified_n == 0` are shown last in this section, since they need bulk tagging not single verifications.
- `trained` last, sorted by `last_calibrated_at` ascending (stalest first — surfaces prototypes that may have drifted as new verified tracks accumulated).

Top-level `summary.next_to_verify` picks the genre with the smallest non-zero `needed` value. Tied → alphabetical.

### 4.3 Where the data comes from

| Field | Source |
|---|---|
| `verified_n` | UNION: count of distinct track_ids in (`verified_tracks` registry where `genre = X` ∪ `genre_verified` playlist where canonical genre = X). Resolved to canonical via `genre::resolve_genre`. |
| `min_tracks_threshold` | Constant `MIN_TRACKS` from `audio_profile.rs:24`. |
| `prototype.last_calibrated_at` | `genre_audio_profiles.updated_at` (existing column, `store.rs:127`). |
| `prototype.n_features` | `COUNT(*)` per genre in `genre_audio_profiles` (one row per feature per `store.rs:120–129`). |
| `prototype.has_timbral` | `EXISTS` in `genre_timbral_centroids` for that genre (`store.rs:130`). |
| `prototype.top_discriminators` | `SELECT feature, fisher_weight FROM genre_audio_profiles WHERE genre = X ORDER BY fisher_weight DESC LIMIT 5`. Same shape as the existing `top_discriminators` in `handle_calibrate_audio_profiles` at `classify_handler.rs:821–832`. |
| `summary.last_calibrated_at` | `MAX(updated_at)` across `genre_audio_profiles`. |

Reading the playlist requires the Rekordbox connection (`rekordbox_conn`); registry and prototype tables are in the cache store (`cache_store_conn`). The handler opens both, same as `handle_calibrate_audio_profiles` does today.

### 4.4 Genre enumeration

The full canonical genre list (`genre::GENRES` at `src/genre.rs:6`) is the row set, so genres with zero verified tracks still appear (with `hint: "no verified tracks"`). This catches the "Tech House: 0 verified — start tagging or skip in classification" case in the brief.

## 5. SOP Updates

Both SOPs ship in `site/src/partials/sops/` and are embedded at compile time via `help_handler.rs:8–20`. Changes require a release.

### `genre-classification.mdx`

- **Prerequisites (line 9):** add `calibration_coverage()` to the prerequisites list. Annotate: "shows which genres need verification — prioritise these during review."
- **Step 4.3 (line 157):** rename to "Verify approved classifications", flip to using `verify_tracks` instead of (or in addition to) `update_tracks` for the user-confirmed subset of medium/low/insufficient tracks. Note the calibration-trigger hint pattern.
- **Step 5 (Export, line 171):** add a sentence: "if `verify_tracks` was called this session and a `genre_verified` playlist mirror is desired, pass `playlists=[{name: 'genre_verified', track_ids: <verified IDs from this session>}]` to `write_xml`. Otherwise the registry alone drives calibration."

### `genre-audit.mdx`

If the audit SOP currently has any reference to manual playlist curation, replace with `verify_track`. (Worth checking during PR 3 — the file at `site/src/partials/sops/genre-audit.mdx` may already be silent on this.)

### `library-health.mdx`

Optional: surface `calibration_coverage()` in the health-check menu so the user has a one-shot view of where verification effort should land. Low priority, not required for v1.

## 6. SQLite Schema Additions

In `src/store.rs:120–144`, add two tables alongside the existing prototype tables:

```sql
CREATE TABLE IF NOT EXISTS verified_tracks (
    track_id     TEXT PRIMARY KEY,
    genre        TEXT NOT NULL,
    source       TEXT NOT NULL,            -- 'user' | 'agent' | 'sop'
    verified_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_verified_tracks_genre ON verified_tracks(genre);

CREATE TABLE IF NOT EXISTS verified_audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    track_id     TEXT NOT NULL,
    old_genre    TEXT,
    new_genre    TEXT NOT NULL,
    source       TEXT NOT NULL,
    note         TEXT,                    -- free-form: 'overwrote_playlist', 'overwrote_registry', etc.
    logged_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_verified_audit_track ON verified_audit_log(track_id);
```

Bump the schema constant if the project uses `STORE_SCHEMA_VERSION`-style migration guards (see `store.rs:148`'s `pragma_update`); otherwise the `IF NOT EXISTS` is sufficient since both tables are additive.

## 7. MCP Tool Registration

In `src/tools/mod.rs`:

- Add `verify_track`, `verify_tracks`, `calibration_coverage` tool decorations next to `calibrate_audio_profiles` (current line 594). Each follows the existing `#[tool(description = "...")]` pattern at lines 209–600.
- New params types in `src/tools/params.rs`: `VerifyTrackParams`, `VerifyTracksParams`, `CalibrationCoverageParams` — model on the existing `CalibrateAudioProfilesParams` at `params.rs:401`.
- New handler functions in a new file `src/tools/verify_handlers.rs` (registered in `mod.rs:9–36`). Handler signatures:
  - `pub(super) fn handle_verify_track(server: &ReklawdboxServer, params: VerifyTrackParams) -> Result<CallToolResult, McpError>`
  - `pub(super) fn handle_verify_tracks(...)`
  - `pub(super) fn handle_calibration_coverage(server: &ReklawdboxServer, params: CalibrationCoverageParams) -> Result<CallToolResult, McpError>`
- New module `src/verified_registry.rs` with `record_verification`, `read_verified_set`, `audit_log_overwrite`, `coverage_summary` functions. Pure SQLite, no MCP types — testable in isolation.

## 8. Integration with Calibration Trigger

`handle_calibrate_audio_profiles` at `classify_handler.rs:725` expands its source set:

```
verified_set = playlist_tracks ∪ registry_tracks
            (deduped by track_id; registry genre wins on conflict)
```

The existing per-track loop at lines 766–800 runs unchanged on the unioned set. The summary at lines 817–847 already groups by genre, so nothing downstream changes shape.

Add a `source_breakdown` field to the summary so the user sees how many came from each source:

```json
"source_breakdown": {
  "playlist_only": 570,
  "registry_only": 4,
  "both": 4
}
```

This makes the "tonight's 4 verifications fed in" feedback loop visible.

## 9. Validation

Per project convention (small ear-verified fixture sets, mirrors `genre-classification-improvements.md`):

1. **Unit tests** in `src/verified_registry.rs`:
   - Idempotent same-genre re-verify is a no-op.
   - Different-genre re-verify writes audit log entry and overwrites.
   - Registry rollback when `ChangeManager::stage` is reverted (atomicity test).
2. **Integration test** in `src/tools/tests.rs`:
   - `verify_track` then `calibration_coverage` reflects the new count.
   - `verify_tracks` batch then `calibrate_audio_profiles` produces a prototype if MIN_TRACKS crossed.
   - `verify_track` with non-canonical genre returns `invalid_params`.
3. **Smoke test** post-deploy (per `CLAUDE.md` MCP Development Loop):
   - Run `./scripts/deploy-local.sh`, ask user to `/mcp` reconnect.
   - Call `calibration_coverage()` and confirm output matches the live state of the cache.
   - Call `verify_track(track_id="170960182", genre="Downtempo")` (one of tonight's tracks). Confirm `coverage_delta` and that calling `calibration_coverage()` again shows verified_n incremented.
   - Edge case: call `verify_track` with a non-canonical genre (e.g. "Liquid DnB"). Confirm rejection with helpful message.

## 10. Risks

1. **Verification rot.** Tracks verified months ago may have been retagged in Rekordbox to a different genre that the user now prefers. The registry won't know. Mitigation: include a `verified_at` filter in `calibration_coverage` so the user can spot stale verifications. Optional later: a `re_verify_track` flow that re-confirms a stale entry.
2. **Stale entries when files move.** If a track's Rekordbox `track_id` changes (rare, but possible on full library rebuild) the registry entry orphans. Mitigation: on `calibrate_audio_profiles`, drop registry rows whose `track_id` no longer resolves in `djmdContent`. Log dropped count in the response.
3. **Registry vs playlist drift.** The user could still hand-edit the `genre_verified` playlist in Rekordbox without going through `verify_track`. The union read handles this gracefully — playlist-only entries still feed calibration. Drift is benign.
4. **Conflict resolution loop.** A track in the registry as Deep Techno but in the playlist as Techno: which wins? Decision: registry wins (more recent, captures user-in-conversation intent). Audit log entry for visibility. If the user disagrees, they can either re-verify with the playlist's genre or `clear_caches` the registry.
5. **`write_xml` playlist mirror overwrites.** As noted in §3.1, exporting a `genre_verified` playlist with N entries when Rekordbox already has 574 will replace, not merge, on reimport. Mitigation: when staging the playlist mirror for export, prefill with the union (registry ∪ existing playlist tracks read from Rekordbox at export time) so the exported playlist contains *everything*, not just session adds. Keeps Rekordbox in sync without data loss. This is a real semantic gotcha to call out in the PR.
6. **Bootstrap correctness.** The user's existing 574-track `genre_verified` playlist remains the bootstrap source. D3 does not migrate it into the registry; the union read uses both. If the user later wants the registry to be the sole source, a one-shot import command can be added — out of scope for v1.
7. **Performance.** Registry reads are O(verified_count); current verified count is ~574 + a handful per session. SQLite scans dominated by the per-track audio-feature extraction in calibration, not by the registry read. No performance concern at current scale.

## 11. PR Breakdown

Three PRs in order, each independently shippable.

### PR 1 — `calibration_coverage` tool (read-only, low risk)

- New table `verified_tracks` in `src/store.rs` (created but unused initially — schema-only).
- New module `src/verified_registry.rs` with `coverage_summary` (returns the per-genre breakdown reading from playlist + registry; registry will be empty until PR 2).
- New handler `src/tools/verify_handlers.rs::handle_calibration_coverage`.
- Tool registration in `src/tools/mod.rs`.
- Unit tests for the coverage shape (against fixture cache stores).
- SOP update to `genre-classification.mdx` Prerequisites is *not* in this PR — release gated, deferred to PR 3.

Cost: ~1 day. No behaviour change for existing tools.

### PR 2 — `verify_track` / `verify_tracks` tools (depends on PR 1)

- Activate `verified_tracks` and add `verified_audit_log` tables.
- Implement `record_verification`, conflict resolution, audit logging in `verified_registry.rs`.
- Implement `handle_verify_track` and `handle_verify_tracks` in `verify_handlers.rs`.
- Wire registry into `handle_calibrate_audio_profiles` as union source.
- Wire `coverage_delta` into `verify_track` response (queries the same `coverage_summary` from PR 1).
- Add `verified_playlist_pending` buffer to `ServerState` (`src/tools/mod.rs:89–106`) and append to `write_xml` playlists at `staging_handlers.rs:303–376`.
- Integration tests covering atomicity, idempotency, conflict.
- Smoke test with tonight's four tracks per §9.

Cost: ~2 days.

### PR 3 — SOP updates (release-gated)

- Edit `site/src/partials/sops/genre-classification.mdx` per §5.
- Optional: edit `genre-audit.mdx` and `library-health.mdx`.
- Optional: add `verify` topic to `help_handler.rs` for `help(topic="verify")` standalone.
- Release per `CLAUDE.md` Releasing section: `./scripts/release.sh <version>` after PRs 1 and 2 are merged.

Cost: ~0.5 day.

Total: ~3.5 days plus release cycle.
