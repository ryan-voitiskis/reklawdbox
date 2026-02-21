# SOP Step 2 Taxonomy Review Report

Test run of `docs/genre-classification-sop.md` Step 2 against a live Rekordbox library (2460 tracks, 1992 genred, 468 ungenred).

## Results

### Library Genre Distribution (canonical genres)

| Genre | Tracks |
|-------|--------|
| Techno | 501 |
| House | 432 |
| Deep House | 264 |
| Breakbeat | 167 |
| Ambient | 118 |
| Dub Techno | 69 |
| Electro | 46 |
| Deep Techno | 38 |
| Disco | 21 |
| IDM | 20 |
| Downtempo | 17 |
| Drum & Bass | 15 |
| Experimental | 12 |
| Dancehall | 10 |
| Ambient Techno | 8 |
| Tech House | 7 |
| Trance | 6 |
| Pop | 6 |
| Broken Beat | 6 |
| Synth-pop | 3 |
| Reggae | 3 |
| Dubstep | 3 |
| Psytrance | 2 |
| Hip Hop | 2 |
| Garage | 2 |
| Rock | 1 |
| Afro House | 1 |

### Alias Review

199 tracks across 34 alias mappings were reviewed. User accepted most aliases (formatting normalization like Hip-Hop → Hip Hop, DnB → Drum & Bass, R & B → R&B) without issue.

### Unknown Genres

13 tracks with 4 unmapped genres:

| Genre | Tracks | Decision |
|-------|--------|----------|
| Electronic Techno | 10 (all Kangding Ray) | Case-by-case during classification |
| Jazz | 1 (Flying Lotus — Pygmy) | Leave as Jazz — add to taxonomy |
| Ballad | 1 (Britney Spears — Everytime) | Case-by-case during classification |
| Anti-music | 1 (Grace — Untitled 1) | Case-by-case during classification |

## Taxonomy Bugs to Fix in `genre.rs`

The following alias mappings are incorrect and should be removed or changed. These genres should be added as canonical genres in the taxonomy:

1. **Dub Reggae → Dub** — Dub Reggae is a distinct genre, not the same as Dub (which in this collection's context leans toward Dub Techno influence). Should stay as `Dub Reggae` or be added as canonical.

2. **Drone Techno → Deep Techno** — Drone Techno is a distinct subgenre (Sigha, SHXCXCHCXSH, Acronym etc.). Flattening to Deep Techno loses useful specificity for set building. Should be canonical.

3. **Gospel House → House** — Gospel House is a meaningful subgenre (Terrence Parker gospel edits). Should be canonical.

4. **Progressive House → House** — Progressive House is well-established and distinct from House. Should be canonical.

5. **Highlife → Afro House** — Highlife is a West African genre, not the same as Afro House (which is electronic). Should be canonical.

6. **Jazz** — Not in the taxonomy at all. Should be added as a canonical genre (Flying Lotus, etc.).

These are specific to this user's library, but future users will likely hit the same issues with Drone Techno, Progressive House, and Highlife at minimum.

## SOP Feedback

### What worked well

- `suggest_normalizations` tool output is well-structured — the alias/unknown/canonical split makes the review straightforward.
- The SOP's format for presenting aliases with track counts and debatable flags is effective.
- The override mechanism (session overrides) works — the agent can track user preferences without touching compiled code.

### What the SOP should change

1. **Taxonomy should be refined through conversation BEFORE mapping recommendations are presented.** The current SOP flow is: get taxonomy → present aliases → collect overrides. But many of the "debatable" mappings are actually taxonomy bugs (wrong aliases). The user shouldn't have to override things that shouldn't be aliased in the first place.

   **Proposed change:** Add a Step 1.5 or pre-step where the agent presents the full canonical genre list and alias map to the user for review, discusses additions/removals, and the user refines the taxonomy. THEN run `suggest_normalizations` against the refined taxonomy. This avoids the awkward pattern of "here are 21 debatable mappings, override each one" when several of them are simply wrong.

   In practice this means the taxonomy in `genre.rs` should be configurable (e.g., a TOML/JSON file) rather than compiled, so user preferences persist across sessions without code changes. Until then, the SOP should at least document that the alias map is a starting point and will need per-user refinement.

2. **The "debatable" flag is applied to too many aliases.** Every mapping that isn't a pure formatting fix (Hip-Hop → Hip Hop) gets flagged as debatable, making the list noisy. The agent should distinguish between:
   - **Clearly wrong** (Highlife → Afro House) — these are bugs
   - **Loses specificity but defensible** (Progressive House → House) — these are preference
   - **Reasonable normalization** (Techno (Peak Time / Driving) → Techno) — these are fine

   The SOP should provide criteria for each category rather than blanket-flagging everything.

3. **Unknown genre handling is underspecified.** The SOP says "Map to — assign a canonical genre for this session" or "Leave." But for genres like "Electronic Techno" (10 tracks, single artist), the right answer might be to inspect the tracks and decide they're actually Techno or Experimental. The SOP should suggest looking at enrichment data for unknown-genre tracks before asking the user to map them blind.
