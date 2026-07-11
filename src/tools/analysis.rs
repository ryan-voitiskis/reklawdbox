use rusqlite::Connection;
use std::collections::HashMap;

use crate::{audio, store};

#[derive(Clone, Debug)]
pub(super) struct AudioCacheIdentity {
    pub(super) cache_key: String,
    pub(super) file_size: i64,
    pub(super) file_mtime: i64,
    pub(super) stratum_input_fingerprint: Option<String>,
}

impl AudioCacheIdentity {
    pub(super) fn as_stratum_store_identity(&self) -> Option<store::AudioAnalysisIdentity<'_>> {
        Some(store::AudioAnalysisIdentity {
            file_path: &self.cache_key,
            file_size: self.file_size,
            file_mtime: self.file_mtime,
            input_fingerprint: self.stratum_input_fingerprint.as_deref()?,
        })
    }

    pub(super) fn as_essentia_store_identity(&self) -> store::AudioAnalysisIdentity<'_> {
        store::AudioAnalysisIdentity {
            file_path: &self.cache_key,
            file_size: self.file_size,
            file_mtime: self.file_mtime,
            input_fingerprint: "",
        }
    }
}

fn file_mtime_unix(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

pub(super) fn resolved_audio_cache_key(raw_file_path: &str) -> String {
    super::resolve_file_path(raw_file_path).unwrap_or_else(|_| raw_file_path.to_string())
}

pub(super) fn audio_cache_identity(raw_file_path: &str) -> Option<AudioCacheIdentity> {
    let cache_key = super::resolve_file_path(raw_file_path).ok()?;
    let metadata = std::fs::metadata(&cache_key).ok()?;
    Some(AudioCacheIdentity {
        cache_key,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
        stratum_input_fingerprint: None,
    })
}

pub(super) fn audio_cache_identities_with_fingerprint_loader<'a>(
    raw_file_paths: impl IntoIterator<Item = &'a str>,
    mut load_fingerprint: impl FnMut(&str) -> String,
) -> Vec<Option<AudioCacheIdentity>> {
    let mut fingerprints_by_cache_key = HashMap::new();
    raw_file_paths
        .into_iter()
        .map(|raw_file_path| {
            let mut identity = audio_cache_identity(raw_file_path)?;
            let fingerprint = fingerprints_by_cache_key
                .entry(identity.cache_key.clone())
                .or_insert_with(|| load_fingerprint(&identity.cache_key))
                .clone();
            identity.stratum_input_fingerprint = Some(fingerprint);
            Some(identity)
        })
        .collect()
}

#[cfg(test)]
pub(super) fn audio_cache_identity_with_stratum_input_fingerprint(
    raw_file_path: &str,
    input_fingerprint: impl Into<String>,
) -> Option<AudioCacheIdentity> {
    let mut identity = audio_cache_identity(raw_file_path)?;
    identity.stratum_input_fingerprint = Some(input_fingerprint.into());
    Some(identity)
}

pub(super) fn audio_cache_identities_with_current_stratum_input<'a>(
    raw_file_paths: impl IntoIterator<Item = &'a str>,
) -> Vec<Option<AudioCacheIdentity>> {
    audio_cache_identities_with_fingerprint_loader(raw_file_paths, |cache_key| {
        audio::load_rekordbox_grid_input_for_path(cache_key).fingerprint
    })
}

fn audio_cache_identity_for_analyzer(
    raw_file_path: &str,
    analyzer: &str,
) -> Option<AudioCacheIdentity> {
    if analyzer == audio::ANALYZER_STRATUM {
        audio_cache_identities_with_current_stratum_input([raw_file_path])
            .into_iter()
            .next()
            .flatten()
    } else {
        audio_cache_identity(raw_file_path)
    }
}

pub(super) fn get_fresh_analysis_entry(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<store::CachedAudioAnalysis>, String> {
    let Some(identity) = audio_cache_identity_for_analyzer(raw_file_path, analyzer) else {
        return Ok(None);
    };
    get_fresh_analysis_entry_for_identity(store, &identity, analyzer, schema_version)
}

pub(super) fn get_fresh_analysis_entry_for_identity(
    store: &Connection,
    identity: &AudioCacheIdentity,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<store::CachedAudioAnalysis>, String> {
    let store_identity = if analyzer == audio::ANALYZER_STRATUM {
        identity
            .as_stratum_store_identity()
            .ok_or_else(|| "Stratum cache identity is missing its input fingerprint".to_string())?
    } else {
        identity.as_essentia_store_identity()
    };
    let cached = store::get_audio_analysis(store, &identity.cache_key, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    if store::is_audio_analysis_fresh(
        cached.as_ref(),
        schema_version,
        store_identity.file_size,
        store_identity.file_mtime,
        store_identity.input_fingerprint,
    ) {
        Ok(cached)
    } else {
        Ok(None)
    }
}

pub(super) fn check_analysis_cache(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<String>, String> {
    let Some(identity) = audio_cache_identity_for_analyzer(raw_file_path, analyzer) else {
        return Ok(None);
    };
    check_analysis_cache_for_identity(store, &identity, analyzer, schema_version)
}

pub(super) fn check_analysis_cache_for_identity(
    store: &Connection,
    identity: &AudioCacheIdentity,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<String>, String> {
    Ok(
        get_fresh_analysis_entry_for_identity(store, identity, analyzer, schema_version)?
            .map(|entry| entry.features_json),
    )
}

pub(super) async fn analyze_stratum(file_path: &str) -> Result<audio::StratumAnalysis, String> {
    let path = file_path.to_string();
    let (samples, sample_rate) =
        tokio::task::spawn_blocking(move || audio::decode_to_samples(&path))
            .await
            .map_err(|e| format!("Decode task failed: {e}"))?
            .map_err(|e| format!("Decode error: {e}"))?;

    let path_for_grid = file_path.to_string();
    tokio::task::spawn_blocking(move || {
        let input = audio::load_rekordbox_grid_input_for_path(&path_for_grid);
        audio::analyze_with_stratum_input(&samples, sample_rate, input)
    })
    .await
    .map_err(|e| format!("Analysis task failed: {e}"))?
    .map_err(|e| format!("Analysis error: {e}"))
}
