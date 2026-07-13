use std::collections::HashMap;

use rusqlite::Connection;

use crate::adapters::audio;
#[cfg(not(test))]
use crate::adapters::rekordbox::{self as rekordbox, anlz as rekordbox_anlz};
use crate::adapters::state::analysis as state_analysis;

use super::model::{RekordboxGridInput, StratumAnalysis};

impl RekordboxGridInput {
    pub fn from_grid(grid: Option<stratum_dsp::BeatGrid>) -> Self {
        let fingerprint = grid.as_ref().map_or_else(
            || audio::STRATUM_HMM_INPUT_FINGERPRINT.to_string(),
            fingerprint_rekordbox_grid,
        );
        Self { grid, fingerprint }
    }
}

fn fingerprint_rekordbox_grid(grid: &stratum_dsp::BeatGrid) -> String {
    use sha2::{Digest, Sha256};

    fn hash_bytes(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }

    fn hash_series(hasher: &mut Sha256, label: &[u8], values: &[f32]) {
        hash_bytes(hasher, label);
        hasher.update((values.len() as u64).to_be_bytes());
        for value in values {
            hasher.update(value.to_bits().to_be_bytes());
        }
    }

    let mut hasher = Sha256::new();
    hash_bytes(&mut hasher, b"reklawdbox:stratum-grid:v1");
    hash_series(&mut hasher, b"beats", &grid.beats);
    hash_series(&mut hasher, b"downbeats", &grid.downbeats);
    hash_series(&mut hasher, b"bars", &grid.bars);
    format!("grid:v1:{:x}", hasher.finalize())
}

#[cfg(not(test))]
pub fn load_rekordbox_grid_input_for_path(file_path: &str) -> RekordboxGridInput {
    load_rekordbox_grid_inputs_for_paths(&[file_path])
        .into_iter()
        .next()
        .unwrap_or_else(|| RekordboxGridInput::from_grid(None))
}

/// Unit tests use explicit synthetic grid inputs and never inspect the user's
/// Rekordbox library as an accidental side effect of cache probing.
#[cfg(test)]
pub fn load_rekordbox_grid_input_for_path(_file_path: &str) -> RekordboxGridInput {
    RekordboxGridInput::from_grid(None)
}

#[cfg(not(test))]
pub fn load_rekordbox_grid_inputs_for_paths(file_paths: &[&str]) -> Vec<RekordboxGridInput> {
    if file_paths.is_empty() {
        return Vec::new();
    }
    let Some(db_path) = rekordbox::resolve_db_path() else {
        tracing::warn!(
            "Rekordbox grid lookup: no master.db found (set REKORDBOX_DB_PATH); \
             falling back to HMM for all tracks"
        );
        return file_paths
            .iter()
            .map(|_| RekordboxGridInput::from_grid(None))
            .collect();
    };
    let conn = match rekordbox::open(&db_path) {
        Ok(conn) => conn,
        Err(error) => {
            tracing::warn!(
                "Rekordbox grid lookup: could not open master.db ({error}); falling back to HMM"
            );
            return file_paths
                .iter()
                .map(|_| RekordboxGridInput::from_grid(None))
                .collect();
        }
    };
    file_paths
        .iter()
        .map(|path| {
            RekordboxGridInput::from_grid(rekordbox_anlz::load_rekordbox_grid_for_path_with_conn(
                &conn, path,
            ))
        })
        .collect()
}

#[cfg(test)]
pub fn load_rekordbox_grid_inputs_for_paths(file_paths: &[&str]) -> Vec<RekordboxGridInput> {
    file_paths
        .iter()
        .map(|_| RekordboxGridInput::from_grid(None))
        .collect()
}

#[derive(Clone, Debug)]
pub(crate) struct AudioCacheIdentity {
    pub(crate) cache_key: String,
    pub(crate) file_size: i64,
    pub(crate) file_mtime: i64,
    pub(crate) stratum_input_fingerprint: Option<String>,
}

impl AudioCacheIdentity {
    pub(crate) fn as_stratum_store_identity(
        &self,
    ) -> Option<state_analysis::AudioAnalysisIdentity<'_>> {
        Some(state_analysis::AudioAnalysisIdentity {
            file_path: &self.cache_key,
            file_size: self.file_size,
            file_mtime: self.file_mtime,
            input_fingerprint: self.stratum_input_fingerprint.as_deref()?,
        })
    }

    pub(crate) fn as_essentia_store_identity(&self) -> state_analysis::AudioAnalysisIdentity<'_> {
        state_analysis::AudioAnalysisIdentity {
            file_path: &self.cache_key,
            file_size: self.file_size,
            file_mtime: self.file_mtime,
            input_fingerprint: "",
        }
    }
}

pub(crate) fn file_mtime_unix(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

pub(crate) fn resolved_audio_cache_key(raw_file_path: &str) -> String {
    audio::resolve_audio_path(raw_file_path).unwrap_or_else(|_| raw_file_path.to_string())
}

pub(crate) fn audio_cache_identity(raw_file_path: &str) -> Option<AudioCacheIdentity> {
    let cache_key = audio::resolve_audio_path(raw_file_path).ok()?;
    let metadata = std::fs::metadata(&cache_key).ok()?;
    Some(AudioCacheIdentity {
        cache_key,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
        stratum_input_fingerprint: None,
    })
}

#[cfg(test)]
pub(crate) fn audio_cache_identities_with_fingerprint_loader<'a>(
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
pub(crate) fn audio_cache_identity_with_stratum_input_fingerprint(
    raw_file_path: &str,
    input_fingerprint: impl Into<String>,
) -> Option<AudioCacheIdentity> {
    let mut identity = audio_cache_identity(raw_file_path)?;
    identity.stratum_input_fingerprint = Some(input_fingerprint.into());
    Some(identity)
}

pub(crate) fn audio_cache_identities_with_current_stratum_input<'a>(
    raw_file_paths: impl IntoIterator<Item = &'a str>,
) -> Vec<Option<AudioCacheIdentity>> {
    let mut identities: Vec<_> = raw_file_paths
        .into_iter()
        .map(audio_cache_identity)
        .collect();
    let mut unique_cache_keys = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for identity in identities.iter().flatten() {
        if seen.insert(identity.cache_key.as_str()) {
            unique_cache_keys.push(identity.cache_key.clone());
        }
    }
    let cache_key_refs: Vec<_> = unique_cache_keys.iter().map(String::as_str).collect();
    let fingerprints: HashMap<String, String> = unique_cache_keys
        .iter()
        .cloned()
        .zip(
            load_rekordbox_grid_inputs_for_paths(&cache_key_refs)
                .into_iter()
                .map(|input| input.fingerprint),
        )
        .collect();
    for identity in identities.iter_mut().flatten() {
        identity.stratum_input_fingerprint = fingerprints.get(&identity.cache_key).cloned();
    }
    identities
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

pub(crate) fn get_fresh_analysis_entry(
    store: &Connection,
    raw_file_path: &str,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<state_analysis::CachedAudioAnalysis>, String> {
    let Some(identity) = audio_cache_identity_for_analyzer(raw_file_path, analyzer) else {
        return Ok(None);
    };
    get_fresh_analysis_entry_for_identity(store, &identity, analyzer, schema_version)
}

pub(crate) fn get_fresh_analysis_entry_for_identity(
    store: &Connection,
    identity: &AudioCacheIdentity,
    analyzer: &str,
    schema_version: &str,
) -> Result<Option<state_analysis::CachedAudioAnalysis>, String> {
    let store_identity = if analyzer == audio::ANALYZER_STRATUM {
        identity
            .as_stratum_store_identity()
            .ok_or_else(|| "Stratum cache identity is missing its input fingerprint".to_string())?
    } else {
        identity.as_essentia_store_identity()
    };
    let cached = state_analysis::get_audio_analysis(store, &identity.cache_key, analyzer)
        .map_err(|e| format!("Cache read error: {e}"))?;
    if state_analysis::is_audio_analysis_fresh(
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

pub(crate) fn check_analysis_cache(
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

pub(crate) fn check_analysis_cache_for_identity(
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

pub(crate) fn is_cache_fresh(
    cached: Option<&state_analysis::CachedAudioAnalysis>,
    schema_version: &str,
    file_size: i64,
    file_mtime: i64,
    input_fingerprint: &str,
) -> bool {
    state_analysis::is_audio_analysis_fresh(
        cached,
        schema_version,
        file_size,
        file_mtime,
        input_fingerprint,
    )
}

pub(crate) struct CacheProbe {
    pub(crate) cache_key: String,
    pub(crate) file_size: i64,
    pub(crate) file_mtime: i64,
    pub(crate) stratum_input_fingerprint: String,
}

impl CacheProbe {
    pub(crate) fn input_fingerprint_for_analyzer(&self, analyzer: &str) -> &str {
        if analyzer == audio::ANALYZER_STRATUM {
            &self.stratum_input_fingerprint
        } else {
            ""
        }
    }
}

pub(crate) fn cache_probe_for_path(file_path: &str, skip_cached: bool) -> Option<CacheProbe> {
    if !skip_cached {
        return None;
    }
    let cache_key = audio::resolve_audio_path(file_path).ok()?;
    let metadata = std::fs::metadata(&cache_key).ok()?;
    let stratum_input_fingerprint = load_rekordbox_grid_input_for_path(&cache_key).fingerprint;
    Some(CacheProbe {
        cache_key,
        file_size: metadata.len() as i64,
        file_mtime: file_mtime_unix(&metadata),
        stratum_input_fingerprint,
    })
}

pub(crate) fn has_fresh_cache_entry(
    store_conn: &Connection,
    cache_probe: Option<&CacheProbe>,
    analyzer: &str,
    schema_version: &str,
) -> Result<bool, rusqlite::Error> {
    if let Some(cache_probe) = cache_probe {
        let cached =
            state_analysis::get_audio_analysis(store_conn, &cache_probe.cache_key, analyzer)?;
        Ok(is_cache_fresh(
            cached.as_ref(),
            schema_version,
            cache_probe.file_size,
            cache_probe.file_mtime,
            cache_probe.input_fingerprint_for_analyzer(analyzer),
        ))
    } else {
        Ok(false)
    }
}

pub(crate) fn cache_status_for_track(
    store_conn: &Connection,
    cache_probe: Option<&CacheProbe>,
    skip_cached: bool,
    essentia_available: bool,
) -> Result<(bool, bool), rusqlite::Error> {
    let has_stratum = if skip_cached {
        has_fresh_cache_entry(
            store_conn,
            cache_probe,
            audio::ANALYZER_STRATUM,
            audio::STRATUM_SCHEMA_VERSION,
        )?
    } else {
        false
    };

    let has_essentia = if !essentia_available {
        true
    } else if skip_cached {
        has_fresh_cache_entry(
            store_conn,
            cache_probe,
            audio::ANALYZER_ESSENTIA,
            audio::ESSENTIA_SCHEMA_VERSION,
        )?
    } else {
        false
    };

    Ok((has_stratum, has_essentia))
}

pub(crate) fn analyze_with_stratum_input(
    samples: &[f32],
    sample_rate: u32,
    input: RekordboxGridInput,
) -> Result<StratumAnalysis, audio::AudioError> {
    analyze_with_stratum_input_using(input, |grid| {
        audio::analyze_with_stratum(samples, sample_rate, grid)
    })
}

pub(crate) fn analyze_with_stratum_input_using(
    input: RekordboxGridInput,
    analyze: impl FnOnce(
        Option<stratum_dsp::BeatGrid>,
    ) -> Result<audio::StratumResult, audio::AudioError>,
) -> Result<StratumAnalysis, audio::AudioError> {
    let RekordboxGridInput { grid, fingerprint } = input;
    let result = analyze(grid)?;
    Ok(StratumAnalysis {
        result,
        input_fingerprint: fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> stratum_dsp::BeatGrid {
        stratum_dsp::BeatGrid {
            beats: vec![0.5, 1.0, 1.5, 2.0],
            downbeats: vec![0.5],
            bars: vec![0.5, 2.5],
        }
    }

    #[test]
    fn grid_input_fingerprint_is_stable_and_versioned() {
        let first = RekordboxGridInput::from_grid(Some(grid()));
        let second = RekordboxGridInput::from_grid(Some(grid()));

        assert_eq!(first.fingerprint, second.fingerprint);
        assert!(first.fingerprint.starts_with("grid:v1:"));
        assert!(!first.fingerprint.contains('/'));
    }

    #[test]
    fn grid_input_fingerprint_covers_every_semantic_series() {
        let base = RekordboxGridInput::from_grid(Some(grid())).fingerprint;

        let mut beat_changed = grid();
        beat_changed.beats[1] = f32::from_bits(beat_changed.beats[1].to_bits() + 1);
        let mut downbeat_changed = grid();
        downbeat_changed.downbeats[0] = 1.0;
        let mut bar_changed = grid();
        bar_changed.bars[1] = 3.0;
        let mut length_changed = grid();
        length_changed.beats.push(2.5);
        let mut order_changed = grid();
        order_changed.beats.swap(0, 1);

        for changed in [
            beat_changed,
            downbeat_changed,
            bar_changed,
            length_changed,
            order_changed,
        ] {
            assert_ne!(
                RekordboxGridInput::from_grid(Some(changed)).fingerprint,
                base
            );
        }
    }

    #[test]
    fn grid_input_fingerprint_distinguishes_hmm_source() {
        let hmm = RekordboxGridInput::from_grid(None);
        let grid = RekordboxGridInput::from_grid(Some(grid()));

        assert_eq!(hmm.fingerprint, "hmm:v1");
        assert!(hmm.grid.is_none());
        assert_ne!(hmm.fingerprint, grid.fingerprint);
    }

    #[test]
    fn stratum_analysis_keeps_the_fingerprint_paired_with_its_grid_snapshot() {
        let analyzed_input = RekordboxGridInput::from_grid(Some(grid()));
        let analyzed_fingerprint = analyzed_input.fingerprint.clone();
        let mut changed_grid = grid();
        changed_grid.beats[0] = 0.25;
        let later_current = RekordboxGridInput::from_grid(Some(changed_grid));

        let analyzed = analyze_with_stratum_input_using(analyzed_input, |snapshot| {
            assert_eq!(snapshot.unwrap().beats, grid().beats);
            Ok(audio::StratumResult::default())
        })
        .unwrap();

        assert_eq!(analyzed.input_fingerprint, analyzed_fingerprint);
        assert_ne!(analyzed.input_fingerprint, later_current.fingerprint);
    }

    #[test]
    fn grid_fingerprint_is_stable_and_versioned() {
        let first = RekordboxGridInput::from_grid(Some(grid()));
        let second = RekordboxGridInput::from_grid(Some(grid()));
        let mut changed_grid = grid();
        changed_grid.bars.push(4.5);
        let changed = RekordboxGridInput::from_grid(Some(changed_grid));
        let no_grid = RekordboxGridInput::from_grid(None);
        assert_eq!(first.fingerprint, second.fingerprint);
        assert!(first.fingerprint.starts_with("grid:v1:"));
        assert_ne!(first.fingerprint, changed.fingerprint);
        assert_eq!(no_grid.fingerprint, audio::STRATUM_HMM_INPUT_FINGERPRINT);
    }

    #[test]
    fn audio_cache_identity_preserves_field_contract() {
        let directory = tempfile::tempdir().expect("temporary audio directory");
        let path = directory.path().join("identity.flac");
        let payload = b"synthetic identity fixture";
        std::fs::write(&path, payload).expect("write synthetic audio identity fixture");
        let raw_path = path.to_string_lossy();
        let identity = audio_cache_identities_with_current_stratum_input([raw_path.as_ref()])
            .into_iter()
            .next()
            .flatten()
            .expect("identity should resolve from the synthetic file");
        let metadata = std::fs::metadata(&path).expect("read synthetic fixture metadata");

        assert_eq!(identity.cache_key, raw_path);
        assert_eq!(identity.file_size, payload.len() as i64);
        assert_eq!(identity.file_mtime, file_mtime_unix(&metadata));
        assert_eq!(
            identity.stratum_input_fingerprint.as_deref(),
            Some(audio::STRATUM_HMM_INPUT_FINGERPRINT)
        );

        let stratum = identity
            .as_stratum_store_identity()
            .expect("Stratum identity should include its input fingerprint");
        assert_eq!(stratum.file_path, raw_path);
        assert_eq!(stratum.file_size, payload.len() as i64);
        assert_eq!(
            stratum.input_fingerprint,
            audio::STRATUM_HMM_INPUT_FINGERPRINT
        );
        let essentia = identity.as_essentia_store_identity();
        assert_eq!(essentia.file_path, raw_path);
        assert_eq!(essentia.input_fingerprint, "");
    }
}
