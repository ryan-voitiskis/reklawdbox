# Prototype Training Pipeline Improvements: Implementation Plan

**Date:** 2026-04-26
**Status:** Design proposal. No implementation yet. Validation gates the merge of D1.
**Scope:** Sections D1 and D2 from [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md).
**Related:**
- [genre-classification-implementation.md](genre-classification-implementation.md) — Fisher discriminant design rationale, dry-run results, `AFFINITY_CAP=0.5`.
- [genre-classification-improvements.md](genre-classification-improvements.md) — empirically invalidated features (`mod_centroid`, `harmonic_proportion`, `bpm_confidence`, `grid_stability`).
- Current implementation: `src/audio_profile.rs` (~1016 lines).

## Goal

Improve per-genre prototype quality on the existing 13 scalar features without adding new audio features. Two changes:

- **D1 — Hierarchical prototypes.** Build a per-family centroid (e.g. all Techno-family verified tracks pooled), then store each genre's prototype as a delta from its family centroid. Per-genre Fisher weights re-rank features by intra-family discriminative power rather than vs the global mean. The Elvism dry-run in `genre-classification-implementation.md` shows Dub Techno ranking 5th because its single strongest centroid signal collapses against the global "other" set; re-baselining vs the family resolves that.
- **D2 — Hard feature pruning per genre.** A feature near zero variance both within and between siblings is pure noise — but the existing `FISHER_WEIGHT_FLOOR = 0.02` at `src/audio_profile.rs:28` always keeps it active, so it leaks small contributions to every score. Hard-disable below threshold rather than just down-weight.

Neither change introduces new features; both are pure pipeline refinements operating on the existing 13 scalars + 3 timbral centroid vectors at `src/audio_profile.rs:32-46, 142-173`.

## 1. Model

### Two-tier prototype, per genre (D1)

```
For genre G in family F:
  family_centroid[F]        = mean over all verified tracks t where family(genre(t)) == F
  genre_centroid[G]         = mean over all verified tracks t where genre(t) == G
  genre_delta[G]            = genre_centroid[G] - family_centroid[F]
  intra_family_fisher_w[G]  = Fisher score per feature, genre G vs (family F minus G)
                              i.e. sibling-discriminative weights, not global one-vs-all
```

A track at classification time decomposes scoring into two stages:

```
family_distance = weighted L2 distance from track to family_centroid[F]
                  using a fixed family-level weight set (currently
                  proposal: equal weight, see §3 for rationale)
genre_distance  = weighted L2 distance from track to genre_centroid[G]
                  using intra_family_fisher_w[G]
genre_score(G)  = family_affinity[F] * intra_family_score[G]
```

Where `family_affinity[F] = max(0, 1 - family_distance/SCALE_F)` and the genre score caps at `AFFINITY_CAP = 0.5` from `src/audio_profile.rs:20`. Tracks far from any family centroid get small genre scores everywhere; tracks close to a family centroid get sharp differentiation among that family's members. Genres in `GenreFamily::Other` (`src/genre.rs:454-461`) — which is the default fallthrough family — keep the existing flat one-vs-all behaviour, since "Other" is not a coherent acoustic group.

### Hard feature pruning per genre (D2)

For each (genre G, feature f) pair, compute the inter-sibling separation:

```
sibling_mean[G,f]  = mean of f over family(G) verified tracks excluding G
sibling_std[G,f]   = stddev of f over family(G) verified tracks excluding G
separation[G,f]    = | mean_G[f] - sibling_mean[G,f] | / max(sibling_std[G,f], eps)
```

If `separation[G,f] < SEPARATION_THRESHOLD` (default `0.3`, units of σ), the feature is hard-disabled for G's prototype: stored with `fisher_weight = 0.0` and a new `active = false` flag. Disabled features contribute nothing to scoring (the existing `if s.fisher_weight > 0.0` guard at `src/audio_profile.rs:467` already handles this — we lean on it).

This is in addition to, not in place of, the existing top-N truncation at `src/audio_profile.rs:285, 325` (`max_features = N/5`).

## 2. Training Pipeline Changes

All changes localized to `src/audio_profile.rs` `calibrate()` (lines 240-420) and the `TrackSample` struct (lines 180-186).

### Family grouping pass — new (D1)

After the existing genre-grouping pass at `src/audio_profile.rs:271-274`, add a parallel family-grouping pass:

```
group tracks by family (via crate::genre::genre_family at src/genre.rs:902)
for each family F with n_F >= MIN_FAMILY_TRACKS (recommend 20):
  family_centroid[F]: per-feature mean ignoring None (mean_of, src/audio_profile.rs:197)
  family_stddev[F]:   per-feature pop stddev (stddev_of, src/audio_profile.rs:213)
families with n_F < 20 fall through to one-vs-all (current behaviour)
```

Recommend `MIN_FAMILY_TRACKS = 20` based on the verified-set distribution in `genre-classification-implementation.md`: Techno-family pool (Techno 50 + Deep Techno 20 + Dub Techno 17 + Minimal ~7 + Drone Techno + Ambient Techno) clears 20 trivially; House-family clears it heavily; Hardcore is borderline (~5-10) and should fall through; Bass-family and Downtempo similarly fall through.

### Per-genre Fisher weights against family rest — modified (D1)

Replace the global one-vs-all Fisher computation at `src/audio_profile.rs:288-321` with a sibling-discriminative version when a family centroid exists:

```
for each genre G with n_G >= MIN_TRACKS:
  F = family(G)
  if F has a centroid AND n_F - n_G >= MIN_TRACKS:
    sibling_pool = tracks in family F minus tracks in G
    for each feature f:
      sibling_mean[f], sibling_var[f] from sibling_pool
      genre_mean[f],   genre_var[f]   from G
      var_reg = alpha(n_G) * genre_var + (1-alpha) * sibling_var
                (alpha formula unchanged, src/audio_profile.rs:308)
      fisher = (genre_mean - sibling_mean)^2 / (var_reg + sibling_var)
      separation = |genre_mean - sibling_mean| / max(sqrt(sibling_var), eps)
      if separation < SEPARATION_THRESHOLD:
        active = false
        fisher = 0
      else:
        active = true
  else:
    fall through to existing global one-vs-all (lines 301-321)
```

Then proceed as today: sort by Fisher descending, truncate to `max_features = n_G / 5`, clamp/floor/normalize to sum to 1.0. Pruned features (active=false) are stored but contribute zero to both the truncation count and the weight sum.

### Genre delta storage — modified (D1)

The persisted prototype carries the absolute genre centroid (as today, for backwards-compat scoring) plus a derived delta computed at calibration time and stored alongside. The delta is what gets used for the within-family discriminator term; the absolute centroid is what gets used when no family centroid is available (e.g. `GenreFamily::Other` genres). Storing both is roughly 2x the centroid bytes per genre — negligible for ~40 genres × 13 features.

### Timbral centroids — unchanged (out of scope)

The MFCC and spectral-contrast centroids at `src/audio_profile.rs:348-398` stay exactly as today. Hierarchical timbral centroids would be a follow-on; the immediate accuracy wins are on the 13 scalar features.

## 3. Scoring Changes (mainly D1)

Changes localized to `score_track` at `src/audio_profile.rs:449-540`.

### Family affinity — new

Before iterating per-prototype, compute family affinities once per track:

```
for each family F with a stored centroid:
  family_dist = weighted L2 to family_centroid[F]
                (weights: equal across the 13 scalars, ignoring missing values)
  family_affinity[F] = max(0, 1 - family_dist / FAMILY_SCALE)
                       FAMILY_SCALE = SCALE = 2.5 (reuse src/audio_profile.rs:22)
```

Equal weights at the family level is the conservative choice: the family centroid is a coarse "is this Techno-shaped at all" check, not a fine discriminator. Weighting by global Fisher would re-introduce the global one-vs-all problem we're trying to avoid.

### Per-genre score — modified

Replace the scoring at `src/audio_profile.rs:457-524` for genres whose family has a centroid:

```
intra_family_z2 = 0
for each scalar feature with active=true and present in track:
  effective_std = max(stat.stddev, global_std * 0.1)  (unchanged)
  z = (track_value - genre_mean[f]) / effective_std
  intra_family_z2 += fisher_weight * z*z

intra_family_score = max(0, 1 - sqrt(intra_family_z2) / SCALE)

# timbral terms unchanged (lines 491-514)

genre_score = family_affinity[F] * intra_family_score

vote_weight = AFFINITY_CAP * genre_score   (capped 0..AFFINITY_CAP)
```

For genres in families without a centroid (`GenreFamily::Other` or families below `MIN_FAMILY_TRACKS`), keep the existing scoring path verbatim — no regression on genres outside the four well-populated families.

### Confidence penalty — keep

The small-genre penalty at `src/audio_profile.rs:519-521` (`0.5 * sqrt(n_active / total_n)`) stays. It applies independently of D1.

### Affinity threshold — keep

The `affinities.filter(|a| a.vote_weight >= 0.05)` at `src/audio_profile.rs:438` stays. Vote-injection threshold at `src/classify.rs:607` (`if a.vote_weight < 0.05 continue`) stays.

## 4. SQLite Schema Additions

Additive only. Existing tables at `src/store.rs:120-144` (`genre_audio_profiles`, `genre_timbral_centroids`, `genre_global_stats`) are unchanged.

```sql
CREATE TABLE IF NOT EXISTS genre_family_centroids (
    family        TEXT PRIMARY KEY,            -- 'Techno', 'House', 'Bass', 'Hardcore', 'Downtempo'
    feature_means TEXT NOT NULL,               -- JSON map: feature -> mean
    feature_stds  TEXT NOT NULL,               -- JSON map: feature -> stddev
    n_verified    INTEGER NOT NULL,
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);
```

Storing one row per family rather than one row per (family, feature) keeps the table tiny (5 rows max) and keeps the load path a single read. JSON-blob storage is consistent with the existing `genre_timbral_centroids.values_json` pattern at `src/store.rs:130-138`.

Additions to `genre_audio_profiles` — two new columns (nullable for backwards compat with rows from the old schema):

```sql
ALTER TABLE genre_audio_profiles ADD COLUMN active        INTEGER NOT NULL DEFAULT 1;
ALTER TABLE genre_audio_profiles ADD COLUMN delta_mean    REAL;     -- genre_mean - family_mean
```

`active` is the D2 hard-prune flag (1 = scored, 0 = stored but inactive — useful for telemetry/audit). `delta_mean` is the D1 delta carried explicitly so the load path doesn't have to recompute it.

**Family centroid storage choice:** A separate `genre_family_centroids` table (rather than overloading `genre_audio_profiles` with synthetic family rows) keeps the schemas of "per-genre" and "per-family" data orthogonal, avoids special-case `WHERE genre NOT LIKE 'family:%'` filters everywhere, and lets the family centroid carry its own `n_verified`, which differs from any single genre's count.

## 5. Migration / Calibration Schema Version

Bump `STORE_SCHEMA_VERSION` at `src/store.rs:11` from 7 to 8. The migration adds the new table + new columns idempotently. No data rewrite — existing `genre_audio_profiles` rows are valid (`active` defaults to 1, `delta_mean` is NULL until next calibration).

A `calibration_schema_version` constant is added to `src/audio_profile.rs` (recommend `const CALIBRATION_SCHEMA_VERSION: u32 = 2`). Stored as a single row in a new `genre_calibration_meta(key TEXT PRIMARY KEY, value TEXT)` table. At startup, if the persisted version is below the current constant, log a warning and treat the registry as missing (so vote injection at `src/classify.rs:603-620` falls through cleanly until the user runs `calibrate_audio_profiles`).

The persisted version is also returned in the `calibrate_audio_profiles` MCP tool response (`src/tools/classify_handler.rs:725-862`) so the user knows which model version generated the prototypes.

## 6. Validation

The same harness used for the existing dry-run in `genre-classification-implementation.md` re-runs against the 574-track verified playlist (`genre_verified` per `src/tools/classify_handler.rs:729`). Acceptance gates:

1. **Per-genre prototype quality.** For each genre with `n >= 10`, compute mean within-genre distance and mean nearest-sibling distance under both old and new prototypes. The new prototypes must show:
   - within-genre distance ≤ old (no regression on tightness)
   - between-sibling distance > old by ≥10% on at least 60% of genres (the core D1 win)
2. **Elvism case.** The Burger/Ink "Elvism" track (case study from `genre-classification-improvements.md#2-elvism`) currently ranks Dub Techno 5th at vote weight 0.219. Acceptance: Dub Techno ranks ≥ 3rd with vote weight ≥ 0.30 under new prototypes. (Top-1 is not required; Fisher-only signals are not expected to overrule enrichment alone.)
3. **No regression on already-strong genres.** House, Hip Hop, Drum & Bass top-1 confusion rate (track classified as itself when scored against all prototypes) must stay within 5 percentage points of the current baseline. House currently dominates at Fisher weights `centroid 0.46 / dance 0.24 / bpm 0.23` per the implementation doc — sibling-rest weights should preserve this.
4. **Pruning telemetry.** New per-genre `n_active_features` reported by the calibrate tool. Expected ranges:
   - Most genres retain 70-90% of features post-prune.
   - Drum & Bass (`168-180 BPM`, distinctive): retains ~all features.
   - Hardcore (`160+`): retains ~all.
   - Atonal genres (Drone Techno, Ambient): prune `key_clarity` (and possibly `dissonance_mean`).
5. **Pruning false-prune check.** No genre with `n >= 10` should drop below `n / 5` active features (the `max_features` floor). If pruning + truncation would push below that, raise `SEPARATION_THRESHOLD` for that genre adaptively (or accept the smaller feature set; the truncation rule is already conservative).

The dry-run output (committed alongside the PR) is a Markdown report listing per-genre old/new within-genre distance, between-sibling distance, n_active_features, top-3 features by intra-family Fisher weight.

## 7. Risks

1. **Family centroid dominated by a single high-N genre.** If Techno-family has 50 Techno + 20 Deep Techno + 17 Dub Techno + smaller siblings, the family centroid is ~50% Techno-proper by mass. Deep Techno's delta is then computed against a Techno-leaning origin, which may *reinforce* the very confusion D1 aims to fix. **Mitigation:** compute the family centroid as the **per-genre mean of genre means** (i.e. equal weight per sibling), not a flat per-track mean. Smaller siblings get equal voice. Document this explicitly in the calibration code.

2. **Pruning over-aggressive on the bubble.** A feature with `separation = 0.31` (just above threshold) survives; a feature at `0.29` is killed. If two genres share most features near the threshold, results swing on noise. **Mitigation:** apply pruning *after* truncation, not before — truncation already keeps only the top-N most discriminative features per genre, so any survivor is by definition above the truncation bar; pruning then removes only the tail. Also: log the per-genre pruning decisions in the calibrate tool output so the user can see what was killed and at what separation.

3. **Numerical stability of delta storage.** Storing `genre_mean` as `family_mean + delta` and recomputing absolute distances at scoring time accrues two rounding errors instead of one. **Mitigation:** store both `genre_mean` (as today) and `delta_mean` explicitly, and use `genre_mean` for distance computation. The delta is metadata for evidence/diagnostics only. This costs ~13 floats per genre — trivial.

4. **`MIN_FAMILY_TRACKS = 20` cuts Hardcore and Bass.** Both fall through to the existing one-vs-all path. That's a non-regression by construction (their behaviour is unchanged), but it means the win is concentrated in Techno-family and House-family. Acceptable for v1; revisit `MIN_FAMILY_TRACKS` after a few rounds of verified-set growth.

5. **Sibling-rest can be tiny.** For Dub Techno (n=17) in Techno-family (n≈97), the sibling rest is 80 — fine. For Disco (n=11) in House-family (n≈300), sibling rest is ~289 — also fine. But for Trance (n~6-12) in Techno-family, the rest still works. The `MIN_TRACKS = 5` floor at `src/audio_profile.rs:24` already handles the bottom end.

6. **`GenreFamily::Other` collisions.** Any genre not in the four well-populated families uses the existing global Fisher path. This is a no-op for those genres — the new code is entirely opt-in based on `MIN_FAMILY_TRACKS` clearance.

## 8. PR Breakdown

### PR 1 — D2 hard feature pruning

Small, additive, well-isolated. Lives entirely in `src/audio_profile.rs:288-346` plus the `active` column migration in `src/store.rs:120-128` and a bump of `STORE_SCHEMA_VERSION` to 8.

- Add `active: bool` to `FeatureStat` at `src/audio_profile.rs:53-59`. Default true on existing-row load.
- Add `SEPARATION_THRESHOLD: f64 = 0.3` constant.
- In `calibrate()`, after the per-feature stats are computed but before sorting (line 322), compute `separation` against the *global* mean (not family-rest yet — that lands in PR 2) and set `active = false` if below threshold.
- Persist `active` to SQLite. Load path filters on `active = true`. The existing scoring guard `if s.fisher_weight > 0.0` (line 467) needs no change since pruned features get fisher_weight 0.
- Add per-genre `n_active_features` to the calibrate tool response at `src/tools/classify_handler.rs:830-840`.
- Tests: existing tests keep passing (House and Ambient prototypes have plenty of separation per the verified-feature ranges in `genre-classification-implementation.md` Empirical Reference Data §). Add a test that constructs a genre with one feature near zero and confirms it's pruned.

Effort: 0.5-1 day. No scoring changes. No risk to already-good genres.

### PR 2 — D1 hierarchical prototypes

Schema additions, calibration changes, scoring changes. Larger.

- Add `genre_family_centroids` table + `delta_mean` column + `genre_calibration_meta` table to `src/store.rs:120-144`. Bump `STORE_SCHEMA_VERSION` from 8 to 9 (chained on PR 1).
- Add `MIN_FAMILY_TRACKS: u32 = 20`, `CALIBRATION_SCHEMA_VERSION: u32 = 2` to `src/audio_profile.rs`.
- Add `FamilyCentroid` struct holding `feature_means: HashMap<&str, f64>`, `feature_stds: HashMap<&str, f64>`, `n_verified: u32`.
- Extend `ProfileRegistry` at `src/audio_profile.rs:81-85` with `family_centroids: HashMap<GenreFamily, FamilyCentroid>`.
- In `calibrate()`: compute per-genre means first, then per-family centroid as **mean of per-genre means** (per Risk 1 mitigation). Compute sibling-pool Fisher weights for genres whose family clears `MIN_FAMILY_TRACKS`. Switch the D2 separation check from global to sibling-rest.
- In `score_track()`: compute family affinities once per call; multiply through into per-genre scores.
- Add family centroid persistence to `save_to_db` at `src/audio_profile.rs:547-629` and load to `load_from_db` at `:632-778`.
- Update calibrate tool response with family-level summary (n_F, top family features, member genres + their delta_mean magnitudes).
- Validation: re-run dry-run harness, commit Markdown report, gate merge on §6 acceptance criteria.

Effort: 2-3 days, plus 0.5-1 day for the validation harness re-run and report.

PR 2 depends on PR 1 (the `active` flag is the cleanest place to store the sibling-rest separation result). PR 2 also requires a one-time `calibrate_audio_profiles` re-run after merge — documented in the PR description; the existing prototypes remain functional in the meantime since the load path tolerates missing family centroids.

## 9. Cost Estimate

| Stage | Effort |
|---|---|
| PR 1 (D2 pruning + schema + telemetry) | 0.5-1 day |
| PR 2 (D1 schema + calibration + scoring) | 2-3 days |
| PR 2 validation harness re-run + report | 0.5-1 day |
| Buffer for dry-run iteration (threshold tuning, family-centroid weighting) | 0.5-1 day |
| **Total** | 3.5-6 days |

Budget 5-6 days realistically. Most of the risk is in §7.1 (family-centroid weighting) and §7.2 (pruning threshold); both are tunable post-merge without further schema changes.
