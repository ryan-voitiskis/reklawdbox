# Conjunctive Decision-Tree Templates: Implementation Plan

> **Historical design note:** This proposal predates the 2026-07-14 Beatport
> removal and source-aware confidence model. Its provider weights, source paths,
> and insertion points are not current. Re-audit it against the live
> source-aware classifier and readiness regressions before implementing any
> template work.

**Date:** 2026-04-26
**Status:** Design proposal. Depends on B1/B2/B3 (cached-feature wiring) and A1/A2/A3/A5 (new stratum-dsp features) shipping first. No template work begins until at least one of A1–A5 has passed validation.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) sections C1, C2, C4, and C5; [chord-stab-detector-plan.md](chord-stab-detector-plan.md) (A1, gates C2); [genre-classification-improvements.md](genre-classification-improvements.md), [genre-classification-implementation.md](genre-classification-implementation.md) (current consensus + Fisher behaviour).

## Goal

Five conjunctive per-genre rules ("templates") that fire only when several independent audio signals agree. Each template is a stronger override than any single-feature signal — the audio-profile Fisher layer caps any one genre's vote at `AFFINITY_CAP = 0.5` (`src/audio_profile.rs:20`), and individual `CharFlag`s influence only the same-family resolver. Templates produce confident decisions by requiring 4–6 independent flags to align before they fire.

The four remaining templates from `deep-techno-classification-ideas.md`:

- **C1 — Deep Techno (Berghain template)**: `Atonal` + Techno-family votes + `LongTail` + `rhythm_regularity > 0.9` + `Compressed` + `Dancefloor` (not `HighEnergy`) + `kick_pattern == FourOnFloor`
- **C2 — Dub Techno**: C1 base + `dub_stab_score > 0.5`
- **C4 — Electro veto**: `kick_pattern == BrokenBeat` → veto Techno-family / House-family, propose Electro
- **C5 — Tech House**: `sidechain_depth > 0.4` + `Dancefloor` + `kick_pattern == FourOnFloor`

## 1. Decision-Flow Placement

Current order in `classify_track_with_profiles` at `src/classify.rs:219`:

1. `compute_audio_profile` (`src/classify.rs:280`) → `AudioProfile { bucket, flags, bpm, centroid }`
2. `check_audio_vetoes` (`src/classify.rs:353`) — hard-vetoes Ambient, Trip-Hop, Downtempo from the energy bucket
3. `gather_votes` (`src/classify.rs:514`) — Beatport, Discogs, label, current-genre tokens, audio-profile Fisher (capped at 0.5)
4. `find_consensus` (`src/classify.rs:637`) — tally → ranked → margin/total\_weight tiers (>0.4 High, >0.15 Medium, else Low/tiebreak)
5. Same-family resolver `resolve_same_family_specificity` (`src/classify.rs:914`) — invoked inside `find_consensus` when top two are same-family
6. BPM-implausibility swap (`src/classify.rs:840`) — winner replaced by next-plausible candidate
7. HighEnergy / Dancefloor demotion (`src/classify.rs:868–894`) — `Deep Techno → Techno` etc.

Templates need to slot **between (4) consensus tally and (5) same-family resolver**, but they must be aware of (3) vote evidence. The cleanest insertion point is at the start of `find_consensus`, immediately after `ranked` is computed at `src/classify.rs:656` and before the same-family branch at `src/classify.rs:783`.

**Recommended placement: a `apply_templates` pre-pass called from `find_consensus` after the `ranked` vector exists but before any same-family resolution or confidence-tier assignment.** A template that fires:

- replaces `top_genre` with its target genre (C1, C2, C5)
- vetoes the current `top_genre` and forces a re-rank without the vetoed family (C4)
- floors confidence at High (subject to gating in §3 and §4)
- skips the same-family resolver call at `src/classify.rs:784` and `:796`
- still passes through HighEnergy demotion at `src/classify.rs:868–894` (see §7)

Templates also need access to `audio.dub_stab_score`, `audio.kick_pattern`, `audio.flux_low/mid/high`, `audio.sidechain_depth`, `audio.duration` — fields that don't exist on `AudioProfile` today. The `AudioProfile` struct at `src/classify.rs:190–195` should be extended with the new flags (§6) so the template pre-pass remains a function of `(votes, ranked, audio_profile, evidence.audio)` without re-passing every raw audio scalar.

## 2. Template Precedence

Multiple templates can fire simultaneously. The precedence order is:

```
C4 (Electro veto)             — runs FIRST, hardest signal
  ↓
C2 (Dub Techno specific)      — strictly extends C1; if both fire, C2 wins
  ↓
C5 (Tech House specific)      — sidechain is incompatible with C1's Compressed deep-Techno signature
  ↓
C1 (Deep Techno generic)      — base template
```

Justifications:

- **C4 first**: `BrokenBeat` is a hard structural signal. A track whose kick is on 2 and 4 with off-beat hits cannot be 4/4 Techno or House, period. Running C4 first means C1/C2/C5 (which all require `kick_pattern == FourOnFloor`) cannot fire on Electro tracks; this is correct by construction but explicit short-circuiting in code is clearer than relying on the implicit AND.
- **C2 over C1**: C2's signal set is a strict superset of C1's. The original spec ("Template C1 base + dub\_stab\_score > 0.5 → Dub Techno, **overrides C1**") makes C2 dominant explicit.
- **C5 over C1**: Compressed deep techno (`loudness_range < 1.0`) and aggressive sidechain (`sidechain_depth > 0.4`) can co-occur in noisy productions but should not. If both fire we treat C5's sidechain as the higher-leverage discriminator since `Compressed` is a much weaker signal (every modern dance track is loudness-compressed) than detected sidechain modulation. Log a warning in evidence when both fire so we can audit.
- **C1 before C3**: A track that meets both C1 and C3 (atonal + long tail + low flux\_high) must be a borderline case where the kick-pattern check decides. C1 requires `kick_pattern == FourOnFloor`; C3 doesn't check kick pattern but does require `flux_low low`, which a 4/4 kick should fail. So in practice they are mutually exclusive. We still order C1 before C3 to make the kick-bearing case win deterministically.

Implementation: a single `apply_templates` function that returns `Option<TemplateOutcome>`, evaluating templates in the order above and returning at the first match. Veto (C4) is special-cased — it returns a "veto" outcome that triggers a different downstream path (§5).

## 3. Discogs/Beatport Evidence Gating

A template firing on pure audio with zero supporting provider evidence is risky: the providers may know the track is something completely different (e.g. an experimental Ambient track that happens to tick the Berghain audio boxes). The audio-profile Fisher layer already caps its influence at 0.5 (`src/audio_profile.rs:20`) precisely for this reason.

**Gating rule for C1, C2, C5:** the template fires only if **the target genre's family has at least one supporting non-audio vote** in the `votes` vector (Beatport, Discogs, label, or current-genre tokens — anything except `source == "audio-profile"`). For C1/C2: at least one Techno-family vote. For C5: at least one House-family or Techno-family vote (Tech House sits on the boundary).

**C4 is the exception.** A `BrokenBeat` kick pattern is a hard structural fact about the audio that cannot coexist with a 4/4 Techno or House track regardless of provider claims — providers regularly mis-tag Electro as Tech House or Deep Techno. So C4 fires on audio alone and **vetoes** Techno-family / House-family votes outright, then re-ranks. If the second-rank candidate after vetoing is itself Techno- or House-family, C4 falls back to proposing "Electro" as a Low-confidence candidate (since we have no positive Electro evidence, only a veto). This is a deliberate downgrade from "high-confidence Electro" to "review-needed: probably Electro" when the providers offered no Electro signal at all.

**Contradiction handling for C1/C2/C5:** if the template's target genre has supporting evidence but a **different non-target genre** has *strictly higher* total provider vote weight (Discogs + Beatport + label, audio votes excluded), the template still fires but with confidence floored at Medium rather than High (§4). This handles the case where Beatport says "Tech House" with weight 1.0 but C1 fires for "Deep Techno" — we shouldn't blindly override Beatport.

## 4. Confidence Tier Implications

Confidence today is computed from margin/total\_weight thresholds at `src/classify.rs:771–820`:

- `>0.4` → High
- `>0.15` → Medium (with same-family resolver)
- else → Low or Insufficient (with audio-assisted tiebreak)

Templates short-circuit this. When a template fires:

- **High confidence by default.** The audio is unambiguous on 4–6 independent dimensions; that's a stronger signal than a 0.4 vote-margin victory.
- **Demoted to Medium** if §3's "contradiction handling" applies (a non-target genre has strictly higher non-audio vote weight).
- **Demoted to Low** if the template is C4 (veto) and there is no positive Electro evidence — see §3.
- **Never Insufficient** when a template fires.

Critically: template confidence is *not* derived from the same `margin/total_weight` formula. A template firing pushes a flag like `template-c1-fired` into `flags` and sets `confidence` directly. The same-family resolver is **skipped entirely** for template-fired classifications, because the template has already chosen between same-family alternatives by definition.

The existing BPM-implausibility check at `src/classify.rs:840` should still run on the template's chosen genre. If e.g. C1 fires with target "Deep Techno" but the BPM is 95 (outside Deep Techno's range), demote to Medium and add the `bpm-implausible` flag. This is unlikely in practice — C1's `Dancefloor` bucket implies danceability ≥ 1.0 which typically implies Techno-range BPM — but the guard is cheap.

## 5. Override Mechanics

For each template, choose between three mechanics:

- **(a) Vote boost**: add a synthetic `GenreVote { source: "template", weight: <large> }` and re-tally. Composes naturally with existing logic but the boost size is a magic number that has to outpace Beatport's 1.0 to actually win. Effectively requires weight ≥ 1.5 to beat a Beatport vote, which makes it a soft-override that's awkward to reason about.
- **(b) Direct override**: replace `top_genre` outright, set confidence directly, skip same-family resolver. Bypasses vote arithmetic. Easiest to reason about, but loses the vote-evidence audit trail.
- **(c) Resolver flag**: set a `template_target: Option<&'static str>` field that `resolve_same_family_specificity` consumes to override its depth/atmospheric heuristic.

**Recommendation:**

- **C2, C5: mechanic (b) — direct override.** These are *specific* templates that flip a previously-chosen genre to a more specific neighbour (Deep Techno → Dub Techno, Deep Techno → Tech House). A direct override is the cleanest expression of "we know better than the vote tally here." The vote audit trail is preserved in `ev_lines` via an explicit "template C2 fired: dub\_stab\_score=0.74, …" entry.
- **C1: mechanic (b) — direct override.** The resolver's only job is depth selection within a family (Techno → Deep Techno), so C1 should replace the resolver rather than feed a parallel decision path.
- **C4: mechanic (a) variant — veto-then-rerank.** C4 doesn't pick a winner; it eliminates a class. Implement as: drop all Techno-family and House-family votes from the tally, re-rank, and if the resulting top candidate is below a confidence floor (no positive Electro evidence), propose "Electro" as a Low-confidence candidate with the `kick-pattern-broken` flag set.

This means **only C4 modifies the vote vector**; C1/C2/C5 leave votes intact and override the consensus result. The vote bag remains the input to `gather_votes`'s caller and any downstream debugging.

## 6. Implementation Structure

**Recommendation: new module `src/classify/templates.rs`.**

`src/classify.rs` is already 1000+ lines and adding four templates plus dispatch logic to it inline would push it past 1500. Splitting into a sub-module:

```
src/classify.rs                  (existing — orchestration)
src/classify/templates.rs        (new — 4 templates + dispatcher)
```

To convert `classify.rs` from a flat module to a directory module, rename `src/classify.rs` to `src/classify/mod.rs` and add `mod templates;`. This is a mechanical change but it does affect every existing `use crate::classify::*` site, so audit those before splitting.

Alternative considered and rejected: **extending `compute_audio_profile` to set per-template flags consumed by the existing tree.** This would keep `classify.rs` flat but spreads template logic across `compute_audio_profile` + `find_consensus` + `resolve_same_family_specificity` and makes "template fired → confidence High" hard to express because confidence is computed downstream. Templates are a coherent unit; they should be one module.

`templates.rs` exports:

```text
pub(super) fn apply_templates(
    audio: &AudioFeatures,
    profile: &AudioProfile,
    votes: &[GenreVote],
    ranked: &[(&'static str, f32, bool)],
) -> Option<TemplateOutcome>;

pub(super) enum TemplateOutcome {
    Override { genre: &'static str, confidence: ClassificationConfidence, evidence: String, name: &'static str },
    Veto { vetoed_families: Vec<GenreFamily>, suggested: Option<&'static str>, evidence: String, name: &'static str },
}
```

One private function per template (`apply_c1`, `apply_c2`, etc.) returning `Option<TemplateOutcome>`. `apply_templates` calls them in precedence order from §2 and short-circuits on first match.

`AudioProfile` at `src/classify.rs:190` extends with new flags (`Atonal`, `LongTail`, `Compressed`) added to `CharFlag` and the new structural fields (`kick_pattern: Option<KickPattern>`, `dub_stab_score: Option<f32>`, `flux_low/mid/high: Option<f32>`, `sidechain_depth: Option<f32>`, `duration_secs: Option<f32>`). These come from `AudioFeatures` which `compute_audio_profile` already has access to.

`find_consensus` at `src/classify.rs:637` adds, around line 670 (just after `ranked` is built):

```text
if let Some(profile) = audio_profile {
    if let Some(outcome) = templates::apply_templates(audio_features, profile, &votes, &ranked) {
        return apply_template_outcome(outcome, evidence, votes, ranked, ev, flags);
    }
}
```

`apply_template_outcome` is a small helper that converts a `TemplateOutcome` into the same `(Option<&'static str>, ClassificationConfidence, Vec<String>, Vec<String>)` shape `find_consensus` already returns, performing the vote-veto re-rank for the `Veto` variant.

## 7. Integration with Existing Logic

**HighEnergy demotion (`src/classify.rs:868–894`):** templates do **not** pre-empt this. Reasoning: if C1 fires for Deep Techno but the audio is HighEnergy (`danceability > 2.5`), one of two things is true — either the danceability detector is wrong, or the track is genuinely an aggressive Techno that shouldn't be classified as Deep Techno. The `Dancefloor` bucket is a *precondition* of C1 (the original spec says "not HighEnergy"), so a HighEnergy track shouldn't trigger C1 in the first place. If it does fire and HighEnergy demotion still kicks in afterward, the demotion is correct; if it doesn't fire, no conflict. **No code change needed for compatibility — templates' bucket precondition naturally prevents the conflict.**

**Same-family resolver (`src/classify.rs:914`):** templates that fire skip it entirely. This is enforced by template outcomes returning a final `(Option<&'static str>, ClassificationConfidence, …)` from `find_consensus` early, before reaching the `same_family` branch at `src/classify.rs:783`/`:796`.

**Audio-profile Fisher affinity (`src/audio_profile.rs:514–524`):** templates supplement, not replace. Fisher votes still enter the vote tally at `src/classify.rs:610` capped at `AFFINITY_CAP = 0.5`. Templates run *after* the tally is formed and can boost the same target genre that Fisher already voted for — that's fine, the audit trail will show both signals agreeing. If a template fires for a *different* target than Fisher's top affinity, the template wins (it has 4–6 signals; Fisher has its multivariate distance score). Log both in evidence so disagreement is visible.

**`check_audio_vetoes` at `src/classify.rs:353`:** the existing energy-bucket vetoes (Ambient, Trip-Hop, Downtempo, Drum-and-Bass) run before vote gathering and short-circuit the entire classifier. They are unaffected by templates and continue to take precedence.

**BPM-implausibility swap (`src/classify.rs:840`):** runs after templates fire, on the template's chosen genre. See §4.

## 8. Dependency Map and Staging

```
B1 Atonal flag         (key_confidence < 0.1)        — cheap
B2 LongTail flag       (decay_mid_tau > 200ms)       — cheap
B3 Compressed flag     (loudness_range < 1.0)        — extract loudness_range first

A1 Chord-stab          (dub_stab_score)              — chord-stab-detector-plan.md
A2 Kick-pattern        (kick_pattern enum)
A3 Band-split flux     (flux_low/mid/high)
A4 Sub-rumble          (sub_rumble_proportion)       — not used by any template
A5 Sidechain depth     (sidechain_depth)

Templates by dependencies:
  C4 Electro veto      ← A2 (only)
  C5 Tech House        ← A2 + A5
  C2 Dub Techno        ← A1 + A2 + B1 + B2 + B3
  C1 Deep Techno       ← A2 + B1 + B2 + B3
```

**Staging:**

1. **Stage 0 — B-flags**: `Atonal`, `LongTail`, `Compressed` wired into `CharFlag` at `src/classify.rs:181`, set in `compute_audio_profile` at `src/classify.rs:280`. ~1 PR. Independent. Unblocks nothing on its own but is fast feedback that the wiring works.
2. **Stage 1 — A2 kick-pattern**: stratum-dsp work plus mapping to `AudioFeatures`. Unblocks C4. ~1 large PR (similar shape to chord-stab-detector-plan.md but smaller because no period/template scoring).
3. **Stage 2 — C4 (Electro veto)**: simplest template, single dependency. Tests the template-dispatch infrastructure end-to-end on a low-risk template (a veto can be tuned conservatively without affecting positive classifications).
4. **Stage 3 — A1 chord-stab**: per chord-stab-detector-plan.md, validation gates classification wiring.
5. **Stage 4 — C2 (Dub Techno)**: requires A1 + A2 + B-flags. Highest-leverage template (Dub Techno↔Deep Techno is the most consequential confusion).
6. **Stage 5 — A5 sidechain depth + C5 (Tech House)**: bundle together; A5 only feeds C5 and one same-family resolver hint, not worth a separate PR.
7. **Stage 6 — C1 (Deep Techno)**: requires A2 + B-flags only. Drops in cleanly because C2 has already proven the template-dispatch pattern, and C1's dispatch-order spot is already wired up by C2 (C2 short-circuits before C1 runs).
Total: ~7 PRs over the templates infrastructure (some bundled with their dependency PRs). At any stage, partial deployment is meaningful — e.g. shipping just C4 already prevents a real class of misclassification.

## 9. Validation

Validation per template, mirroring the methodology in `genre-classification-improvements.md`:

1. **Curate 4–6 ear-verified positive tracks per template** (genre matches the template's target, audio meets all conjunction conditions).
2. **Curate 4–6 ear-verified negative tracks per template** (genre is something the template should *not* fire for, but tracks share some audio characteristics with the positive set — e.g. for C1, include atonal Ambient Techno and atonal Tech House).
3. **Run the classifier on positives and negatives** and record (a) whether the template fired, (b) the final genre, (c) the final confidence.
4. **Acceptance criteria**: precision ≥ 0.8 (of tracks the template fires on, ≥80% are correctly classified) **and** recall ≥ 0.8 (of canonical positive tracks, ≥80% have the template fire). Lower than chord-stab's 6/8 + 7/8 thresholds because templates combine multiple signals and small misses on individual flags cumulate.
5. **Threshold-sweep audit**: for each numeric threshold in the template (e.g. `dub_stab_score > 0.5`, `flux_high < 30`), produce a small table of how the precision/recall figures shift at ±20% threshold. Documents the tuning surface.

Per-template fixture sets:

- **C1 Deep Techno**: positives — Marcel Dettmann, Klockworks, Norman Nodge, Sandwell District. Negatives — atonal Tech House (Hot Creations), sparse Ambient Techno (Voices From The Lake), atonal Dub Techno (Basic Channel — should fire C2 not C1).
- **C2 Dub Techno**: positives — Basic Channel, Echocord, Convextion, Burger/Ink. Negatives — Deep Techno (no chord stab), Tech House with off-beat hats, ambient pads.
- **C4 Electro veto**: positives (where veto should fire) — DMX Krew, Drexciya, Anthony Rother, Helena Hauff. Negatives (where veto should NOT fire) — straight 4/4 Detroit Techno, Tech House. Also include Drum-and-Bass and Halftime Dubstep to confirm the veto doesn't over-extend (those should already be caught by `check_audio_vetoes` upstream).
- **C5 Tech House**: positives — Hot Creations roster, Solid Grooves, Cuttin' Headz. Negatives — Deep Techno (no sidechain), straight House (sidechain present but typically lighter), Disco-influenced House.

Reuse the chord-stab fixture set where applicable (C2's positives and negatives overlap with chord-stab-detector-plan.md's set substantially).

Validation harness: a new test in `src/classify/tests/` that loads cached `AudioFeatures` for the fixture track IDs and asserts template firing. Run on demand, gated on fixtures being available locally (similar pattern to chord-stab-detector-plan.md's `dub_stab_validation.rs`).

## 10. Risks

1. **Over-firing (false positives).** A template fires on tracks the providers correctly identify as something else. Mitigations: §3 gating (require non-audio vote support); §4 demotion-to-Medium when contradicted; per-template precision tracked in validation; conservative initial thresholds (e.g. `dub_stab_score > 0.6` rather than `> 0.5` for C2 in the first ship).

2. **Under-firing (false negatives).** A canonical Berghain Deep Techno track that doesn't tick all six C1 boxes (e.g. `loudness_range = 1.05`, just above the `Compressed` threshold). Mitigations: explicit threshold-sweep audit (§9 step 5); permit one of {Compressed, Atonal} to be a "near miss" (within 20% of threshold) for C1 — but this adds tuning surface and should be deferred to v2 if simple thresholds underperform.

3. **Interaction effects.** Multiple templates firing produce inconsistent overrides — e.g. C5 fires saying "Tech House", C1 fires saying "Deep Techno". Mitigations: §2 strict precedence ordering with first-match short-circuit; explicit log entry when more than one template *would have* fired (run the rest of the templates in audit-only mode to surface this in evidence). The audit-mode run is cheap (just five boolean checks) and catches threshold drift early.

4. **Threshold drift.** The multi-feature thresholds tune together; tweaking `dub_stab_score > 0.5` to `> 0.6` for C2 might pass a track that then satisfies only C1's criteria, flipping it from "Dub Techno High" to "Deep Techno High" — silently changing classifications. Mitigations: the per-template threshold-sweep audit (§9 step 5) plus regression tests on the curated fixture set whose expected outputs are checked into the repo.

5. **Vote-veto re-rank distortions in C4.** Vetoing all Techno-family / House-family votes can leave the second-rank candidate looking artificially strong. If e.g. a track's votes are `[Tech House: 1.2, Drum-and-Bass: 0.3]`, vetoing Tech House promotes Drum-and-Bass to top — but the original Drum-and-Bass weight was a noise vote, not a real signal. Mitigation: after veto, require the new top candidate to have non-vetoed weight ≥ 0.5 to stand; otherwise propose "Electro" with Low confidence (§3 fallback).

6. **`Compressed` flag is a weak signal.** Most modern dance music masters to `loudness_range < 1.0`. If C1 leans heavily on `Compressed`, it'll over-fire. Mitigation: validate the discriminative power of `Compressed` against the fixture set as part of B3 wiring, *before* C1 ships. If `Compressed` shows total overlap (per the methodology in `genre-classification-improvements.md`), drop it from C1's conjunction.

## 11. PR Breakdown

In dependency order, smallest first:

1. **PR T0 — `apply_templates` infrastructure.** No actual templates yet — just the module skeleton, `TemplateOutcome` enum, dispatcher, and integration into `find_consensus`. Wire-up tests prove the dispatcher returns `None` for all current tracks (no behavioural change). Lets PR T1+ each ship as a small additive change. Depends on B1/B2/B3 being wired (because templates reference those flags).
2. **PR T1 — C4 (Electro veto).** Simplest template; only depends on A2. Includes vote-veto-and-rerank logic in `apply_template_outcome`. Most defensive: if C4 fails validation, no positive classification regresses (worst case: no Electro vetoing, status quo).
3. **PR T2 — C2 (Dub Techno).** Highest leverage. Depends on chord-stab classification wiring landing first (PR 6 in chord-stab-detector-plan.md).
4. **PR T3 — C5 (Tech House).** Depends on A5 sidechain depth.
5. **PR T4 — C1 (Deep Techno).** Comes after C2 because C2 supersedes C1; if C1 ships first, every Dub Techno track classifies as Deep Techno until C2 lands.

Each PR is independently revertable. Each PR ships with its fixture set and validation report committed alongside.

## Cost Estimate

| Stage | Effort |
|---|---|
| PR T0 (infrastructure) | 1 day |
| PR T1 (C4 Electro veto) | 0.5 day (after A2 lands) |
| PR T2 (C2 Dub Techno) | 0.5 day (after A1 + B-flags land) |
| PR T3 (C5 Tech House) | 0.5 day (after A5 lands) |
| PR T4 (C1 Deep Techno) | 0.5 day |
| Per-template fixture curation + validation | 0.5 day each = ~2 days |
| **Total** | ~5 days, spread across A/B feature landings |

Templates themselves are cheap — almost all the cost is in their dependencies (A1–A5 stratum-dsp work and validation-set curation per template).

## Open Questions

- Should template firing be exposed in the MCP `classify_tracks` output as a structured field (e.g. `template_fired: Some("C2")`) so downstream tools can audit template hit-rates without parsing evidence strings? Probably yes; cheap to add.
- For C4 specifically: should the veto-then-rerank fall back to proposing "Electro" *with* the missing-Electro-evidence flag, or surface it as a Low-confidence "Insufficient" requiring user review? Current recommendation in §3/§5 is the former. Re-evaluate after C4 ships in production for a week.
- Should `Compressed` (B3) be validated against the fixture set as a pre-condition for C1 shipping, or trusted on theoretical grounds? Recommendation: validate. If it has no discriminative power, drop from C1 and rely on the other five conditions.
- If two templates fire simultaneously despite §2 precedence (audit-only mode catches this), should the classifier emit a `flags.push("template-conflict")` warning so the audit tooling surfaces them for manual review? Yes — cheap and high-information.
