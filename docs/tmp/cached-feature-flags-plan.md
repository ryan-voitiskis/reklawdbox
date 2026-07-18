# Cached Feature Flags: Implementation Plan

**Date:** 2026-04-26
**Status:** Wiring-only plan. No new audio analysis or DSP work. All four features either already exist in the cache or come from already-emitted Essentia output that is not being read.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) — items B1–B4.
**Prior validation context:** [genre-classification-improvements.md](genre-classification-improvements.md) — empirically invalidated `harmonic_proportion`, `bpm_confidence` (single-detector), `grid_stability`, `mod_centroid`. Features in this plan are different signals; see B1 note below.

## Goal

Wire four cached audio features into the classification decision tree. None of these require new DSP; the cost is reading already-cached values and adding a handful of decision-rule additions. This is the highest-ROI batch from the parent ideas doc and a clean dry-run for the broader "promote cached features into the tree" pattern.

## Why these are not subject to the prior invalidation

The 2026-04-10 research invalidated `bpm_confidence`, `grid_stability`, `harmonic_proportion`, `mod_centroid` for total cross-genre overlap on a 17-track ear-verified set. The features in this plan are categorically distinct:

- **`key_confidence`** (B1): direct atonality signal. The invalidated `bpm_confidence` is a *rhythmic-detection-quality* metric; `key_confidence` is a *tonal-content-presence* metric. Atonality is itself a genre discriminator (Deep House requires chord progressions, Deep Techno is typically atonal). The signal is the data, not the detector quality.
- **`decay_mid_tau`** (B2): already validated for cross-genre separation — it is a Fisher-input feature in `src/audio_profile.rs:32-46` and survived the calibration pipeline's variance check. The wiring gap is tree-side only.
- **`loudness_range`** (B3): produced by Essentia's BS.1770 EBU loudness estimator, distinct from `dynamic_complexity` (perceptual loudness variance). `loudness_range` measures the LU spread between low and high loudness percentiles — a direct mastering-style signal. Listed as "Theoretical" in `genre-classification-improvements.md:835` (untested but plausible). Validation must confirm.
- **`bpm_agreement`** (B4): a *cross-detector consistency* check (Rekordbox vs stratum vs Essentia), not a single-detector confidence. It tells the tree when the Rekordbox BPM is suspect — the prior invalidation explicitly does not touch it (note in `deep-techno-classification-ideas.md:107`).

## Per-Flag Specification

### B1 — `Atonal` flag from `key_confidence`

- **Threshold:** `audio.key_confidence < 0.1`
- **Rationale:** Pilot four-track session at `deep-techno-classification-ideas.md:13-16` showed `key_conf` ranging 0.01–0.05 across atonal Techno-family / Electro / Downtempo tracks. 0.1 leaves a small headroom for borderline-tonal cases.
- **Currently extracted?** No. `key_confidence` is in the stratum cache at `src/audio.rs:35` (and serialised in `stratum-dsp/src/analysis/result.rs:197`) but `extract_audio_features` in `src/tools/classify_handler.rs:687-722` does not pull it.
- **Distinct from:** `bpm_confidence` (invalidated single-detector rhythmic quality) and `key_clarity` (already extracted at `classify_handler.rs:706-709`, used in Fisher).

### B2 — `LongTail` flag from `decay_mid_tau`

- **Threshold:** `audio.decay_mid_tau > 200.0` (ms)
- **Rationale:** From the Berghain Deep Techno pilot (`deep-techno-classification-ideas.md:15`, decay 336 / 254 ms), and cross-checked against shorter-tail Downtempo (102 / 83 ms). 200 ms is the empirically-suggested split.
- **Currently extracted?** Yes — already in `AudioFeatures.decay_mid_tau` at `src/classify.rs:129` and consumed by Fisher at `src/audio_profile.rs:41`. It just isn't used to set a tree-side `CharFlag`.
- **Distinct from:** Nothing invalidated. Already passes the Fisher cross-genre variance check.

### B3 — `Compressed` flag from `loudness_range`

- **Threshold:** `audio.loudness_range < 1.0` (LU)
- **Rationale:** EBU R128 loudness range. Heavily mastered club tracks compress to <1 LU; headphone/listening masters typically present 4–10+ LU. Validation must confirm against verified buckets (this is the "Theoretical" flag from `genre-classification-improvements.md:835`).
- **Currently extracted?** Partially. Python emits it (`src/essentia_analysis.py:55-69`) and the Rust struct has the field (`src/audio.rs:73` in `EssentiaOutput`), but `extract_audio_features` at `classify_handler.rs:687-722` never reads it onto `AudioFeatures`. It is read in scoring (`src/tools/scoring.rs:663,687`) — that path is unrelated to classification.
- **Distinct from:** `dynamic_complexity` (Essentia perceptual loudness variance over time) measures *moment-to-moment loudness variability*; `loudness_range` is a *quasi-stationary distribution-percentile* metric. They are correlated but not redundant; classification doc at `genre-classification-implementation.md:269` calls them redundant for the Fisher input set, but for a flat `< 1.0` threshold the percentile-based metric is the right primitive.

### B4 — `bpm_agreement` for plausibility-check fallback

- **Trigger:** `bpm_agreement == Some(false)`, i.e. `|stratum_bpm - rekordbox_bpm| > 2.0` per the existing computation at `src/tools/classify_handler.rs:685`. Tighten downstream to >3% relative when both detectors agree against Rekordbox.
- **Behaviour:** When triggered and both stratum + Essentia consensus on a different BPM (within 3% of each other), use `(stratum_bpm + essentia_bpm) / 2` as the effective BPM for the plausibility check at `src/classify.rs:625`. Add a flag `"bpm-rekordbox-disagrees"` so the disagreement is visible in evidence/flags output. Conservative guard from implementation validation: only apply this on Dancefloor/HighEnergy audio and reject near-2x tempo relationships, because low-energy/non-dancefloor tracks commonly produce detector consensus at misleading half/double-time tempos.
- **Currently extracted?** Yes, as `Option<bool>` at `src/classify.rs:122` with `#[allow(dead_code)]`. The relative-disagreement check is the new logic.
- **Distinct from:** `bpm_confidence` (invalidated single-detector). `bpm_agreement` is a cross-source consistency check — same source-of-truth validation pattern as enrichment cross-confirmation.

## CharFlag Enum Additions

`CharFlag` lives at `src/classify.rs:181-188` with six variants today (`Ambient`, `Atmospheric`, `Broken`, `Irregular`, `Fast`, `Slow`). Add three:

- `Atonal` (B1)
- `LongTail` (B2)
- `Compressed` (B3)

Update the evidence-string mapping at `src/classify.rs:721-730` (the `match f` inside `find_consensus`) to include `"atonal"`, `"long-tail"`, `"compressed"`.

## `compute_audio_profile` Additions

Function at `src/classify.rs:280-324`. Changes, in source-order:

1. **After existing `dc` and `rr` reads (lines 295-296):** read `key_confidence`, `decay_mid_tau`, `loudness_range` from `audio` (all `Option<f64>`).
2. **After the existing flag-set block (lines 298-316):** push `CharFlag::Atonal` if `key_confidence.is_some_and(|kc| kc < 0.1)`.
3. **Same block:** push `CharFlag::LongTail` if `decay_mid_tau.is_some_and(|t| t > 200.0)`.
4. **Same block:** push `CharFlag::Compressed` if `loudness_range.is_some_and(|lr| lr < 1.0)`.

Each check is a single `if let` — under 10 lines added.

## Decision-Rule Additions

### B1 — `Atonal` rules

1. **In `resolve_same_family_specificity` (`src/classify.rs:914-960`):** when family is Techno-family and `Atonal` is set, prefer the *deeper* (Deep Techno) over Techno proper regardless of energy bucket — currently only `Atmospheric` or `LowEnergy` triggers depth preference (line 936). Atonal should join that condition for Techno-family.
2. **In the same resolver:** when family is House-family and `Atonal` is set, *strongly* prefer the shallower (House) over Deep House — Deep House requires chord progressions, so Atonal-Deep-House is contradictory. This is a near-veto, not just a tiebreak; consider also returning Tech House if Tech House is a candidate.
3. **In `audio_clearly_favors_family` (`src/classify.rs:970-1008`):** Atonal disqualifies the House family entirely (return `false` for `GenreFamily::House`).
4. **HighEnergy demotion interaction (`src/classify.rs:868-894`):** Atonal does *not* prevent HighEnergy demotion — a HighEnergy atonal track is still demoted from Deep Techno → Techno. The B1 boost runs *before* demotion in the same-family resolver and applies only at Dancefloor/LowEnergy. No interaction change needed.

### B2 — `LongTail` rules

1. **In `resolve_same_family_specificity`:** `LongTail` reinforces the existing `Atmospheric || LowEnergy` deep-preference branch at line 936 — add it to the OR.
2. **In `audio_clearly_favors_family` for Techno:** `LongTail` joins `dark_timbre` as an alternative qualifier in the LowEnergy branch (line 989-993) — currently only `dark_timbre` qualifies.

### B3 — `Compressed` rules

1. **In `resolve_same_family_specificity`:** `Compressed + Techno-family + Dancefloor` → prefer Deep Techno over Techno (club-master signal).
2. **In `audio_clearly_favors_family` for Techno (Dancefloor branch, line 986-988):** `Compressed` joins as a confirming condition (raises confidence of the audio-tiebreak).
3. **Negative for Downtempo/Ambient:** `Compressed` makes the Ambient veto at `src/classify.rs:373-387` *less* likely correct — if `Compressed` is set, skip the expanded Atmospheric → Ambient veto. (Atmospheric+Compressed is more likely Dub Techno than Ambient.)

### B4 — BPM disagreement fallback

1. **In `gather_votes` around `src/classify.rs:625` (the `bpm_plausible` calls):** compute an `effective_bpm`:
   - Default: `audio_profile.bpm` (Rekordbox BPM, current behaviour)
   - When `bpm_agreement == Some(false)` AND `essentia_bpm` is present AND `|essentia_bpm - stratum_bpm| / stratum_bpm < 0.03` AND the audio bucket is Dancefloor/HighEnergy AND the detector consensus is not near 2x Rekordbox BPM: set `effective_bpm = (stratum_bpm + essentia_bpm) / 2`
   - Otherwise: keep Rekordbox BPM
2. **Pass `effective_bpm` to all `bpm_plausible(...)` calls** in `gather_votes`. The same value is used at line 614 for the audio-profile-vote plausibility check.
3. **Add an evidence line** when fallback fires: `"bpm-fallback: rekordbox 130.8 → detector consensus 125.0"`. Add flag `"bpm-rekordbox-disagrees"`.
4. **Existing BPM override at `src/classify.rs:840-866`:** uses `audio_profile.bpm` (Rekordbox). Update to use the same `effective_bpm` so override behaviour is consistent with the gather-votes plausibility decision.

## `AudioFeatures` Struct Additions

Struct at `src/classify.rs:117-151`. Add:

- `pub(crate) key_confidence: Option<f64>` (new — B1)
- `pub(crate) loudness_range: Option<f64>` (new — B3)
- `pub(crate) essentia_bpm: Option<f64>` (new — B4 needs Essentia BPM separately from `stratum_bpm`)

Already present and used:
- `decay_mid_tau` (B2)
- `bpm_agreement` (B4)
- `stratum_bpm` (B4)

Drop the `#[allow(dead_code)]` on `bpm_agreement` and `stratum_bpm` once read by the new logic.

Update the `make_audio` test helper at `src/classify.rs:1310-1331` to include defaults for the new fields (`None`), and the same in `src/audio_profile.rs:833,964` and any other test fixtures using `AudioFeatures` literals.

## `classify_handler` Extraction Changes

`extract_audio_features` at `src/tools/classify_handler.rs:687-722`. Add three field reads:

1. **`key_confidence`:** from `stratum_json` (alongside the existing `decay_mid_tau` / `key_clarity` reads at lines 698-709). JSON key: `"key_confidence"`. Maps to `f64`.
2. **`loudness_range`:** from `essentia_data` (the typed `EssentiaOutput`). Use `essentia_data.as_ref().and_then(|e| e.loudness_range)`. The field exists at `src/audio.rs:73`.
3. **`essentia_bpm`:** from `essentia_data` — `essentia_data.as_ref().and_then(|e| e.bpm_essentia)` (field at `src/audio.rs:76`).

No changes needed to the `EssentiaOutput` or `StratumResult` structs — fields already exist.

## Schema Versioning

- **`STRATUM_SCHEMA_VERSION` (`src/audio.rs:60`, currently `"4"`):** *no bump needed*. `key_confidence` is already serialised into the cached JSON; we are only changing the consumer.
- **`ESSENTIA_SCHEMA_VERSION` (`src/audio.rs:61`, currently `"2"`):** *no bump needed for B3*. `loudness_range` is already in `EssentiaOutput` and serialised — Python already emits it (verified `src/essentia_analysis.py:69`), and tests at `src/audio.rs:809` confirm round-trip. The field is just not read by the classifier.
- **B4:** no schema impact — `bpm_essentia` (Essentia) and `stratum.bpm` are both already serialised.

If validation surfaces that any cached entry pre-dates loudness_range emission, bump Essentia to `"3"` to force re-analysis. Default assumption: not needed.

## Tests

Existing tests live in `src/classify.rs:1306-end` (`mod tests`). Helper `make_audio` at line 1310 — must extend with new field defaults.

Add new test cases:

1. **`atonal_house_demotes_to_house_or_tech_house`** — Atonal flag set, House-family votes, asserts result is not Deep House. (Covers the parent doc's pilot Untitled 42433 misclassification.)
2. **`atonal_techno_prefers_deep_techno`** — Atonal + Dancefloor + Techno-family votes, asserts Deep Techno wins over Techno.
3. **`compressed_dancefloor_prefers_deep_techno`** — Compressed + Dancefloor + Techno-family ties, asserts Deep Techno over Techno.
4. **`bpm_disagreement_uses_detector_consensus`** — Rekordbox 130.77, stratum 125, Essentia 125, genre with BPM range 118–132. Without fallback the genre is BPM-implausible; with fallback it is plausible. Asserts `"bpm-rekordbox-disagrees"` flag set.
5. **`bpm_disagreement_no_consensus_uses_rekordbox`** — Rekordbox 130, stratum 125, Essentia 130. Detectors disagree with each other → fall back to Rekordbox (no flag).
7. **`compressed_atmospheric_skips_ambient_veto`** — verifies the negative B3 rule that Compressed+Atmospheric is not silently routed to Ambient by the expanded veto at `classify.rs:373-387`.

Tests in `src/tools/tests.rs` (e.g. line 3023, 3104, 3335, 3654) using `loudness_range: None` and `bpm_agreement: ...` already exercise the surrounding paths; verify they still pass after struct additions.

## PR Breakdown

Each B-item is a small, independently revertable PR. Order chosen for: cheapest first, highest-immediate-accuracy first, lowest-risk first.

1. **PR 1 — B1 (`Atonal`).** ~30 LOC. Catches the parent doc's Deep House misclassification. Lowest extraction cost (`key_confidence` read is mechanical), most-confident threshold (the pilot data is unambiguous: 0.01–0.05 across atonal tracks). Includes the House-family near-veto, which is the highest-impact single rule.
2. **PR 2 — B3 (`Compressed`).** ~30 LOC. New `Option<f64>` extraction + flag + 3 rules + ambient-veto interaction. Slightly higher risk (threshold less proven) but mechanical wiring. Run validation before merging.
3. **PR 3 — B2 (`LongTail`).** ~20 LOC. Pure flag addition (the value is already extracted) + rule additions. Lowest-risk because the Fisher path already validates that `decay_mid_tau` separates genres.
4. **PR 4 — B4 (`bpm_agreement` fallback).** ~50 LOC. Most-substantial logic change (touches `gather_votes`, BPM plausibility, BPM override). Land last since it changes vote-gathering semantics rather than just flag-setting; lowest reversibility risk if landed alone with focused tests.

PR 1 is also the cleanest dry-run for the broader pattern; if its validation surfaces threshold-tuning issues, the same lessons apply to PR 2 and PR 3.

## Validation

For these threshold-based features, validation is cheaper than for new DSP. Steps per PR:

1. Run `classify_tracks` (or the audit equivalent) over the existing 574-track verified playlist (memory: `genre_verified` Rekordbox playlist).
2. For each new flag, count flag-fire rate per genre bucket. Expected:
   - **Atonal**: highest in Deep Techno / Dub Techno / Ambient; near-0% in Deep House / Disco / Soul / Trance. Fires in House would mean threshold too lenient.
   - **LongTail**: highest in Dub Techno / Ambient Techno / Deep Techno; low in Tech House / Hardcore / DnB. Should not fire on most Downtempo (per pilot data, Downtempo had 102 ms — well below threshold).
   - **Compressed**: highest in Deep Techno / Tech House / Hardcore; low in Ambient / Trip-Hop / Downtempo. If it fires on >40% of Downtempo, threshold is wrong.
3. Manual spot-check 5 unexpected fires per flag (e.g. "fired on a House track" for Atonal). If unexpected fires represent a genre-distribution issue rather than a threshold issue, accept and move on; if the flag is firing on most of an unrelated genre bucket, retune.
4. For B4: count tracks where the fallback fires (i.e. Rekordbox disagrees with detector consensus). Expected to be small (single-digit percent of library); large fire-rate suggests detector calibration is the problem, not the fallback.

No fixture-set work needed; the verified playlist already exists.

## Risks

1. **Threshold sensitivity.** The three thresholds (0.1, 200 ms, 1.0 LU) are eyeballed from the parent doc and standard mastering practice, not validated. **Mitigation:** validation step above; surface flag-fire rates and tune before merging. If a threshold is uncertain, ship behind a feature flag and tune from production data.

2. **Over-firing flags.** Particularly `LongTail` — `decay_mid_tau > 200 ms` may capture more than just dub/drone tails (e.g. reverb-heavy House). **Mitigation:** the rules using LongTail are conjunctive (LongTail+Atonal, or LongTail in the LowEnergy Techno branch), so a noisy single flag does little damage. If LongTail-alone proves noisy, the rule additions remain correct.

3. **B3 Essentia `loudness_range` semantics.** Loudness range from EBU R128 measures perceived spread; very-short tracks (<30 s) can yield artificially low values. **Mitigation:** add a duration guard (`duration > 60s`) before setting `Compressed`. The duration field is already available via stratum's `duration_seconds` (`src/audio.rs:38`).

4. **B4 detector-disagreement direction.** The fallback assumes when Rekordbox disagrees with detector consensus, Rekordbox is wrong. This is consistent with the pilot (Untitled 26596) but not always true — Rekordbox sometimes corrects double-time / half-time cases that detectors get wrong. **Mitigation:** require >3% relative disagreement (not just the existing 2 BPM absolute), require *both* detectors to agree against Rekordbox, reject near-2x tempo relationships explicitly, and only apply the fallback when the audio profile is Dancefloor/HighEnergy. Validation on the rehydrated `genre_verified` cache showed these extra guards are necessary: without them the fallback mostly fired on Ambient/Dancehall half-time or low-energy cases.

5. **Averaging vs picking the more-confident detector (B4).** Averaging stratum and Essentia is simpler but may dilute when one detector is right and the other is wrong. **Mitigation:** ship averaging first (deterministic, simple). If validation surfaces consistent-bias cases, switch to "prefer stratum when stratum_bpm_confidence is high" — but `bpm_confidence` is invalidated, so a heuristic like "prefer the detector whose BPM is closer to a common dance-music range (110–140)" may be needed instead. Defer to a follow-up if needed.

6. **Interaction with existing depth demotion (`src/classify.rs:868`).** HighEnergy still demotes Deep Techno → Techno even when Atonal is set. This is intentional (HighEnergy is a louder signal than Atonal), but verify with a test case (`high_energy_atonal_still_demotes_deep_techno`).

7. **Test fixture sprawl.** Many existing tests use `AudioFeatures { ... }` literals; adding three fields means touching every literal. Alternative: introduce `AudioFeatures::default_for_test()` helper to centralise. Prefer the helper to avoid future churn.
