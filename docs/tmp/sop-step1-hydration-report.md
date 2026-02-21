# SOP Step 1 Hydration Report

Test run of `docs/genre-classification-sop.md` Steps 1 and 1b against a live Rekordbox library (2460 tracks, 468 ungenred).

## Results

### Final Cache Coverage (ungenred tracks)

| Provider    | Cached | Total | Coverage |
|-------------|--------|-------|----------|
| Stratum-DSP | 422    | 468   | 90.2%    |
| Essentia    | 0      | 468   | 0.0%     |
| Discogs     | 397    | 468   | 84.8%    |
| Beatport    | 398    | 468   | 85.0%    |

- No audio analysis: 46 tracks
- No enrichment: 70 tracks
- No data at all: 28 tracks
- Discogs failures: 1 (HTTP 502)

All providers meet the SOP thresholds (stratum > 90%, discogs > 70%).

## Issues Found

### 1. Search pagination caps at 200 results (blocking)

`search_tracks` has a hard max of 200 results with no offset/pagination parameter. `analyze_audio_batch` and `enrich_tracks` use the same search internally, so filter-based queries (e.g. `has_genre=false`) always return the same first 200 tracks.

**Impact:** Cannot reach tracks beyond the first 200 using filters alone. After the first batch, `skip_cached=true` doesn't help because the search still returns the same 200 (now cached) tracks — it never sees the remaining 268.

**Workaround used:** Partitioned search by date ranges (`added_before`, `added_after`) across 3 overlapping queries to collect 444/468 unique track IDs, then passed explicit `track_ids` arrays to analysis/enrichment tools in batches of 50.

**24 tracks were lost** to date boundary gaps (likely `added_before`/`added_after` are exclusive, so tracks added exactly on boundary dates were excluded from all partitions). Not investigated further.

**Recommendation:** Add an `offset` parameter to `search_tracks`, or make batch tools internally paginate past the 200-result search limit. Alternatively, `skip_cached` could filter at the query level (exclude tracks that already have cached results) rather than just skipping the analysis/enrichment step.

### 2. `max_tracks` default on `analyze_audio_batch` with `track_ids`

When passing explicit `track_ids` to `analyze_audio_batch`, the `max_tracks` parameter still defaults to 20. Passing 200 track IDs without setting `max_tracks=200` only processes the first 20.

**Impact:** Minor — just requires the caller to always set `max_tracks` explicitly. But unintuitive when you've already specified exactly which tracks you want.

**Recommendation:** When `track_ids` is provided, default `max_tracks` to `track_ids.len()` instead of 20.

### 3. `skip_cached` semantics are confusing

`skip_cached=true` means "don't re-analyze/re-enrich already-cached tracks." It does NOT mean "skip past cached tracks in the search to find uncached ones." The search still returns cached tracks — they just get a `cached` status instead of being analyzed.

**Impact:** The agent expected `skip_cached` to act as a filter, but it only affects whether work is redone. Combined with the 200-result search cap, this means the agent gets stuck in a loop returning the same cached tracks.

**Recommendation:** Either rename to `force_refresh=false` (inverted default) to clarify intent, or add a separate `exclude_cached` search filter. The current `skip_cached` name implies the tracks are skipped entirely.

### 4. Essentia not installed

Essentia coverage is 0% because it's not installed. The SOP's audio-only fallback (Step C) relies on Essentia features like `danceability` and `rhythm_regularity` for genre family inference. Without Essentia, audio-only classification will be limited to BPM-only heuristics, which the SOP marks as always **insufficient** confidence.

**Impact:** The ~70 tracks with no enrichment data will have weaker classification. Essentia would improve confidence for those.

**Not a bug** — just a deployment note.

## SOP Feedback

### What worked well

- Cache coverage tool (`cache_coverage`) provides clear go/no-go signal for classification readiness.
- The SOP's threshold table (Step 1) gave a clear recommendation: "all providers < 50% → recommend hydration."
- Enrichment tools handled rate limiting gracefully — no cascading failures.
- The batch-then-check-coverage loop is sound as a workflow.

### What the SOP should address

1. **Pagination strategy needs to be documented.** The SOP says "repeat with offset if collection is larger than `max_tracks`" but there is no offset mechanism. The SOP should document the track_ids workaround (partition search → collect IDs → batch by IDs).

2. **Batch size guidance.** The SOP suggests `max_tracks=200` for audio analysis and `max_tracks=50` for enrichment but doesn't mention the 200-result search cap or that `track_ids` batches also need explicit `max_tracks`.

3. **Time estimate.** Audio analysis of 444 tracks took ~15 minutes (CPU-bound DSP). Enrichment of 444 tracks across 2 providers took ~10 minutes (network-bound). The SOP could set expectations for the user.
