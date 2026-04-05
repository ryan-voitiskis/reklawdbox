import json
import sys
import essentia
import essentia.standard as es
import numpy as np

audio = es.MonoLoader(filename=sys.argv[1], sampleRate=44100)()
features = {}

def first_scalar_or_none(value):
    if value is None:
        return None
    if isinstance(value, (list, tuple)):
        for item in value:
            scalar = first_scalar_or_none(item)
            if scalar is not None:
                return scalar
        return None
    try:
        arr = np.asarray(value)
        if arr.size > 0:
            return float(arr.reshape(-1)[0])
    except Exception:
        pass
    try:
        return float(value)
    except Exception:
        return None

features["danceability"] = first_scalar_or_none(es.Danceability()(audio))

try:
    ebu = es.LoudnessEBUR128()(audio)
except TypeError:
    # LoudnessEBUR128 requires VECTOR_STEREOSAMPLE; MonoLoader produces VECTOR_REAL.
    # StereoMuxer produces the correct native type from two mono channels.
    try:
        stereo_audio = es.StereoMuxer()(audio, audio)
        ebu = es.LoudnessEBUR128()(stereo_audio)
    except Exception:
        ebu = None

if ebu is not None:
    if isinstance(ebu, (tuple, list)):
        # Typical output is (momentary, short_term, integrated, loudness_range).
        # Prefer integrated/range slots when present, otherwise fallback to first two scalars.
        if len(ebu) >= 4:
            integrated = first_scalar_or_none(ebu[2])
            loudness_range = first_scalar_or_none(ebu[3])
        else:
            integrated = None
            loudness_range = None

        if integrated is None or loudness_range is None:
            scalar_values = [first_scalar_or_none(v) for v in ebu]
            scalar_values = [v for v in scalar_values if v is not None]
            if integrated is None:
                integrated = scalar_values[0] if len(scalar_values) > 0 else None
            if loudness_range is None:
                loudness_range = scalar_values[1] if len(scalar_values) > 1 else None

        features["loudness_integrated"] = integrated
        features["loudness_range"] = loudness_range
    else:
        features["loudness_integrated"] = first_scalar_or_none(ebu)
        features["loudness_range"] = None
else:
    features["loudness_integrated"] = None
    features["loudness_range"] = None

features["dynamic_complexity"] = first_scalar_or_none(es.DynamicComplexity()(audio))
features["average_loudness"] = first_scalar_or_none(es.Loudness()(audio))

rhythm = es.RhythmExtractor2013(method="multifeature")(audio)
features["bpm_essentia"] = first_scalar_or_none(rhythm[0] if isinstance(rhythm, (tuple, list)) and len(rhythm) > 0 else rhythm)
onset_result = es.OnsetRate()(audio)
if isinstance(onset_result, (tuple, list)) and len(onset_result) >= 2:
    features["onset_rate"] = first_scalar_or_none(onset_result[1])
else:
    features["onset_rate"] = first_scalar_or_none(onset_result)

beats = rhythm[1]
if len(beats) > 4:
    bl = es.BeatsLoudness(beats=beats)(audio)
    band_ratios = bl[1]
    if len(band_ratios) > 0:
        downbeat_values = [band_ratios[i][0] for i in range(0, len(band_ratios), 4)]
        all_values = [row[0] for row in band_ratios]
        downbeat_energy = sum(downbeat_values) / max(len(downbeat_values), 1)
        all_energy = sum(all_values) / max(len(all_values), 1)
        features["rhythm_regularity"] = float(downbeat_energy / max(all_energy, 1e-6))
    else:
        features["rhythm_regularity"] = None
else:
    features["rhythm_regularity"] = None

features["spectral_centroid_mean"] = first_scalar_or_none(es.SpectralCentroidTime()(audio))

# --- Frame-based features (shared loop) ---
frame_size = 2048
hop_size = 1024
windowing = es.Windowing(type='hann')
spectrum_algo = es.Spectrum()
mfcc_algo = es.MFCC(numberCoefficients=13)
centroid_algo = es.Centroid(range=44100/2)
flux_algo = es.Flux()
contrast_algo = es.SpectralContrast()
peaks_algo = es.SpectralPeaks()
dissonance_algo = es.Dissonance()

try:
    intensity_algo = es.Intensity()
    has_intensity = True
except Exception:
    has_intensity = False

mfcc_accum = []
centroid_accum = []
flux_accum = []
contrast_accum = []
dissonance_accum = []
intensity_values = []

for frame in es.FrameGenerator(audio, frameSize=frame_size, hopSize=hop_size):
    windowed = windowing(frame)
    spec = spectrum_algo(windowed)

    _, mfcc_coeffs = mfcc_algo(spec)
    mfcc_accum.append(mfcc_coeffs)

    centroid_val = first_scalar_or_none(centroid_algo(spec))
    if centroid_val is not None:
        centroid_accum.append(centroid_val)

    frame_energy = float(np.sum(np.asarray(spec) ** 2))
    raw_flux = first_scalar_or_none(flux_algo(spec))
    if raw_flux is not None and frame_energy > 1e-10:
        flux_accum.append(raw_flux / frame_energy)

    sc = contrast_algo(spec)
    if isinstance(sc, (tuple, list)) and len(sc) >= 1:
        coeffs = sc[0]
        if hasattr(coeffs, '__len__') and len(coeffs) > 0:
            contrast_accum.append([float(x) for x in coeffs])

    freqs, mags = peaks_algo(spec)
    if len(freqs) > 1:
        diss = dissonance_algo(freqs, mags)
        diss_val = first_scalar_or_none(diss)
        if diss_val is not None:
            dissonance_accum.append(diss_val)

    if has_intensity:
        try:
            power_spec = [x**2 for x in spec]
            try:
                val = intensity_algo(power_spec)
            except TypeError:
                val = intensity_algo(frame)
            int_val = first_scalar_or_none(val)
            if int_val is not None:
                intensity_values.append(int_val)
        except Exception:
            pass

if mfcc_accum:
    mfcc_arr = np.array(mfcc_accum)
    features["mfcc_mean"] = [float(x) for x in np.mean(mfcc_arr, axis=0)]
    features["mfcc_std"] = [float(x) for x in np.std(mfcc_arr, axis=0)]
else:
    features["mfcc_mean"] = None
    features["mfcc_std"] = None

if centroid_accum:
    c_arr = np.array(centroid_accum)
    c_mean = float(np.mean(c_arr))
    c_std = float(np.std(c_arr))
    features["spectral_centroid_cv"] = c_std / c_mean if c_mean > 1e-10 else None
else:
    features["spectral_centroid_cv"] = None

if flux_accum:
    f_arr = np.sort(np.array(flux_accum))
    features["spectral_flux_mean"] = float(np.mean(f_arr))
    q1 = float(np.percentile(f_arr, 25))
    q3 = float(np.percentile(f_arr, 75))
    features["spectral_flux_iqr"] = q3 - q1
else:
    features["spectral_flux_mean"] = None
    features["spectral_flux_iqr"] = None

if contrast_accum:
    n_bands = len(contrast_accum[0])
    features["spectral_contrast_mean"] = [sum(c[i] for c in contrast_accum) / len(contrast_accum) for i in range(n_bands)]
else:
    features["spectral_contrast_mean"] = None

if dissonance_accum:
    features["dissonance_mean"] = sum(dissonance_accum) / len(dissonance_accum)
else:
    features["dissonance_mean"] = None

if intensity_values:
    features["intensity_mean"] = sum(intensity_values) / len(intensity_values)
    mean = features["intensity_mean"]
    features["intensity_var"] = sum((v - mean)**2 for v in intensity_values) / len(intensity_values)
else:
    features["intensity_mean"] = None
    features["intensity_var"] = None

features["analyzer_version"] = essentia.__version__

json.dump(features, sys.stdout)
