# Spec: Essentia Script v2 — Additional Features for Set Building

## Context

reklawdbox runs Essentia via a Python subprocess (`audio::run_essentia` in `src/audio.rs`). The inline Python script (`ESSENTIA_SCRIPT` constant, line 32) extracts audio features and outputs JSON to stdout. Results are cached in the internal SQLite store keyed by `(file_path, "essentia")`.

The current script extracts 10 features. This spec adds 4 new feature groups (MFCCs, SpectralContrast, Dissonance, Intensity) that improve transition scoring and set building.

## Current Features (keep as-is)

```
danceability          float    Danceability score (0-3 range)
loudness_integrated   float    EBU R128 integrated loudness (LUFS)
loudness_range        float    EBU R128 loudness range (LU)
dynamic_complexity    float    Dynamic complexity score
average_loudness      float    Average perceived loudness
bpm_essentia          float    BPM (cross-reference with stratum-dsp)
onset_rate            float    Transient density (onsets per second)
rhythm_regularity     float    Downbeat-to-all-beats energy ratio
spectral_centroid_mean float   Average brightness (Hz)
analyzer_version      string   Essentia version string
```

## New Features to Add

### 1. MFCCs (Timbral Fingerprint)

**What:** 13-coefficient Mel-Frequency Cepstral Coefficients, averaged across all frames. MFCCs are the standard representation for "what does this track sound like" regardless of genre labels.

**Essentia pipeline:**
```python
# Frame-based: need windowed spectrum → mel bands → MFCC
frame_size = 2048
hop_size = 1024
frames = es.FrameGenerator(audio, frameSize=frame_size, hopSize=hop_size)
mfcc_extractor = es.MFCC(numberCoefficients=13)
windowing = es.Windowing(type='hann')
spectrum = es.Spectrum()

mfcc_accum = []
for frame in frames:
    windowed = windowing(frame)
    spec = spectrum(windowed)
    _, mfcc_coeffs = mfcc_extractor(spec)
    mfcc_accum.append(mfcc_coeffs)

if mfcc_accum:
    mfcc_mean = [float(sum(c[i] for c in mfcc_accum) / len(mfcc_accum)) for i in range(13)]
else:
    mfcc_mean = None
features["mfcc_mean"] = mfcc_mean
```

**Output:** `"mfcc_mean": [c0, c1, ..., c12]` — array of 13 floats, or null.

**Scoring use:** Cosine distance between two tracks' MFCC vectors = timbral similarity. Low distance = tracks sound alike. This can improve `score_genre_axis` as a fallback when genre labels are missing, or become a new `timbral_similarity` axis in `score_transition_profiles`.

### 2. SpectralContrast

**What:** Measures the difference between spectral peaks and valleys in each frequency band. High contrast = clear tonal content (melodies, strong harmonics). Low contrast = noise-like or ambient texture. Captures dense-vs-sparse better than spectral centroid alone.

**Essentia pipeline:**
```python
contrast_accum = []
for frame in frames:  # reuse same FrameGenerator loop as MFCC
    windowed = windowing(frame)
    spec = spectrum(windowed)
    sc = es.SpectralContrast()(spec)
    # sc returns (contrast_coeffs, spectral_valley)
    if isinstance(sc, (tuple, list)) and len(sc) >= 1:
        coeffs = sc[0]
        if hasattr(coeffs, '__len__') and len(coeffs) > 0:
            contrast_accum.append([float(x) for x in coeffs])

if contrast_accum:
    n_bands = len(contrast_accum[0])
    contrast_mean = [sum(c[i] for c in contrast_accum) / len(contrast_accum) for i in range(n_bands)]
else:
    contrast_mean = None
features["spectral_contrast_mean"] = contrast_mean
```

**Output:** `"spectral_contrast_mean": [band0, band1, ..., band5]` — array of 6 floats (default band count), or null.

**Scoring use:** Euclidean distance between spectral contrast vectors. Large distance = mixing a dense track into a sparse one (or vice versa), which is risky. Could be a penalty factor on the composite score, or a new `texture` axis.

### 3. Dissonance

**What:** Measures harmonic roughness / sensory dissonance from spectral peaks. High dissonance = harsh, aggressive, industrial. Low = clean, consonant, smooth. Complements key compatibility — two tracks in compatible keys can still clash if one is highly dissonant.

**Essentia pipeline:**
```python
dissonance_accum = []
spectral_peaks = es.SpectralPeaks()
dissonance_algo = es.Dissonance()
for frame in frames:  # reuse same loop
    windowed = windowing(frame)
    spec = spectrum(windowed)
    freqs, mags = spectral_peaks(spec)
    if len(freqs) > 1:
        diss = dissonance_algo(freqs, mags)
        diss_val = first_scalar_or_none(diss)
        if diss_val is not None:
            dissonance_accum.append(diss_val)

if dissonance_accum:
    features["dissonance_mean"] = sum(dissonance_accum) / len(dissonance_accum)
else:
    features["dissonance_mean"] = None
```

**Output:** `"dissonance_mean": float` — average dissonance (0-1 range), or null.

**Scoring use:** Dissonance delta between two tracks. Large increase in dissonance across a transition sounds harsh. Could multiply the key axis score: if dissonance delta > threshold, penalize even key-compatible transitions. Or become a standalone `harmonic_roughness` axis.

### 4. Intensity

**What:** Classifies audio frames as relaxed/moderate/aggressive/... based on dynamics, loudness, and spectral shape. More perceptually meaningful than raw loudness for modeling felt energy.

**Essentia pipeline:**
```python
intensity = es.Intensity()
intensity_values = []
for frame in frames:  # reuse same loop
    windowed = windowing(frame)
    spec = spectrum(windowed)
    # Intensity expects the power spectrum
    power_spec = [x**2 for x in spec]  # or use es.PowerSpectrum()
    val = intensity(power_spec)
    int_val = first_scalar_or_none(val)
    if int_val is not None:
        intensity_values.append(int_val)

if intensity_values:
    features["intensity_mean"] = sum(intensity_values) / len(intensity_values)
    # Also useful: intensity variance shows how much the energy fluctuates
    mean = features["intensity_mean"]
    features["intensity_var"] = sum((v - mean)**2 for v in intensity_values) / len(intensity_values)
else:
    features["intensity_mean"] = None
    features["intensity_var"] = None
```

**Output:** `"intensity_mean": float`, `"intensity_var": float` — or null.

**Scoring use:** Direct replacement/supplement for the energy proxy in `compute_track_energy`. Intensity mean is perceived energy level; intensity variance indicates builds/drops within the track (high variance = dynamic, low = flat). Could improve `score_energy_axis` and energy phase classification.

**Important:** Check the exact Intensity API — it may operate on the audio signal directly rather than on a spectrum. Consult the [Essentia docs for Intensity](https://essentia.upf.edu/reference/std_Intensity.html). If it takes the audio signal directly (not spectrum), simplify to `es.Intensity()(audio)` and extract the per-frame or aggregated result.

## Implementation Notes

### Shared Frame Loop

All 4 new features are frame-based. The current script runs each algorithm on the full audio signal independently. The new features should share a single `FrameGenerator` loop to avoid redundant computation:

```python
# Single pass over frames
frame_size = 2048
hop_size = 1024
windowing = es.Windowing(type='hann')
spectrum_algo = es.Spectrum()
mfcc_algo = es.MFCC(numberCoefficients=13)
contrast_algo = es.SpectralContrast()
peaks_algo = es.SpectralPeaks()
dissonance_algo = es.Dissonance()

mfcc_accum = []
contrast_accum = []
dissonance_accum = []

for frame in es.FrameGenerator(audio, frameSize=frame_size, hopSize=hop_size):
    windowed = windowing(frame)
    spec = spectrum_algo(windowed)

    # MFCC
    _, mfcc_coeffs = mfcc_algo(spec)
    mfcc_accum.append(mfcc_coeffs)

    # SpectralContrast
    sc = contrast_algo(spec)
    ...

    # Dissonance
    freqs, mags = peaks_algo(spec)
    if len(freqs) > 1:
        ...

# Aggregate after loop
```

This is significantly faster than 4 separate passes. The existing full-signal algorithms (Danceability, LoudnessEBUR128, etc.) remain outside this loop since they operate on the complete audio array.

### Output Schema

The essentia JSON blob stored in `audio_analysis_cache.features_json` is schemaless — it's just a JSON object with string keys. New fields are additive. No schema migration needed. Old cached entries simply won't have the new fields; code consuming them must handle `None`/missing gracefully (as it already does for all essentia fields).

### Cache Invalidation

Tracks analyzed before this change will have cached essentia results WITHOUT the new fields. Two options:

1. **Lazy approach (recommended):** Treat missing fields as null. When scoring, if `mfcc_mean` is null for either track, skip the timbral similarity axis. Over time, re-analysis (via `--no-skip-cached` or new tracks) fills in the data.

2. **Forced re-analysis:** Run `reklawdbox analyze --no-skip-cached` to regenerate all essentia results. Only needed if you want 100% coverage immediately.

### Performance Impact

The frame loop adds CPU time per track. Rough estimates:
- MFCC: ~10% overhead (lightweight per-frame FFT + mel filterbank)
- SpectralContrast: ~5% (reuses same spectrum)
- Dissonance: ~15% (SpectralPeaks is moderately expensive)
- Intensity: ~5% (depends on exact API)

Total: ~35% increase in essentia processing time. For a 5-minute track currently taking ~10s with essentia, expect ~13-14s. Acceptable for batch analysis.

### Test Plan

1. **Unit test the Python script** in the existing essentia mock test (`run_essentia_handles_non_scalar_outputs_via_stereo_fallback` in `src/audio.rs`). Add fake implementations of MFCC, SpectralContrast, SpectralPeaks, Dissonance, and Intensity to the mock `essentia/standard.py`. Verify the new fields appear in the output JSON.

2. **Integration test** with a real audio file (`--ignored` tests). Verify all new fields are present and have sane values (MFCC array length = 13, spectral contrast array length = 6, dissonance in 0-1, intensity > 0).

3. **Cache round-trip.** Analyze a track, verify new fields are stored in the cache and survive a read-back.

4. **Backward compat.** Verify that `resolve_tracks_data`, `cache_coverage`, and `score_transition` still work when essentia cache entries lack the new fields (i.e., old cached data).

## Files to Change

| File | Change |
|------|--------|
| `src/audio.rs` | Update `ESSENTIA_SCRIPT` constant with new algorithms and shared frame loop |
| `src/audio.rs` | Update mock essentia test to cover new features |
| `src/tools.rs` | No changes required in phase 1 (scoring integration is phase 2) |

## Phase 2 (Separate Spec): Scoring Integration

After the new features are extracted and cached across the library:

1. Add `mfcc_mean`, `spectral_contrast_mean`, `dissonance_mean`, `intensity_mean`, `intensity_var` to `TrackProfile`
2. Add `timbral_similarity` axis to `score_transition_profiles` using MFCC cosine distance
3. Add `texture` axis using SpectralContrast euclidean distance
4. Integrate `dissonance_mean` as a modifier on the `key` axis
5. Replace or supplement `compute_track_energy` with `intensity_mean`
6. Update `composite_score` weights to include new axes
7. Update `build_set` to use new axes in candidate ranking

This is a separate piece of work that should be planned after the library is re-analyzed with the new features and the data can be explored.
