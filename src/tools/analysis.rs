use rusqlite::Connection;

use crate::{audio, store};

pub(super) fn resolved_audio_cache_key(raw_file_path: &str) -> String {
    super::resolve_file_path(raw_file_path).unwrap_or_else(|_| raw_file_path.to_string())
}

pub(super) fn get_fresh_analysis_entry(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<store::CachedAudioAnalysis>, String> {
    let file_path = resolved_audio_cache_key(raw_file_path);
    let cached = store::get_audio_analysis(store, &file_path, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    match cached {
        Some(entry) if entry.analysis_version == schema_version => Ok(Some(entry)),
        _ => Ok(None),
    }
}

pub(super) fn check_analysis_cache(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<String>, String> {
    let file_path = resolved_audio_cache_key(raw_file_path);
    let cached = store::get_audio_analysis(store, &file_path, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    match cached {
        Some(entry) if entry.analysis_version == schema_version => Ok(Some(entry.features_json)),
        _ => Ok(None),
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
