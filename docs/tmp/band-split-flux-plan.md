# Band-Split Spectral Flux: Implementation Plan

**Date:** 2026-04-26
**Status:** Design proposal. No implementation yet.
**Related:** [deep-techno-classification-ideas.md](deep-techno-classification-ideas.md) — feature A3. [chord-stab-detector-plan.md](chord-stab-detector-plan.md) — structural template (this is a much smaller change). [genre-classification-improvements.md](genre-classification-improvements.md) — empirically invalidated features list (do not reintroduce: `harmonic_proportion`, `bpm_confidence`, `grid_stability`, `mod_centroid`).

## Goal

Replace the single `spectral_flux_mean` global average with three band-restricted values — `flux_low`, `flux_mid`, `flux_high` — so the Berghain Deep Techno signature (low low-band flux + low mid-band flux + high upper-band flux) becomes visible to the Fisher discriminant. Tonight's Untitled 27037 example: `spectral_flux_mean = 18` (read as drone-y), but the user heard "warped synth messy melodies" rhythmically active in 2–8 kHz — the global average drowned that signal in the static lows.

## Important Up-Front Note: Where the Feature Lives Today

The user prompt frames this as a stratum-dsp change. That is the right *target* but not the current state — `spectral_flux_mean` is sourced from **Essentia** (`src/essentia_analysis.py:233`, frame-by-frame `es.Flux()` divided by frame energy), serialised on `EssentiaOutput` (`src/audio.rs:86–87`), and threaded through `classify_handler.rs:714`. `StratumResult` (`src/audio.rs:30–48`) does not currently carry it. stratum-dsp computes the shared STFT (`stratum-dsp/src/lib.rs:166`, `compute_stft`) but never reduces it to a global flux scalar.

**Recommendation:** Move the feature to stratum-dsp. Reasons:
1. The shared STFT is already there; no second decode/FFT pass needed.
2. Essentia's per-frame `Flux()` is energy-normalised in a way that complicates per-band reasoning (each frame's band flux would need to share the same normaliser to be comparable across bands within a frame). Doing it natively in Rust gives full control.
3. Removes a dependency edge: classification can use band-split flux even when Essentia is unavailable.
4. Aligns with the chord-stab plan, which also adds a band-restricted flux primitive to stratum-dsp (`detect_band_onsets`).

The Essentia-side `spectral_flux_mean` is then deprecated (option C below).

## Signal Definition

### Bands

Defaults: **`flux_low` 60–250 Hz, `flux_mid` 250–2000 Hz, `flux_high` 2000–8000 Hz.**

Justification:
- **60 Hz lower bound on `flux_low`:** below kick fundamentals you get rumble and DC, not rhythmic content. The DC bin and the first few sub-bass bins are dominated by mastering choices, not genre.
- **250 Hz split between low and mid:** above kick fundamentals + low harmonics, below where chord fundamentals and bass-pluck partials sit. Same boundary the chord-stab plan uses on its lower edge for the same reason.
- **2 kHz split between mid and high:** above most chord stabs and bass partials (the chord-stab plan caps its mid band at 2 kHz for this reason), below the brightness-band where hi-hats, ride bells, and "warped synth" textures dominate. This is the band where Untitled 27037's rhythmic activity actually lived.
- **8 kHz upper bound:** above this is mostly air-band reverb and mastering polish; flux up here is dominated by lossy-codec artefacts and reverb tails rather than rhythmic events.
- These match perceptual band conventions (low/mid/high splits used in spectral contrast, EBU R128 weighting bands) more than equal-width-octave splits, which would put the mid–high boundary near 1 kHz and miss the chord-stab band entirely.

These should be configurable in `DspConfig` so the validation step can sweep them without code changes.

### Statistic

For each band, compute the **mean half-wave-rectified spectral flux per frame transition over the whole track**, in the same units as the existing Essentia `spectral_flux_mean` *would* be if restricted to that band. This is a single scalar per band.

Considered and rejected: per-band IQR (would mirror Essentia's `spectral_flux_iqr`, but that feature was rated **Validated — some overlap, Low priority** in the prior research — see `genre-classification-improvements.md:840`. Not worth the schema cost upfront).

## Computation

Trivial extension of the existing flux primitive at `stratum-dsp/src/features/onset/spectral_flux.rs:69`. For each band:

```
band_bins[b] = bins whose centre frequency f_k = k * sample_rate / frame_size
              falls inside band b's [f_lo, f_hi)

For each frame transition t = 1..N:
  flux_band[b][t] = sqrt(sum over k in band_bins[b] of max(0, |X[t,k]| - |X[t-1,k]|)^2)

flux_band_mean[b] = mean over t of flux_band[b][t]
```

Half-wave rectification matches the existing primitive (only count magnitude *increases*). Per-frame normalisation as in `spectral_flux.rs:117–132` is applied before the per-band split, so the three band values share a common scale and sum to something close to (but not exactly equal to) the existing all-bins flux.

Cost: O(n_frames × bins_in_band) summed across bands ≈ O(n_frames × total_bins_in_active_range), single-digit-percent overhead on top of the existing STFT pass. Cheaper than the current Essentia-side computation since no Python subprocess overhead.

## Backward Compatibility

Three options for handling the existing `spectral_flux_mean` in the Fisher prototypes (`src/audio_profile.rs:43`, position 11 in `SCALAR_FEATURE_NAMES`):

**(a) Keep `spectral_flux_mean` AND add `flux_low/mid/high`.** Largest schema, simplest migration. Existing 574-track prototypes keep working unchanged. Downside: 14 scalar features instead of 13, with the global flux now redundant against the three bands. Fisher will down-weight it correctly, but it's dead weight.

**(b) Replace `spectral_flux_mean` with `flux_low/mid/high`.** Cleanest end state. Breaks all existing prototypes — every genre's flux feature gets dropped at load time and recalibration becomes mandatory before classification works at all. Recalibration cost is real: 574 tracks × Essentia + stratum-dsp re-analysis (a few hours of wall time on the user's machine, plus user attention to confirm coverage).

**(c) Add `flux_low/mid/high`, mark `spectral_flux_mean` deprecated, remove it in a later release.** Two-phase migration. Both old and new features active simultaneously for one release cycle; old prototypes keep scoring; new prototypes (rebuilt from re-analysed tracks) include both. After the new prototypes prove out empirically, drop `spectral_flux_mean` from `SCALAR_FEATURE_NAMES` and bump `STRATUM_SCHEMA_VERSION` again to evict the now-redundant Essentia field.

**Recommendation: (c).** Reasons:
- Avoids forcing recalibration in lockstep with the schema bump. The 574-track verified set is the user's most expensive asset; a release that breaks classification until recalibration finishes is bad UX.
- Lets the validation step (section 6) compare the new band features against the old global on the *same* prototype set, which is the cleanest A/B test possible.
- The redundancy cost during the deprecation window is one extra `f64` per track — negligible.
- The decision tree at `src/classify.rs:280` (`compute_audio_profile`) doesn't currently consume `spectral_flux_mean` directly, so there's no tree-side coupling to coordinate.

## Schema Migration

1. **`STRATUM_SCHEMA_VERSION`** at `src/audio.rs:60` — bump from `"4"` to `"5"`. This auto-evicts cached `StratumResult`s without the new `flux_low/mid/high` fields via the existing `is_cache_fresh` check (`src/cli/mod.rs:239`, used in all the analysis caches called out at `src/tools/audio_handlers.rs:42, 70, 206, 250` and elsewhere).
2. **`ESSENTIA_SCHEMA_VERSION`** at `src/audio.rs:61` — *not* bumped in phase 1 (Essentia output unchanged). Bumped in phase 2 when `spectral_flux_mean` is removed from the Essentia script.
3. **Genre prototype rows** in SQLite (`src/store.rs:120`, table `genre_audio_profiles`) — keyed by `(genre, feature)`. New feature names `flux_low`, `flux_mid`, `flux_high` simply appear as new rows after recalibration; old `spectral_flux_mean` rows stay until phase 2. No DDL migration needed.
4. **Recalibration trigger:** `calibrate_audio_profiles` MCP tool re-runs against the verified playlist and writes new rows. The user runs this manually after upgrading. Document this in the release notes.
5. **Auto-eviction guard:** in option (c) the prototype loader at `src/audio_profile.rs:631` should be tolerant of *missing* `flux_low/mid/high` rows for an old genre that hasn't been recalibrated yet — Fisher already handles missing per-feature stats via the floor weight, so no special-case code expected.

## Integration Points

### stratum-dsp side

1. **New module:** `stratum-dsp/src/features/band_flux.rs` (~150 LOC). Contains:
   - `pub fn band_split_flux_mean(magnitude_spec_frames: &[Vec<f32>], sample_rate: u32, frame_size: usize, bands: &[(f32, f32)]) -> Result<Vec<f32>, AnalysisError>` — generic over band list so the chord-stab and kick-pattern detectors can share it.
   - Internal helpers: `bin_range_for_band(f_lo, f_hi, sample_rate, frame_size) -> (usize, usize)`.
2. **Module registration:** `stratum-dsp/src/features/mod.rs` — add `pub mod band_flux;`.
3. **Pipeline wiring:** `stratum-dsp/src/lib.rs:1593–1620` (alongside `mod_centroid`, `harmonic_proportion`, `decay`). Reuses `magnitude_spec_frames` already in scope from line 166.
4. **Result struct:** `stratum-dsp/src/analysis/result.rs:185–232` (`AnalysisResult`). Add three fields:
   ```
   #[serde(skip_serializing_if = "Option::is_none")]
   pub flux_low: Option<f32>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub flux_mid: Option<f32>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub flux_high: Option<f32>,
   ```
5. **Config:** add `BandFluxConfig { low_band_hz: (f32, f32), mid_band_hz: (f32, f32), high_band_hz: (f32, f32) }` to `DspConfig` with the recommended defaults.

### reklawdbox side

6. **`StratumResult`:** `src/audio.rs:30–48` — add `pub flux_low: Option<f64>`, `pub flux_mid: Option<f64>`, `pub flux_high: Option<f64>`.
7. **Schema bump:** `src/audio.rs:60` — `STRATUM_SCHEMA_VERSION = "5"`.
8. **Mapping from stratum JSON:** `src/tools/classify_handler.rs` near line 706 — extract the three new fields alongside the existing `decay_mid_tau`/`decay_high_tau`.
9. **`AudioFeatures`:** `src/classify.rs:143` — replace the line `pub(crate) spectral_flux_mean: Option<f64>,` with three fields:
   ```
   #[allow(dead_code)]
   pub(crate) spectral_flux_mean: Option<f64>,  // deprecated, kept for one release
   #[allow(dead_code)]
   pub(crate) flux_low: Option<f64>,
   #[allow(dead_code)]
   pub(crate) flux_mid: Option<f64>,
   #[allow(dead_code)]
   pub(crate) flux_high: Option<f64>,
   ```
10. **Fisher feature list:** `src/audio_profile.rs:32–46` (`SCALAR_FEATURE_NAMES`) — append `"flux_low"`, `"flux_mid"`, `"flux_high"` after `"spectral_flux_mean"`. `src/audio_profile.rs:118–140` (`extract_scalar_features`) — append three matching `finite(audio.flux_*)` lines. The `debug_assert_eq!` at line 134 keeps these in lockstep.
11. **Short-name map:** `src/audio_profile.rs:803` (`short_name`) — add `"flux_low" => "flux_lo"`, `"flux_mid" => "flux_md"`, `"flux_high" => "flux_hi"`.
12. **Test fixtures:** `src/audio_profile.rs:843, 974` and `src/classify.rs:1324` — add the three new `Option<f64>` fields to the test-helper `AudioFeatures` literals (set to `None`).
13. **Prototype training:** the user re-runs the `calibrate_audio_profiles` MCP tool against `genre_verified` after the release. No code change there, but call it out in the release notes.

## Validation

Lighter than the chord-stab plan — this is a band split of an existing computation, not a new detection algorithm. The validation question is **does the band split give better Fisher separation than the global mean?**

### Method

1. Reuse the chord-stab plan's 32-track fixture set (Dub Techno × 8, Deep Techno × 8, Drone Techno × 4, Tech House × 4, off-beat hat × 4, sustained-pad × 4). It already covers the tonal range where flux discrimination matters most.
2. Compute `flux_low`, `flux_mid`, `flux_high`, and the existing `spectral_flux_mean` for each fixture.
3. For each pair of (positive, negative) buckets relevant to A3's stated discriminations (Deep Techno vs Drone Techno; Deep Techno vs Tech House; Deep Techno vs Dub Techno):
   - Compute Fisher's between/within-class variance ratio for the global mean alone.
   - Compute the same ratio for the three-band features used jointly.
4. Plot per-bucket scatter of `flux_low` vs `flux_high` (the most discriminating pair, by hypothesis from the parent doc).
5. Ear-spot-check Untitled 27037 specifically — its `flux_high` should be significantly higher than the other deep/drone fixtures' `flux_high`. If it's not, the band choices are wrong.

### Acceptance criteria

- **Three-band Fisher ratio beats global Fisher ratio by ≥ 1.5×** on at least 2 of the 3 named discriminations (Deep vs Drone, Deep vs Dub, Deep vs Tech House).
- **No band ends up zero-variance across all genres** (would indicate the band is too narrow or the bin alignment is off).
- **Untitled 27037 specifically** has `flux_high > median(flux_high across Drone Techno bucket)` by a clear margin. This was the originating motivation; if the feature can't discriminate this case, it isn't worth the schema bump.

If criteria fail: iterate on band edges (try 100–300 Hz / 300–2500 Hz / 2500–8000 Hz; or perceptual mel-style edges) before declaring the approach broken.

If criteria pass marginally: ship anyway behind a feature flag, since the cost is small and the Fisher down-weighting handles weak features gracefully.

### Validation harness

`stratum-dsp/tests/band_flux_validation.rs` — same pattern as the chord-stab validation harness. Iterate over fixture WAVs, run `analyze_audio`, dump `flux_low/mid/high` per track to a markdown report, assert the Fisher-ratio criterion, commit the report alongside the PR.

## Risks

1. **FFT bin resolution at low frequencies.** At the existing `frame_size = 2048` and `sample_rate = 44.1 kHz`, bin width is **21.5 Hz**. The 60–250 Hz `flux_low` band gets bins ≈ 3..12 — that's **~9 bins**. Borderline-okay for measuring flux (we're summing, not resolving harmonic structure), but each individual kick-band onset has its energy spread across only 2–3 of those bins. **Mitigation options:**
   - Accept it. We're computing a track-mean, which averages out the per-frame quantisation noise. The chord-stab plan and the existing tempogram band-fusion code (`stratum-dsp/src/lib.rs:346–373`) both already use 2048-frame STFTs at sub-band granularity without obvious problems.
   - Configurable: if validation shows the low band is noisy, pad the band slightly (50–280 Hz instead of 60–250 Hz) for ≈ 11 bins.
   - Larger FFT (4096) for *just* the band-flux pass — rejected, defeats the "reuse the shared STFT" point.
2. **Perceptual vs equal-width band choice.** The recommended bands are perceptual (octave-ish in the high range, narrow in the low). Equal-width octaves (e.g. 60–240, 240–960, 960–3840 Hz) would sit lower and might miss the 2–8 kHz brightness rhythm that motivated A3 in the first place. The validation step explicitly tests Untitled 27037's `flux_high` to catch this.
3. **Cross-band correlation.** The three flux values are not independent — a kick onset has energy across all three bands. Fisher handles correlated features fine (down-weighting redundant ones), but if the within-class correlation is *very* high we get less separation than the per-band variance suggests. The validation Fisher ratio measurement captures this.
4. **Per-frame normalisation choice.** The existing flux primitive normalises each frame to its own max before computing flux (`spectral_flux.rs:117–132`). Doing this *before* band-splitting means a track with massive low-end loudness will have its highs normalised down, dampening `flux_high`. Doing it *after* band-splitting (per-band normalisation) makes the bands independently scaled but breaks comparability. **Recommendation: keep the existing pre-split normalisation** — it matches Essentia's behaviour, gives one fewer knob to tune, and the validation step will show whether it loses the Berghain signature on Untitled 27037.
5. **Prototype recalibration UX.** Until the user re-runs `calibrate_audio_profiles`, no genre has `flux_low/mid/high` rows and Fisher contributes nothing for those features. Classification still works (uses the other 13 scalar features + timbral centroids), but the new feature is dead weight on disk until the user acts. Surface this in the release notes and consider a one-time `audit_calibration` warning when prototypes are stale.

## Suggested Implementation Order

Three small PRs:

1. **PR 1** — `band_split_flux_mean` primitive in `stratum-dsp/src/features/band_flux.rs`, unit tests against synthetic input (one sine in low band, one impulse train in high band, etc.). No `AnalysisResult` integration yet.
2. **PR 2** — `AnalysisResult` fields, pipeline wiring at `stratum-dsp/src/lib.rs:1622`, config plumbing, schema bump from `4` to `5`. Re-analysis of cached tracks happens automatically on the next access.
3. **PR 3** — `AudioFeatures` + `audio_profile.rs` Fisher list updates. Run validation harness, commit report. Run user-side recalibration. Wire is otherwise transparent — no decision-tree change since `spectral_flux_mean` was Fisher-only to begin with.

Optional **PR 4** (one release later) — drop `spectral_flux_mean` from Essentia output, `EssentiaOutput`, `AudioFeatures`, and `SCALAR_FEATURE_NAMES`; bump `ESSENTIA_SCHEMA_VERSION` to `"3"`; remove the deprecated rows from `genre_audio_profiles` in a calibration migration.

## Cost Estimate

| Stage | Effort |
|---|---|
| PR 1 (primitive + tests) | 0.5 day |
| PR 2 (result + pipeline + schema) | 0.5 day |
| PR 3 (Fisher integration + validation + recalibration run) | 1 day |
| PR 4 (deprecation cleanup, later release) | 0.5 day |
| **Total** | ~2.5 days, plus user-side recalibration wall time |

Substantially smaller than chord-stab. The work is mostly plumbing, not algorithm design.
