# Genre Classification — Agent SOP

Standard operating procedure for evidence-based genre tagging in Rekordbox collections. Agents must follow this document step-by-step.

## Overview

This SOP uses existing MCP tools to classify genres across a Rekordbox collection. The agent's role is to gather cached evidence, apply a concrete decision tree, present recommendations in a consistent format, and stage only human-approved changes.

`cache_coverage` is implemented and should be used at the start of every classification session.

## Constraints

- **Taxonomy is compiled.** The alias map in `genre.rs` cannot be changed at runtime. If the user disagrees with a mapping, the agent works around it by using the user's preferred genre directly — not the alias target.
- **No auto-tagging.** Every genre change requires explicit human approval.
- **Cache-first.** Classification must run from cached data. If cache is empty for a track, that track gets flagged as insufficient evidence — the agent does not trigger enrichment mid-classification.
- **XML export only.** No direct DB writes. All changes flow through `update_tracks` → `preview_changes` → `write_xml`.

---

## Step 1: Check Cache Coverage

**Goal:** Determine whether the collection has enough cached data to start classification.

### Tool call

```
cache_coverage()                          # whole collection
cache_coverage(has_genre=false)           # ungenred tracks only
cache_coverage(playlist_id="...")         # specific playlist
```

### Evaluate result

<!-- dprint-ignore -->
| Condition | Action |
|-----------|--------|
| stratum_dsp > 90% and discogs > 70% | Proceed to Step 2. |
| Any provider < 50% | Recommend hydration (Step 1b) before classification. |
| User declines hydration | Proceed, but prepend all recommendations with: "Based on partial evidence." |

### Present to user

```
Cache coverage for [scope]:
  Stratum-DSP:  420/468 (89.7%)
  Essentia:     380/468 (81.2%)
  Discogs:      350/468 (74.8%)
  Beatport:     290/468 (62.0%)

  No audio analysis:  48 tracks
  No enrichment:     118 tracks
  No data at all:     30 tracks

Recommendation: Run hydration for the 48 tracks missing audio
and 118 tracks missing enrichment before classification.
Proceed with hydration? [or skip and classify on partial data]
```

---

## Step 1b: Hydrate Cache (if needed)

**Goal:** Fill cache gaps. Run audio analysis first (local, fast), then enrichment (external, rate-limited).

### Tool calls (in order)

```
analyze_audio_batch(has_genre=false, skip_cached=true, max_tracks=200)
```

Repeat with offset if collection is larger than `max_tracks`. Wait for completion before enrichment.

```
enrich_tracks(has_genre=false, skip_cached=true, providers=["discogs"], max_tracks=50)
enrich_tracks(has_genre=false, skip_cached=true, providers=["beatport"], max_tracks=50)
```

Repeat enrichment calls until coverage targets are met or all tracks have been attempted. Between Discogs batches, expect rate limiting — report progress to user.

### Present to user

After each batch:

```
Hydration progress:
  Audio analysis:  468/468 complete
  Discogs:         280/468 (45 not found, 143 remaining)
  Beatport:        190/468 (78 not found, 200 remaining)

Continue enrichment? [yes / skip to classification]
```

"Not found" means the provider was queried but returned no match. These tracks will rely on other evidence sources.

---

## Step 2: Review Taxonomy

**Goal:** Surface alias mappings the user should validate before classification uses them.

### Tool calls

```
get_genre_taxonomy
suggest_normalizations
```

### Process `suggest_normalizations` result

The tool returns three sections: `alias` (known mappings), `unknown` (unmapped genres), `canonical` (already correct).

**For the `alias` section:** Present all aliases that will be applied during classification. Highlight debatable ones.

### Present to user

```
These genre aliases will be used during classification.
Tracks with these genres will be compared against their canonical form:

  ALIAS                              → CANONICAL        TRACKS
  ─────────────────────────────────────────────────────────────
  Electronica                        → Techno              19   ⚠ debatable
  Bass                               → UK Bass             16   ⚠ debatable
  Drone Techno                       → Deep Techno          8   ⚠ debatable
  Gospel House                       → House                3   ⚠ debatable
  Progressive House                  → House                2   ⚠ debatable
  Hip-Hop                            → Hip Hop             29
  DnB                                → Drum & Bass         16
  Techno (Peak Time / Driving)       → Techno              18
  Techno (Raw / Deep / Hypnotic)     → Deep Techno         14
  [... remaining aliases ...]

⚠ = mapping may lose useful specificity.

For debatable aliases:
  Accept — classification will use the canonical form.
  Override — tell me what genre to use instead.
  Skip — leave these tracks out of batch classification.
```

### Collect user decisions

For each debatable alias the user overrides, record the override. Example:

- User says "Electronica should stay as Electronica, not Techno"
- Agent records: when classifying, if a track's current genre is "Electronica", do not suggest changing it to "Techno". If external evidence says "Techno", present it as a conflict for review.

Store overrides in working memory for the duration of the session. These don't change the compiled taxonomy.

**For the `unknown` section:** Present unmapped genres and ask user for each:

```
These genres in your library are not in the taxonomy:

  GENRE            TRACKS
  ───────────────────────
  Anti-music            1
  Ballad                1
  Jazz                  1

Options per genre:
  Map to — assign a canonical genre for this session.
  Leave — skip these tracks in batch classification.
```

---

## Step 3: Classify Ungenred Tracks

**Goal:** Suggest genres for tracks with no current genre, in reviewable batches.

### Tool call

```
resolve_tracks_data(has_genre=false, max_tracks=30)
```

### For each track, apply the Decision Tree (below)

Produce a classification for every track in the batch. Then group and present.

### Present to user

Group tracks by confidence level. Use this table format:

```
BATCH 1 — 30 tracks (12 high, 10 medium, 5 low, 3 insufficient)

HIGH CONFIDENCE (12 tracks)
  #  ARTIST              TITLE                    SUGGESTED    EVIDENCE
  ──────────────────────────────────────────────────────────────────────
  1  Artist Name         Track Title              Deep House   Discogs ✓ Beatport ✓ Audio ✓
  2  Artist Name         Track Title              Techno       Discogs ✓ Beatport ✓ Audio ✓
  ...

MEDIUM CONFIDENCE (10 tracks)
  #  ARTIST              TITLE                    SUGGESTED    EVIDENCE         NOTES
  ──────────────────────────────────────────────────────────────────────────────────────
  13 Artist Name         Track Title              House        Discogs ✓        No Beatport match
  14 Artist Name         Track Title              Breakbeat    Beatport ✓       Discogs styles split
  ...

LOW CONFIDENCE (5 tracks)
  #  ARTIST              TITLE                    SUGGESTED    EVIDENCE         NOTES
  ──────────────────────────────────────────────────────────────────────────────────────
  23 Artist Name         Track Title              Techno       Audio only       BPM 132, high regularity
  ...

INSUFFICIENT EVIDENCE (3 tracks)
  #  ARTIST              TITLE                    NOTES
  ──────────────────────────────────────────────────────
  28 Artist Name         Track Title              No enrichment, no audio
  ...

Actions:
  approve all        — stage all suggestions as-is
  approve high       — stage only high confidence
  reject #5          — remove track 5 from staging
  change #14 House   — override suggestion for track 14
  skip               — move to next batch without staging
  details #3         — show full evidence for track 3
```

### Handle "details" request

When user asks for details on a specific track, show:

```
Track: Artist Name — Track Title

  Rekordbox:   genre=(none)  BPM=128.00  key=6A  rating=4★
  Stratum:     BPM=127.8 (✓ agrees)  key=Fm (✓ agrees)  confidence=0.92
  Essentia:    danceability=2.10  rhythm_regularity=0.88  loudness=-14.5 LUFS  centroid=1850 Hz
  Discogs:     styles=[Deep House, Tech House]  →  Deep House (exact), Tech House (exact)
  Beatport:    genre=Deep House (exact)
  Taxonomy:    Discogs converges with Beatport on "Deep House"

  Suggestion:  Deep House (high confidence)
  Rationale:   Discogs and Beatport both map to Deep House.
               Audio profile consistent (high danceability, regular rhythm, 128 BPM).
```

### Stage approved changes

```
update_tracks(changes=[
  {"track_id": "...", "genre": "Deep House"},
  {"track_id": "...", "genre": "Techno"},
  ...
])
```

### Repeat

Call `resolve_tracks_data` with the next batch (offset by already-processed tracks or use a filter that excludes already-staged tracks). Continue until all ungenred tracks are processed.

---

## Step 4: Review Existing Genre Conflicts (Optional)

**Goal:** Find tracks where the current genre disagrees with external evidence.

Only run this step if the user requests it after ungenred tracks are done.

### Tool call

```
resolve_tracks_data(has_genre=true, max_tracks=50)
```

### Filter for conflicts

A track has a conflict when ANY of:

- `genre_taxonomy.current_genre_canonical` is null (genre exists but isn't in taxonomy and isn't an alias).
- Discogs styles map to a different canonical genre than the current one.
- Beatport genre maps to a different canonical genre than the current one.
- `audio_analysis.key_agreement` is false (key mismatch — not genre, but flag it).

Skip tracks where current genre matches all evidence.

### Present to user

```
GENRE CONFLICTS — 15 tracks with evidence disagreement

  #  ARTIST              TITLE                    CURRENT      EVIDENCE SAYS    CONFLICT
  ──────────────────────────────────────────────────────────────────────────────────────────
  1  Artist Name         Track Title              Techno       Deep Techno      Discogs+Beatport disagree
  2  Artist Name         Track Title              House        Deep House       Beatport disagrees
  3  Artist Name         Track Title              Electronica  (not in taxonomy) Unknown genre

KEY MISMATCHES — 8 tracks where analysis disagrees with Rekordbox key
  #  ARTIST              TITLE                    RB KEY   STRATUM KEY   PRIORITY
  ────────────────────────────────────────────────────────────────────────────────
  1  Artist Name         Track Title              6A       8A            Medium (single source)
  ...

Actions: same as Step 3 (approve/reject/change/details/skip)
```

---

## Step 5: Preview and Export

**Goal:** Verify all staged changes and export XML.

### Tool calls

```
preview_changes
```

### Present to user

```
STAGED CHANGES — 142 tracks

  TRACK                              CURRENT GENRE    NEW GENRE
  ────────────────────────────────────────────────────────────────
  Artist - Title                     (none)           Deep House
  Artist - Title                     (none)           Techno
  Artist - Title                     Electronica      Deep Techno
  ...

Export to XML? [yes / review individual changes / clear all]
```

### Export

```
write_xml
```

Report the output file path. Remind user to import into Rekordbox.

---

## Decision Tree

Apply this for every track during classification. This is the concrete logic, not a guideline.

### Inputs (from `resolve_tracks_data` per track)

- `current_genre`: `rekordbox.genre` (may be empty)
- `current_canonical`: `genre_taxonomy.current_genre_canonical` (may be null)
- `discogs_mappings`: `genre_taxonomy.discogs_style_mappings[]` — each has `maps_to` and `mapping_type`
- `beatport_mapping`: `genre_taxonomy.beatport_genre_mapping` — has `maps_to` and `mapping_type`
- `data_completeness`: boolean flags per source
- `audio_analysis`: stratum + essentia features (may be null)
- Session overrides from Step 2 (user taxonomy corrections)

### Step A: Gather mapped genres

1. From `discogs_mappings`, collect all entries where `mapping_type` is "exact" or "alias". Extract their `maps_to` values. Deduplicate. Call this `discogs_genres` (a set).
2. From `beatport_mapping`, if `mapping_type` is "exact" or "alias", take `maps_to`. Call this `beatport_genre` (a single value or null).
3. If `current_genre` is non-empty, use `current_canonical` if available, else use `current_genre` as-is. Call this `current`.

### Step B: Find consensus

4. **Full consensus:** `beatport_genre` is non-null AND `beatport_genre` is in `discogs_genres`.
   → `suggested_genre` = that genre. Confidence = **high**.

5. **Partial consensus (Discogs only):** `beatport_genre` is null. `discogs_genres` has exactly one entry.
   → `suggested_genre` = that genre. Confidence = **medium**.

6. **Partial consensus (Beatport only):** `discogs_genres` is empty. `beatport_genre` is non-null.
   → `suggested_genre` = `beatport_genre`. Confidence = **medium**.

7. **Split Discogs + Beatport agrees with one:** `discogs_genres` has multiple entries. `beatport_genre` is in `discogs_genres`.
   → `suggested_genre` = `beatport_genre` (Beatport breaks the tie). Confidence = **medium**.

8. **Split Discogs, no Beatport:** `discogs_genres` has multiple entries. `beatport_genre` is null.
   → `suggested_genre` = pick the Discogs genre that appears most frequently in the styles list (count how many raw styles mapped to each canonical). Confidence = **low**. Note the split in rationale.

9. **Discogs and Beatport disagree:** `beatport_genre` is non-null, NOT in `discogs_genres`, and `discogs_genres` is non-empty.
   → `suggested_genre` = null. Confidence = **insufficient**. Present both options to user. Do not pick one.

9b. **Discogs/Beatport disagree + audio features available:** If Step 9 is true and Essentia features are present, compare both candidates against audio expectations and pick the better fit.
Use these discriminators:

- `spectral_centroid_mean`: lower favors darker genres, higher favors brighter genres.
- `rhythm_regularity`: higher favors straighter grooves, lower favors broken/syncopated grooves.
- `dynamic_complexity`: higher favors more dynamic genres.
- BPM plausibility versus each genre's typical range.
  Common tie-breaks:
  - Deep House vs Tech House: Deep House tends darker/sparser; Tech House tends brighter/busier.
  - Techno vs Trance: Trance usually sits higher BPM and often higher danceability.
  - House vs Garage: Garage tends lower rhythm regularity and higher dynamic complexity.
  - Techno vs Electro: Electro often has lower regularity and sparser onset patterns.
    If audio clearly favors one option, set `suggested_genre` to that option with **low** confidence and note "audio-assisted tie-break."
    If audio remains ambiguous, keep **insufficient** and present both options.

10. **No enrichment data at all:** Both `discogs_genres` and `beatport_genre` are empty/null.
    → Fall through to Step C (audio-only).

### Step C: Audio-only fallback

Only reached when no enrichment data maps to a canonical genre.

Use BPM + Essentia features to suggest a likely family. Danceability uses Essentia's native ~0-3 scale.

<!-- dprint-ignore -->
| BPM | Rhythm Reg | Danceability | Spectral Centroid | Dynamic Comp | Likely genre |
|-----------|-------------------|--------------|-------------------|--------------|---------------|
| 160-180 | any | > 1.5 | any | any | Drum & Bass |
| 135-150 | > 0.9 | > 2.0 | Mid-High | Mid-High | Trance |
| 128-145 | > 0.9 | > 1.8 | Mid | High | Techno (peak/driving) |
| 128-140 | > 0.85 | > 1.2 | Very Low | Low-Mid | Deep Techno / Dub Techno |
| 125-140 | > 0.9 | > 1.0 | Low-Mid | Low | Minimal |
| 125-135 | > 0.85 | > 1.5 | Low | Low-Mid | Deep House |
| 125-135 | > 0.85 | > 1.5 | Mid-High | Mid | Tech House |
| 118-130 | > 0.8 | > 1.5 | Mid | Mid | House |
| 120-135 | 0.6-0.8 | > 1.0 | Mid | Mid-High | Garage / UK Garage |
| 120-140 | < 0.7 | > 1.0 | any | Mid | Breakbeat |
| 80-115 | any | > 1.0 | any | any | Hip Hop or Downtempo |
| 60-110 | < 0.5 | < 0.8 | Low | Low | Ambient |
| 128-145 | > 0.85 | > 1.0 | Mid-High | High | Hard Techno |

Centroid guidelines for table matching:

- Very Low: `< 600 Hz`
- Low: `600-1200 Hz`
- Mid: `1200-2500 Hz`
- Mid-High: `2500-4000 Hz`
- High: `> 4000 Hz`

If a track matches one row: confidence = **low**, note "audio-only inference."
If a track matches multiple rows or none: confidence = **insufficient**.
If Essentia is not available, use BPM alone — always **insufficient** unless BPM is strongly diagnostic (e.g., 170 = almost certainly DnB).

### Step D: Compare to current genre

11. If `current` is non-empty and matches `suggested_genre` → decision = **confirm**. No change needed.
12. If `current` is non-empty and differs from `suggested_genre` → decision = **conflict**. Present both.
13. If `current` is empty → decision = **suggest**. Propose `suggested_genre`.
14. If `suggested_genre` is null (insufficient evidence) → decision = **manual**. Flag for user.

### Step E: Apply session overrides

15. Check if the user overrode any alias mapping in Step 2 that affects this track's current genre or suggested genre. If so, use the user's preference instead of the taxonomy mapping.

---

## Key Mismatch Detection

Run as a side-check during Steps 3-4. Not a genre decision — just useful data surfaced alongside genre results.

### Logic

From `audio_analysis` for each track:

- `rb_key` = `rekordbox.key`
- `stratum_key` = `audio_analysis.stratum_dsp.key` (if present)
- `key_agreement` = `audio_analysis.key_agreement`

<!-- dprint-ignore -->
| Condition | Priority | Note |
|-----------|----------|------|
| `key_agreement` = false and Essentia rhythmic features strongly consistent with stratum key | High | Both analyzers suggest Rekordbox is wrong. |
| `key_agreement` = false, no Essentia confirmation | Medium | Single-source disagreement. |
| No stratum data | — | Cannot assess. Skip. |

Present key mismatches in a separate section of the conflict report (Step 4), not mixed into genre suggestions.

---

## Tool Contract: `cache_coverage`

Implementation reference: `src/tools/mod.rs` (tool registration), `src/tools/resolve_handlers.rs` (handler).

### Purpose

Report cache completeness for a track scope. Lets agent and user decide if hydration is needed.

### Behavior

- Counts tracks matching filters.
- For each provider, counts how many have cached data (check `data_completeness` flags or equivalent store query).
- No external calls.

### Request

Same filter params as `search_tracks` (all optional):

```
track_ids, playlist_id, query, artist, genre, has_genre,
bpm_min, bpm_max, key, rating_min, label, path,
added_after, added_before
```

### Response

```json
{
  "scope": {
    "total_tracks": 2460,
    "filter_description": "has_genre = false",
    "matched_tracks": 468
  },
  "coverage": {
    "stratum_dsp": { "cached": 420, "percent": 89.7 },
    "essentia": { "cached": 380, "percent": 81.2, "installed": true },
    "discogs": { "cached": 350, "percent": 74.8 },
    "beatport": { "cached": 290, "percent": 62.0 }
  },
  "gaps": {
    "no_audio_analysis": 48,
    "no_enrichment": 118,
    "no_data_at_all": 30
  }
}
```

---

## Guardrails

- No direct writes to Rekordbox DB.
- No staging changes without explicit user approval.
- No enrichment calls during classification — work from cache only.
- Agent must state confidence level and evidence sources for every suggestion.
- "I don't know" is always valid. Never force a low-confidence label.

## Success Criteria

Phase 1 is complete when:

- [x] `cache_coverage` tool implemented and functional.
- [ ] 468 ungenred tracks classified or explicitly marked manual-decision.
- [ ] Alias map reviewed with user; session overrides applied where needed.
- [ ] Key mismatches surfaced for user awareness.
- [ ] At least one XML export reimported into Rekordbox successfully.
- [ ] Agent can execute this full SOP end-to-end in a single session.

## Phase 2: Set Builder

Deferred until Phase 1 complete and cache coverage > 80% across all providers. Spec will live in a separate document.

Dependencies from Phase 1:

- Warm cache across audio + enrichment providers.
- Clean genre data (no ungenred tracks remaining without explicit reason).
- Key mismatches addressed or acknowledged.
