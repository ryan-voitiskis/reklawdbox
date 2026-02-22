# SOP Test Session Log — 2026-02-20/22

End-to-end test of `docs/operations/sops/genre-classification.md` against a live Rekordbox library (2460 tracks, 468 ungenred).

## Steps Completed

### Step 1: Cache Coverage + Hydration

Cache was near-empty. Hydrated via audio analysis (Stratum-DSP) and enrichment (Discogs + Beatport).

**Final coverage (ungenred tracks):**
- Stratum-DSP: 422/468 (90.2%)
- Discogs: 397/468 (84.8%)
- Beatport: 398/468 (85.0%)
- Essentia: not installed

**Issues found:**
1. `search_tracks` caps at 200 results with no offset/pagination — workaround: partition by date range, collect IDs, pass explicit `track_ids` in batches of 50.
2. `max_tracks` defaults to 20 even when `track_ids` is provided — must set explicitly.
3. `skip_cached` doesn't filter search results, only prevents re-processing — confusing when combined with the 200-result cap.

Detailed step notes were consolidated into this report.

### Step 2: Taxonomy Review

Ran `suggest_normalizations`. Reviewed 199 alias tracks, 13 unknown-genre tracks.

**Taxonomy bugs identified (wrong aliases in `genre.rs`):**
- Dub Reggae → Dub (should be canonical)
- Drone Techno → Deep Techno (should be canonical)
- Gospel House → House (should be canonical)
- Progressive House → House (should be canonical)
- Highlife → Afro House (should be canonical)
- Jazz missing from taxonomy entirely (should be canonical)

**SOP feedback:** Taxonomy should be refined through conversation with user *before* presenting mapping recommendations, not after.

Detailed step notes were consolidated into this report.

### Step 3: Classification (partial)

First batch of 30 ungenred tracks returned 0 high, 1 medium, 5 low, 24 insufficient. The batch was dominated by System 7 and Kangding Ray tracks with empty artist fields (artist name embedded in title like "01 System 7 - Track Title"), causing enrichment lookups to fail across both providers.

**SOP gap:** No handling for tracks with missing/malformed artist metadata.

### Set Builder Test

Built a 12-track Deep Techno set from the 38 tracks tagged Deep Techno in the library. Used `build_set` with `warmup_build_peak_release` energy curve and `harmonic` priority.

**Set C (highest scored, 8.64/10):**
1. Donato Dozzy, Nuel — Aqua 7 (116, 4A) [warmup]
2. Toki Fuko — Astatine (128, 4A) [build]
3. irini — dreamuniverse pt.ii (128, 4A) [build]
4. Toki Fuko — Polonium (127, 4A) [build]
5. traumprinz — i found truth... (125, 5A) [build]
6. prince of denmark — latenightjam (125, 5A) [peak]
7. prime minister of doom — grand finale (124, 5A) [peak]
8. nthng — Spirit of Ecstasy (127, 5A) [peak]
9. prime minister of doom — the way (123, 6A) [release]
10. Toki Fuko — Bismuth (127, 6A) [release]
11. Donato Dozzy, Nuel — Aqua 3 (120, 6A) [release]
12. prime minister of doom — truth inside (124, 7A) [release]

**Harmonic flow:** 4A → 5A → 6A → 7A ascending walk. Played with master tempo, long blends sounded good throughout.

**DJ feedback after playing:**
- Track 10 (Bismuth, 127 BPM) too energetic for release phase.
- Track 11 (Aqua 3, 120 BPM) nice but still slightly too energetic for release.
- Release section needs tracks closer to 116-122 BPM with more spacious character.
- Without Essentia, energy scores are BPM-derived only (tight 0.40-0.48 cluster), limiting the algorithm's ability to differentiate energy within the same BPM range.

Exported as playlist: `rekordbox-exports/reklawdbox-20260221-213905.xml`

## Summary of Tool Issues

| Issue | Severity | Status |
|-------|----------|--------|
| Search pagination caps at 200 | Blocking | Workaround documented |
| `max_tracks` default with `track_ids` | Minor | Always set explicitly |
| `skip_cached` semantics confusing | Minor | Document better |
| No Essentia = weak energy scoring | Moderate | Install Essentia |
| Empty artist fields break enrichment | Moderate | SOP needs handling |
| 6 wrong alias mappings in `genre.rs` | Moderate | Fix in code |
| SOP should refine taxonomy first | Process | Update SOP |

## Post-Session Status (2026-02-22)

- `search_tracks` now supports `offset` pagination.
- `analyze_audio_batch` and `enrich_tracks` now default `max_tracks` to `track_ids.len()` when `track_ids` is provided.
- Taxonomy updates from this session are reflected in `src/genre.rs` (including `Dub Reggae`, `Drone Techno`, `Gospel House`, `Progressive House`, `Highlife`, and `Jazz`).
