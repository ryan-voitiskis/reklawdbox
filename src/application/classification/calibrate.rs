//! Audio-profile calibration and coverage workflows.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Connection;

use crate::adapters::{audio, state};
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input,
};
use crate::domain::{
    classification::{AudioFeatures, profiles, taxonomy as genre},
    library::Track,
};

use super::evidence::extract_audio_features;

#[derive(Debug)]
pub(crate) enum CalibrationError {
    NoSamples,
    Store(rusqlite::Error),
}

pub(crate) fn calibrate_audio_profiles(
    store_conn: &Connection,
    tracks: &[Track],
    playlist_name: &str,
) -> Result<serde_json::Value, CalibrationError> {
    // 2. Load audio features for each track
    let mut samples: Vec<(&'static str, AudioFeatures)> = Vec::new();
    let mut skipped_no_genre = 0u32;
    let mut skipped_no_audio = 0u32;
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
            Some(features) => samples.push((canonical, features)),
            None => {
                skipped_no_audio += 1;
            }
        }
    }

    if samples.is_empty() {
        return Err(CalibrationError::NoSamples);
    }

    // 3. Calibrate
    let sample_refs: Vec<(&str, &AudioFeatures)> = samples.iter().map(|(g, f)| (*g, f)).collect();
    let registry = profiles::calibrate(&sample_refs);

    // 4. Save to SQLite
    state::classification::save_to_db(store_conn, &registry).map_err(CalibrationError::Store)?;

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
        "tracks_with_features": samples.len(),
        "skipped_no_genre": skipped_no_genre,
        "skipped_unknown_genre": skipped_unknown_genre,
        "skipped_no_audio": skipped_no_audio,
        "prototypes_built": registry.prototypes.len(),
        "genres": genre_summaries,
    });

    Ok(result)
}

#[derive(Debug, Default)]
struct CalibrationGenreStats {
    playlist_tracks: u32,
    tracks_with_audio_features: u32,
    missing_audio_features: u32,
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
    let existing_registry = state::classification::load_from_db(store_conn)?;
    let existing_profiles: HashMap<&'static str, u32> = existing_registry
        .as_ref()
        .map(|registry| {
            registry
                .prototypes
                .values()
                .map(|proto| (proto.genre, proto.total_n))
                .collect()
        })
        .unwrap_or_default();

    let mut by_genre: BTreeMap<&'static str, CalibrationGenreStats> = BTreeMap::new();
    let mut skipped_no_genre = 0u32;
    let mut skipped_unknown_genre = 0u32;
    let mut eligible_tracks = Vec::new();

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

        if extract_audio_features(track, stratum, essentia).is_some() {
            stats.tracks_with_audio_features += 1;
        } else {
            stats.missing_audio_features += 1;
        }
    }

    let mut ready_to_calibrate = 0u32;
    let mut below_min_tracks = 0u32;
    let mut stored_profiles_present = 0u32;
    let mut total_with_audio_features = 0u32;
    let mut total_missing_audio_features = 0u32;
    let mut total_with_stratum_features = 0u32;
    let mut total_missing_stratum_features = 0u32;
    let mut total_with_essentia_features = 0u32;
    let mut total_missing_essentia_features = 0u32;

    let genres: Vec<serde_json::Value> = by_genre
        .iter()
        .map(|(&genre, stats)| {
            let stored_n = existing_profiles.get(genre).copied();
            let prototype_ready = stats.tracks_with_audio_features >= profiles::MIN_TRACKS;
            if prototype_ready && stored_n.is_none() {
                ready_to_calibrate += 1;
            }
            if !prototype_ready {
                below_min_tracks += 1;
            }
            if stored_n.is_some() {
                stored_profiles_present += 1;
            }
            total_with_audio_features += stats.tracks_with_audio_features;
            total_missing_audio_features += stats.missing_audio_features;
            total_with_stratum_features += stats.tracks_with_stratum_features;
            total_missing_stratum_features += stats.missing_stratum_features;
            total_with_essentia_features += stats.tracks_with_essentia_features;
            total_missing_essentia_features += stats.missing_essentia_features;

            let status = if prototype_ready && stored_n.is_some() {
                "profile_present"
            } else if prototype_ready {
                "ready_to_calibrate"
            } else {
                "needs_more_verified_audio"
            };

            serde_json::json!({
                "genre": genre,
                "playlist_tracks": stats.playlist_tracks,
                "tracks_with_audio_features": stats.tracks_with_audio_features,
                "missing_audio_features": stats.missing_audio_features,
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
        "genres": genres,
    });

    Ok(result)
}
