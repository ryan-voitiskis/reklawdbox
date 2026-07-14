//! Audio-profile calibration and coverage workflows.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

use crate::adapters::{audio, state};
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input,
};
use crate::domain::{
    classification::{
        AudioFeatures,
        profiles::{self, ProfileMetadata},
        taxonomy as genre,
    },
    library::Track,
};

use super::evidence::extract_audio_features;

#[derive(Debug)]
pub(crate) enum CalibrationError {
    NoSamples,
    NoUsableProfiles,
    Store(rusqlite::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrainingFingerprintRow {
    track_id: String,
    genre: &'static str,
    file_size: i64,
    file_mtime: i64,
    stratum_input_fingerprint: String,
}

/// Stable, privacy-safe identity for the scorable verified corpus. Absolute
/// paths, payload JSON, timestamps, and iteration order are deliberately absent.
fn training_fingerprint(rows: &[TrainingFingerprintRow]) -> String {
    fn hash_field(hasher: &mut Sha256, value: &[u8]) {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }

    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| {
        a.track_id
            .cmp(&b.track_id)
            .then_with(|| a.genre.cmp(b.genre))
            .then_with(|| a.file_size.cmp(&b.file_size))
            .then_with(|| a.file_mtime.cmp(&b.file_mtime))
            .then_with(|| {
                a.stratum_input_fingerprint
                    .cmp(&b.stratum_input_fingerprint)
            })
    });
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"reklawdbox:genre-profile-training:v1");
    hash_field(&mut hasher, audio::STRATUM_SCHEMA_VERSION.as_bytes());
    hash_field(&mut hasher, audio::ESSENTIA_SCHEMA_VERSION.as_bytes());
    for row in rows {
        hash_field(&mut hasher, row.track_id.as_bytes());
        hash_field(&mut hasher, row.genre.as_bytes());
        hasher.update(row.file_size.to_be_bytes());
        hasher.update(row.file_mtime.to_be_bytes());
        hash_field(&mut hasher, row.stratum_input_fingerprint.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub(crate) fn calibrate_audio_profiles(
    store_conn: &Connection,
    tracks: &[Track],
    playlist_name: &str,
) -> Result<serde_json::Value, CalibrationError> {
    // 2. Load audio features for each track
    let mut samples: Vec<(&'static str, AudioFeatures)> = Vec::new();
    let mut fingerprint_rows = Vec::new();
    let mut tracks_with_audio_features = 0u32;
    let mut skipped_no_genre = 0u32;
    let mut skipped_no_audio = 0u32;
    let mut skipped_unscorable_audio = 0u32;
    let mut skipped_unknown_genre = 0u32;
    let mut eligible_tracks = Vec::new();

    for track in tracks {
        // Must have a genre tag
        if track.genre.is_empty() {
            skipped_no_genre += 1;
            continue;
        }

        // Resolve to canonical genre
        let canonical = match genre::resolve_genre(&track.genre) {
            Some(g) => g,
            None => {
                skipped_unknown_genre += 1;
                continue;
            }
        };

        eligible_tracks.push((track, canonical));
    }

    let current_audio_identities = audio_cache_identities_with_current_stratum_input(
        eligible_tracks
            .iter()
            .map(|(track, _)| track.file_path.as_str()),
    );
    let eligible_tracks: Vec<_> = eligible_tracks
        .into_iter()
        .zip(current_audio_identities)
        .map(|((track, canonical), identity)| (track, canonical, identity))
        .collect();
    let stratum_identities: Vec<_> = eligible_tracks
        .iter()
        .filter_map(|(_, _, identity)| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = eligible_tracks
        .iter()
        .filter_map(|(_, _, identity)| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    let stratum_map = state::batch_get_fresh_audio_analysis(
        store_conn,
        &stratum_identities,
        audio::ANALYZER_STRATUM,
        audio::STRATUM_SCHEMA_VERSION,
    )
    .map_err(CalibrationError::Store)?;
    let essentia_map = state::batch_get_fresh_audio_analysis(
        store_conn,
        &essentia_identities,
        audio::ANALYZER_ESSENTIA,
        audio::ESSENTIA_SCHEMA_VERSION,
    )
    .map_err(CalibrationError::Store)?;

    for (track, canonical, audio_identity) in eligible_tracks {
        let audio_key = audio_identity
            .as_ref()
            .map(|identity| identity.cache_key.as_str());
        let stratum_cache = audio_key.and_then(|key| stratum_map.get(key));
        let essentia_cache = audio_key.and_then(|key| essentia_map.get(key));

        match extract_audio_features(track, stratum_cache, essentia_cache) {
            Some(features) => {
                tracks_with_audio_features += 1;
                if !profiles::has_scorable_optional_features(&features) {
                    skipped_unscorable_audio += 1;
                    continue;
                }
                let identity = audio_identity
                    .as_ref()
                    .expect("fresh audio cache rows require a resolved file identity");
                fingerprint_rows.push(TrainingFingerprintRow {
                    track_id: track.id.clone(),
                    genre: canonical,
                    file_size: identity.file_size,
                    file_mtime: identity.file_mtime,
                    stratum_input_fingerprint: identity
                        .stratum_input_fingerprint
                        .clone()
                        .unwrap_or_default(),
                });
                samples.push((canonical, features));
            }
            None => {
                skipped_no_audio += 1;
            }
        }
    }

    if tracks_with_audio_features == 0 {
        return Err(CalibrationError::NoSamples);
    }
    if samples.is_empty() {
        return Err(CalibrationError::NoUsableProfiles);
    }

    // 3. Calibrate
    let sample_refs: Vec<(&str, &AudioFeatures)> = samples.iter().map(|(g, f)| (*g, f)).collect();
    let mut registry = profiles::calibrate(&sample_refs);
    let usable_genres: HashSet<_> = registry
        .prototypes
        .keys()
        .copied()
        .filter(|candidate| {
            samples.iter().any(|(sample_genre, features)| {
                sample_genre == candidate
                    && profiles::can_score_genre(features, &registry, candidate)
            })
        })
        .collect();
    registry
        .prototypes
        .retain(|genre, _| usable_genres.contains(genre));
    if registry.prototypes.is_empty() {
        return Err(CalibrationError::NoUsableProfiles);
    }

    // 4. Save to SQLite
    let metadata = ProfileMetadata {
        classifier_profile_schema_version: profiles::PROFILE_SCHEMA_VERSION.into(),
        stratum_schema_version: audio::STRATUM_SCHEMA_VERSION.into(),
        essentia_schema_version: audio::ESSENTIA_SCHEMA_VERSION.into(),
        playlist_name: playlist_name.to_string(),
        training_fingerprint: training_fingerprint(&fingerprint_rows),
        scorable_sample_count: samples.len() as u32,
        calibrated_at: chrono::Utc::now().to_rfc3339(),
    };
    state::classification::save_to_db(store_conn, &registry, &metadata)
        .map_err(CalibrationError::Store)?;

    // 5. Build summary
    let mut genre_summaries: Vec<serde_json::Value> = registry
        .prototypes
        .values()
        .map(|proto| {
            let mut top_features: Vec<(&str, f64)> = proto
                .features
                .iter()
                .map(|(&name, stat)| (name, stat.fisher_weight))
                .collect();
            top_features.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            top_features.truncate(5);

            let feature_strs: Vec<String> = top_features
                .iter()
                .map(|(name, weight)| format!("{name} ({:.0}%)", weight * 100.0))
                .collect();

            serde_json::json!({
                "genre": proto.genre,
                "n_verified": proto.total_n,
                "n_features": proto.features.len(),
                "has_timbral": proto.mfcc_centroid.is_some(),
                "top_discriminators": feature_strs,
            })
        })
        .collect();
    genre_summaries.sort_by(|a, b| {
        let na = a["n_verified"].as_u64().unwrap_or(0);
        let nb = b["n_verified"].as_u64().unwrap_or(0);
        nb.cmp(&na)
    });

    let result = serde_json::json!({
        "status": "calibrated",
        "playlist": playlist_name,
        "total_tracks": tracks.len(),
        "tracks_with_features": tracks_with_audio_features,
        "tracks_with_scorable_features": samples.len(),
        "skipped_no_genre": skipped_no_genre,
        "skipped_unknown_genre": skipped_unknown_genre,
        "skipped_no_audio": skipped_no_audio,
        "skipped_unscorable_audio": skipped_unscorable_audio,
        "prototypes_built": registry.prototypes.len(),
        "profile_metadata": metadata,
        "genres": genre_summaries,
    });

    Ok(result)
}

#[derive(Debug, Default)]
struct CalibrationGenreStats {
    playlist_tracks: u32,
    tracks_with_audio_features: u32,
    missing_audio_features: u32,
    tracks_with_scorable_features: u32,
    missing_scorable_features: u32,
    tracks_with_stratum_features: u32,
    missing_stratum_features: u32,
    tracks_with_essentia_features: u32,
    missing_essentia_features: u32,
}

pub(crate) fn calibration_coverage(
    store_conn: &Connection,
    tracks: &[Track],
    resolved_playlist_name: &str,
) -> Result<serde_json::Value, rusqlite::Error> {
    let mut by_genre: BTreeMap<&'static str, CalibrationGenreStats> = BTreeMap::new();
    let mut skipped_no_genre = 0u32;
    let mut skipped_unknown_genre = 0u32;
    let mut eligible_tracks = Vec::new();
    let mut scorable_samples: Vec<(&'static str, AudioFeatures)> = Vec::new();
    let mut fingerprint_rows = Vec::new();

    for track in tracks {
        if track.genre.trim().is_empty() {
            skipped_no_genre += 1;
            continue;
        }

        let Some(canonical) = genre::resolve_genre(&track.genre) else {
            skipped_unknown_genre += 1;
            continue;
        };

        let stats = by_genre.entry(canonical).or_default();
        stats.playlist_tracks += 1;

        eligible_tracks.push((track, canonical));
    }

    let current_audio_identities = audio_cache_identities_with_current_stratum_input(
        eligible_tracks
            .iter()
            .map(|(track, _)| track.file_path.as_str()),
    );
    let eligible_tracks: Vec<_> = eligible_tracks
        .into_iter()
        .zip(current_audio_identities)
        .map(|((track, canonical), identity)| (track, canonical, identity))
        .collect();
    let stratum_identities: Vec<_> = eligible_tracks
        .iter()
        .filter_map(|(_, _, identity)| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = eligible_tracks
        .iter()
        .filter_map(|(_, _, identity)| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    let stratum_map = state::batch_get_fresh_audio_analysis(
        store_conn,
        &stratum_identities,
        audio::ANALYZER_STRATUM,
        audio::STRATUM_SCHEMA_VERSION,
    )?;
    let essentia_map = state::batch_get_fresh_audio_analysis(
        store_conn,
        &essentia_identities,
        audio::ANALYZER_ESSENTIA,
        audio::ESSENTIA_SCHEMA_VERSION,
    )?;

    for (track, canonical, audio_identity) in eligible_tracks {
        let stats = by_genre
            .get_mut(canonical)
            .expect("eligible track genre stats should already exist");
        let audio_key = audio_identity
            .as_ref()
            .map(|identity| identity.cache_key.as_str());
        let stratum = audio_key.and_then(|key| stratum_map.get(key));
        let essentia = audio_key.and_then(|key| essentia_map.get(key));

        if stratum.is_some() {
            stats.tracks_with_stratum_features += 1;
        } else {
            stats.missing_stratum_features += 1;
        }
        if essentia.is_some() {
            stats.tracks_with_essentia_features += 1;
        } else {
            stats.missing_essentia_features += 1;
        }

        match extract_audio_features(track, stratum, essentia) {
            Some(features) => {
                stats.tracks_with_audio_features += 1;
                if profiles::has_scorable_optional_features(&features) {
                    stats.tracks_with_scorable_features += 1;
                    let identity = audio_identity
                        .as_ref()
                        .expect("fresh audio cache rows require a resolved file identity");
                    fingerprint_rows.push(TrainingFingerprintRow {
                        track_id: track.id.clone(),
                        genre: canonical,
                        file_size: identity.file_size,
                        file_mtime: identity.file_mtime,
                        stratum_input_fingerprint: identity
                            .stratum_input_fingerprint
                            .clone()
                            .unwrap_or_default(),
                    });
                    scorable_samples.push((canonical, features));
                } else {
                    stats.missing_scorable_features += 1;
                }
            }
            None => {
                stats.missing_audio_features += 1;
                stats.missing_scorable_features += 1;
            }
        }
    }

    let sample_refs: Vec<_> = scorable_samples
        .iter()
        .map(|(genre, features)| (*genre, features))
        .collect();
    let candidate_registry = profiles::calibrate(&sample_refs);
    let candidate_ready: HashSet<_> = candidate_registry
        .prototypes
        .keys()
        .copied()
        .filter(|candidate| {
            scorable_samples.iter().any(|(sample_genre, features)| {
                sample_genre == candidate
                    && profiles::can_score_genre(features, &candidate_registry, candidate)
            })
        })
        .collect();
    let current_training_fingerprint = training_fingerprint(&fingerprint_rows);
    let profile_load =
        state::classification::load_from_db(store_conn, Some(&current_training_fingerprint))?;
    let existing_profiles: HashMap<&'static str, u32> = profile_load
        .registry
        .as_ref()
        .map(|registry| {
            registry
                .prototypes
                .values()
                .map(|proto| (proto.genre, proto.total_n))
                .collect()
        })
        .unwrap_or_default();

    let mut ready_to_calibrate = 0u32;
    let mut below_min_tracks = 0u32;
    let mut stored_profiles_present = 0u32;
    let mut total_with_audio_features = 0u32;
    let mut total_missing_audio_features = 0u32;
    let mut total_with_scorable_features = 0u32;
    let mut total_missing_scorable_features = 0u32;
    let mut total_with_stratum_features = 0u32;
    let mut total_missing_stratum_features = 0u32;
    let mut total_with_essentia_features = 0u32;
    let mut total_missing_essentia_features = 0u32;

    let genres: Vec<serde_json::Value> = by_genre
        .iter()
        .map(|(&genre, stats)| {
            let stored_n = existing_profiles.get(genre).copied();
            let prototype_ready = candidate_ready.contains(genre);
            if prototype_ready && stored_n.is_none() {
                ready_to_calibrate += 1;
            }
            if stats.tracks_with_scorable_features < profiles::MIN_TRACKS {
                below_min_tracks += 1;
            }
            if stored_n.is_some() {
                stored_profiles_present += 1;
            }
            total_with_audio_features += stats.tracks_with_audio_features;
            total_missing_audio_features += stats.missing_audio_features;
            total_with_scorable_features += stats.tracks_with_scorable_features;
            total_missing_scorable_features += stats.missing_scorable_features;
            total_with_stratum_features += stats.tracks_with_stratum_features;
            total_missing_stratum_features += stats.missing_stratum_features;
            total_with_essentia_features += stats.tracks_with_essentia_features;
            total_missing_essentia_features += stats.missing_essentia_features;

            let status = if prototype_ready && stored_n.is_some() {
                "profile_present"
            } else if prototype_ready {
                "ready_to_calibrate"
            } else if stats.tracks_with_scorable_features >= profiles::MIN_TRACKS {
                "candidate_not_scorable"
            } else {
                "needs_more_verified_audio"
            };

            serde_json::json!({
                "genre": genre,
                "playlist_tracks": stats.playlist_tracks,
                "tracks_with_audio_features": stats.tracks_with_audio_features,
                "missing_audio_features": stats.missing_audio_features,
                "tracks_with_scorable_features": stats.tracks_with_scorable_features,
                "missing_scorable_features": stats.missing_scorable_features,
                "tracks_with_stratum_features": stats.tracks_with_stratum_features,
                "missing_stratum_features": stats.missing_stratum_features,
                "tracks_with_essentia_features": stats.tracks_with_essentia_features,
                "missing_essentia_features": stats.missing_essentia_features,
                "prototype_ready": prototype_ready,
                "profile": {
                    "stored": stored_n.is_some(),
                    "n_verified": stored_n,
                },
                "status": status,
            })
        })
        .collect();

    let playlist_genres: std::collections::HashSet<&str> = by_genre.keys().copied().collect();
    let mut stored_profiles_not_in_playlist: Vec<&str> = existing_profiles
        .keys()
        .filter(|genre| !playlist_genres.contains(**genre))
        .copied()
        .collect();
    stored_profiles_not_in_playlist.sort_unstable();

    let result = serde_json::json!({
        "status": "ok",
        "playlist": resolved_playlist_name,
        "total_tracks": tracks.len(),
        "tracks_with_canonical_genre": by_genre.values().map(|stats| stats.playlist_tracks).sum::<u32>(),
        "tracks_with_audio_features": total_with_audio_features,
        "missing_audio_features": total_missing_audio_features,
        "tracks_with_scorable_features": total_with_scorable_features,
        "missing_scorable_features": total_missing_scorable_features,
        "tracks_with_stratum_features": total_with_stratum_features,
        "missing_stratum_features": total_missing_stratum_features,
        "tracks_with_essentia_features": total_with_essentia_features,
        "missing_essentia_features": total_missing_essentia_features,
        "skipped_no_genre": skipped_no_genre,
        "skipped_unknown_genre": skipped_unknown_genre,
        "min_tracks_per_genre": profiles::MIN_TRACKS,
        "prototypes_existing": existing_profiles.len(),
        "genres_ready_to_calibrate": ready_to_calibrate,
        "genres_below_min_tracks": below_min_tracks,
        "genres_with_stored_profiles": stored_profiles_present,
        "stored_profiles_not_in_playlist": stored_profiles_not_in_playlist,
        "profile_state": {
            "status": profile_load.status,
            "reason": profile_load.reason,
            "metadata": profile_load.metadata,
            "current_training_fingerprint": current_training_fingerprint,
        },
        "genres": genres,
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(track_id: &str, genre: &'static str) -> TrainingFingerprintRow {
        TrainingFingerprintRow {
            track_id: track_id.into(),
            genre,
            file_size: 100,
            file_mtime: 200,
            stratum_input_fingerprint: "grid:v1:test".into(),
        }
    }

    #[test]
    fn training_fingerprint_is_order_stable_and_privacy_safe() {
        let a = row("track-a", "Techno");
        let b = row("track-b", "House");
        let first = training_fingerprint(&[a.clone(), b.clone()]);
        let reversed = training_fingerprint(&[b, a]);
        assert_eq!(first, reversed);
        assert!(first.starts_with("sha256:"));
        assert!(!first.contains('/'));
    }

    #[test]
    fn training_fingerprint_changes_with_training_identity() {
        let baseline = training_fingerprint(&[row("track-a", "Techno")]);
        assert_ne!(baseline, training_fingerprint(&[row("track-a", "House")]));
        assert_ne!(baseline, training_fingerprint(&[row("track-b", "Techno")]));
        let mut changed_audio = row("track-a", "Techno");
        changed_audio.file_mtime += 1;
        assert_ne!(baseline, training_fingerprint(&[changed_audio]));
    }
}
