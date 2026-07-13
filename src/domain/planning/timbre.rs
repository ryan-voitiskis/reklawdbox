//! Timbral-vector construction and normalization policy.

use super::{TimbralFeatures, TimbralNormalization, TrackProfile};

/// Bump whenever timbral vector membership, order, or inclusion rules change.
pub(crate) const TIMBRAL_VECTOR_SCHEMA_VERSION: &str = "1";

fn assemble_timbral_vector(
    mfcc_mean: &[f64],
    mfcc_std: &[f64],
    spectral_contrast: &[f64],
    centroid_cv: f64,
    dissonance: f64,
) -> Vec<f64> {
    let mut vector =
        Vec::with_capacity(mfcc_mean.len() + mfcc_std.len() + spectral_contrast.len() + 2);
    vector.extend_from_slice(mfcc_mean);
    vector.extend_from_slice(mfcc_std);
    vector.extend_from_slice(spectral_contrast);
    vector.push(centroid_cv);
    vector.push(dissonance);
    vector
}

pub(crate) fn build_timbral_vector(profile: &TrackProfile) -> Option<Vec<f64>> {
    profile.timbral.as_ref().map(build_timbral_features_vector)
}

pub(crate) fn build_timbral_features_vector(features: &TimbralFeatures) -> Vec<f64> {
    assemble_timbral_vector(
        &features.mfcc_mean,
        &features.mfcc_std,
        &features.spectral_contrast_mean,
        features.spectral_centroid_cv,
        features.dissonance_mean,
    )
}

pub(crate) fn compute_timbral_normalization(
    vectors: &[Vec<f64>],
) -> Result<TimbralNormalization, String> {
    let count = i64::try_from(vectors.len())
        .map_err(|_| "Too many Essentia entries to compute normalization stats".to_string())?;
    if count < 2 {
        return Err("Need at least 2 Essentia entries to compute normalization stats".to_string());
    }

    let dimensions = vectors[0].len();
    let mut means = vec![0.0; dimensions];
    let mut m2s = vec![0.0; dimensions];

    for (row_index, vector) in vectors.iter().enumerate() {
        let sample_count = (row_index + 1) as f64;
        // Welford's online update
        for (i, &x) in vector.iter().enumerate() {
            let delta = x - means[i];
            means[i] += delta / sample_count;
            let delta2 = x - means[i];
            m2s[i] += delta * delta2;
        }
    }

    let dims: Vec<(f64, f64)> = means
        .iter()
        .zip(m2s.iter())
        .map(|(&mean, &m2)| (mean, (m2 / (count - 1) as f64).sqrt().max(1e-10)))
        .collect();

    Ok(TimbralNormalization {
        dims,
        sample_count: count,
    })
}

pub(crate) fn normalize_timbral_vector(
    raw: &[f64],
    stats: &TimbralNormalization,
) -> Option<Vec<f64>> {
    if raw.len() != stats.dims.len() {
        return None;
    }
    Some(
        raw.iter()
            .zip(stats.dims.iter())
            .map(|(&x, &(mean, stddev))| (x - mean) / stddev)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_timbral_vector_preserves_five_component_order() {
        let features = TimbralFeatures {
            mfcc_mean: vec![1.0],
            mfcc_std: vec![2.0],
            spectral_contrast_mean: vec![3.0],
            spectral_centroid_cv: 4.0,
            dissonance_mean: 5.0,
        };
        assert_eq!(
            build_timbral_features_vector(&features),
            vec![1.0, 2.0, 3.0, 4.0, 5.0]
        );
    }
}
