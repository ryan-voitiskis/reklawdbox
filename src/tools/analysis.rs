use rusqlite::Connection;

use crate::{audio, store};

#[derive(Clone, Debug)]
pub(super) struct AudioCacheIdentity {
    pub(super) cache_key: String,
    pub(super) file_size: i64,
    pub(super) file_mtime: i64,
}

impl AudioCacheIdentity {
    pub(super) fn as_store_identity(&self) -> store::AudioAnalysisIdentity<'_> {
        store::AudioAnalysisIdentity {
            file_path: &self.cache_key,
            file_size: self.file_size,
            file_mtime: self.file_mtime,
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
    })
}

pub(super) fn get_fresh_analysis_entry(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<store::CachedAudioAnalysis>, String> {
    let Some(identity) = audio_cache_identity(raw_file_path) else {
        return Ok(None);
    };
    let cached = store::get_audio_analysis(store, &identity.cache_key, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    if store::is_audio_analysis_fresh(
        cached.as_ref(),
        schema_version,
        identity.file_size,
        identity.file_mtime,
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
    let Some(identity) = audio_cache_identity(raw_file_path) else {
        return Ok(None);
    };
    let cached = store::get_audio_analysis(store, &identity.cache_key, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    if store::is_audio_analysis_fresh(
        cached.as_ref(),
        schema_version,
        identity.file_size,
        identity.file_mtime,
    ) {
        Ok(cached.map(|entry| entry.features_json))
    } else {
        Ok(None)
    }
}

pub(super) async fn analyze_stratum(file_path: &str) -> Result<audio::StratumResult, String> {
    let path = file_path.to_string();
    let (samples, sample_rate) =
        tokio::task::spawn_blocking(move || audio::decode_to_samples(&path))
            .await
            .map_err(|e| format!("Decode task failed: {e}"))?
            .map_err(|e| format!("Decode error: {e}"))?;

    let path_for_grid = file_path.to_string();
    tokio::task::spawn_blocking(move || {
        let grid = audio::load_rekordbox_grid_for_path(&path_for_grid);
        audio::analyze_with_stratum(&samples, sample_rate, grid)
    })
    .await
    .map_err(|e| format!("Analysis task failed: {e}"))?
    .map_err(|e| format!("Analysis error: {e}"))
}
