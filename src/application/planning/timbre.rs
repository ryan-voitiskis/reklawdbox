//! Timbral snapshot and normalization orchestration.

use rusqlite::Connection;
use sha2::{Digest as _, Sha256};

use crate::adapters::state::analysis as state_analysis;
use crate::application::analysis::identity::audio_cache_identity;
use crate::domain::planning::{
    TIMBRAL_VECTOR_SCHEMA_VERSION, TimbralFeatures, TimbralNormalization,
    build_timbral_features_vector, compute_timbral_normalization,
};

#[derive(Debug)]
pub(crate) struct TimbralSourceSnapshot {
    pub(crate) vectors: Vec<Vec<f64>>,
    pub(crate) source_fingerprint: String,
}

fn hash_length_delimited(hasher: &mut Sha256, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("hash input length fits in u64");
    hasher.update(length.to_be_bytes());
    hasher.update(value);
}

fn timbral_features_from_essentia(
    essentia: &crate::adapters::audio::EssentiaOutput,
) -> Option<TimbralFeatures> {
    Some(TimbralFeatures {
        mfcc_mean: essentia.mfcc_mean.clone()?,
        mfcc_std: essentia.mfcc_std.clone()?,
        spectral_contrast_mean: essentia.spectral_contrast_mean.clone()?,
        spectral_centroid_cv: essentia.spectral_centroid_cv?,
        dissonance_mean: essentia.dissonance_mean?,
    })
}

fn load_timbral_source_snapshot_with_vector_schema(
    store_conn: &Connection,
) -> Result<TimbralSourceSnapshot, String> {
    load_timbral_source_snapshot_for_schema(store_conn, TIMBRAL_VECTOR_SCHEMA_VERSION)
}

fn load_timbral_source_snapshot_for_schema(
    store_conn: &Connection,
    vector_schema_version: &str,
) -> Result<TimbralSourceSnapshot, String> {
    let rows = state_analysis::load_timbral_source_rows(
        store_conn,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
    )
    .map_err(|error| format!("DB error: {error}"))?;

    let mut hasher = Sha256::new();
    hash_length_delimited(&mut hasher, vector_schema_version.as_bytes());
    hash_length_delimited(
        &mut hasher,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION.as_bytes(),
    );
    let mut vectors = Vec::new();
    let mut expected_dimensions = None;

    for row in rows {
        let Some(identity) = audio_cache_identity(&row.file_path) else {
            continue;
        };
        if identity.cache_key != row.file_path {
            continue;
        }
        let cached = state_analysis::CachedAudioAnalysis {
            file_path: row.file_path.clone(),
            analyzer: crate::adapters::audio::ANALYZER_ESSENTIA.to_string(),
            file_size: row.file_size,
            file_mtime: row.file_mtime,
            analysis_version: crate::adapters::audio::ESSENTIA_SCHEMA_VERSION.to_string(),
            input_fingerprint: row.input_fingerprint,
            features_json: row.features_json.clone(),
            created_at: String::new(),
        };
        if !state_analysis::is_audio_analysis_fresh(
            Some(&cached),
            crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
            identity.file_size,
            identity.file_mtime,
            "",
        ) {
            continue;
        }

        let essentia: crate::adapters::audio::EssentiaOutput =
            match serde_json::from_str(&row.features_json) {
                Ok(essentia) => essentia,
                Err(_) => continue,
            };
        let Some(features) = timbral_features_from_essentia(&essentia) else {
            continue;
        };
        let vector = build_timbral_features_vector(&features);
        if !vector.iter().all(|value| value.is_finite()) {
            continue;
        }
        let dimensions = *expected_dimensions.get_or_insert(vector.len());
        if vector.len() != dimensions {
            continue;
        }

        hash_length_delimited(&mut hasher, row.file_path.as_bytes());
        hash_length_delimited(&mut hasher, &row.file_size.to_be_bytes());
        hash_length_delimited(&mut hasher, &row.file_mtime.to_be_bytes());
        hash_length_delimited(&mut hasher, row.features_json.as_bytes());
        vectors.push(vector);
    }

    Ok(TimbralSourceSnapshot {
        vectors,
        source_fingerprint: format!("{:x}", hasher.finalize()),
    })
}

pub(crate) fn load_timbral_source_snapshot(
    store_conn: &Connection,
) -> Result<TimbralSourceSnapshot, String> {
    load_timbral_source_snapshot_with_vector_schema(store_conn)
}

#[cfg(test)]
pub(crate) fn load_timbral_source_snapshot_for_test(
    store_conn: &Connection,
    vector_schema_version: &str,
) -> Result<TimbralSourceSnapshot, String> {
    load_timbral_source_snapshot_for_schema(store_conn, vector_schema_version)
}

fn persisted_stats(
    normalization: TimbralNormalization,
    snapshot: &TimbralSourceSnapshot,
) -> state_analysis::TimbralNormStats {
    state_analysis::TimbralNormStats {
        dims: normalization.dims,
        sample_count: normalization.sample_count,
        source_fingerprint: snapshot.source_fingerprint.clone(),
        analysis_version: crate::adapters::audio::ESSENTIA_SCHEMA_VERSION.to_string(),
        vector_schema_version: TIMBRAL_VECTOR_SCHEMA_VERSION.to_string(),
    }
}

fn domain_normalization(stats: &state_analysis::TimbralNormStats) -> TimbralNormalization {
    TimbralNormalization {
        dims: stats.dims.clone(),
        sample_count: stats.sample_count,
    }
}

fn compute_timbral_norm_stats_from_snapshot(
    snapshot: &TimbralSourceSnapshot,
) -> Result<state_analysis::TimbralNormStats, String> {
    let normalization = compute_timbral_normalization(&snapshot.vectors)?;
    Ok(persisted_stats(normalization, snapshot))
}

#[cfg(test)]
pub(crate) fn compute_timbral_norm_stats(
    store_conn: &Connection,
) -> Result<state_analysis::TimbralNormStats, String> {
    let snapshot = load_timbral_source_snapshot(store_conn)?;
    compute_timbral_norm_stats_from_snapshot(&snapshot)
}

pub(crate) fn ensure_timbral_norm_stats(
    store_conn: &Connection,
) -> Result<Option<state_analysis::TimbralNormStats>, String> {
    let snapshot = load_timbral_source_snapshot(store_conn)?;
    if snapshot.vectors.len() < 2 {
        state_analysis::clear_timbral_norm_stats(store_conn)
            .map_err(|error| format!("Failed to clear norm stats: {error}"))?;
        return Ok(None);
    }

    let existing = state_analysis::get_timbral_norm_stats(store_conn)
        .map_err(|error| format!("DB error: {error}"))?;

    if let Some(stats) = existing {
        let expected_dimensions = snapshot.vectors[0].len();
        let provenance_matches = stats.source_fingerprint == snapshot.source_fingerprint
            && stats.analysis_version == crate::adapters::audio::ESSENTIA_SCHEMA_VERSION
            && stats.vector_schema_version == TIMBRAL_VECTOR_SCHEMA_VERSION;
        let dimensions_are_coherent = stats.dims.len() == expected_dimensions
            && stats
                .dims
                .iter()
                .all(|(mean, stddev)| mean.is_finite() && stddev.is_finite() && *stddev > 0.0);
        let sample_count_matches = usize::try_from(stats.sample_count)
            .is_ok_and(|sample_count| sample_count == snapshot.vectors.len());
        if provenance_matches && dimensions_are_coherent && sample_count_matches {
            return Ok(Some(stats));
        }
    }

    let stats = compute_timbral_norm_stats_from_snapshot(&snapshot)?;
    state_analysis::save_timbral_norm_stats(store_conn, &stats)
        .map_err(|error| format!("Failed to save norm stats: {error}"))?;
    Ok(Some(stats))
}

pub(crate) fn normalization_from_persisted(
    stats: &state_analysis::TimbralNormStats,
) -> TimbralNormalization {
    domain_normalization(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_timbral_snapshot_round_trip_preserves_schema() {
        let snapshot = TimbralSourceSnapshot {
            vectors: vec![vec![1.0, 2.0], vec![3.0, 4.0]],
            source_fingerprint: "snapshot".to_string(),
        };
        let persisted = compute_timbral_norm_stats_from_snapshot(&snapshot).unwrap();
        let round_trip = domain_normalization(&persisted);
        assert_eq!(
            persisted.vector_schema_version,
            TIMBRAL_VECTOR_SCHEMA_VERSION
        );
        assert_eq!(round_trip.sample_count, persisted.sample_count);
        assert_eq!(round_trip.dims, persisted.dims);
    }
}
