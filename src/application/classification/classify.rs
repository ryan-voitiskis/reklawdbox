//! Classification batch orchestration over cached evidence.

use rusqlite::Connection;

use crate::adapters::{audio, state};
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input, resolved_audio_cache_key,
};
use crate::domain::classification::{ClassificationResult, engine::classify_track_with_profiles};
use crate::domain::library::Track;
use crate::domain::metadata as normalize;

use super::evidence::build_track_evidence;

pub(crate) fn classify_batch(
    store_conn: &Connection,
    tracks: &[Track],
    overrides: &[(String, String)],
) -> Result<(Vec<ClassificationResult>, u32), rusqlite::Error> {
    classify_batch_inner(store_conn, tracks, overrides, true)
}

/// Classify from cached metadata and rule-based audio evidence without loading
/// a persisted profile registry. Used by non-leaky benchmark baselines.
#[cfg(test)]
pub(crate) fn classify_batch_rules_only(
    store_conn: &Connection,
    tracks: &[Track],
    overrides: &[(String, String)],
) -> Result<(Vec<ClassificationResult>, u32), rusqlite::Error> {
    classify_batch_inner(store_conn, tracks, overrides, false)
}

fn classify_batch_inner(
    store_conn: &Connection,
    tracks: &[Track],
    overrides: &[(String, String)],
    load_profiles: bool,
) -> Result<(Vec<ClassificationResult>, u32), rusqlite::Error> {
    // Pre-compute normalized keys and resolved audio paths.
    let current_audio_identities = audio_cache_identities_with_current_stratum_input(
        tracks.iter().map(|track| track.file_path.as_str()),
    );
    let norm_keys: Vec<_> = tracks
        .iter()
        .zip(current_audio_identities)
        .map(|(t, audio_identity)| {
            let a = normalize::normalize_for_matching(&t.artist);
            let ti = normalize::normalize_for_matching(&t.title);
            let al = normalize::normalize_for_matching(&t.album);
            let audio_key = audio_identity
                .as_ref()
                .map(|identity| identity.cache_key.clone())
                .unwrap_or_else(|| resolved_audio_cache_key(&t.file_path));
            (
                a,
                ti,
                (!al.is_empty()).then_some(al),
                audio_key,
                audio_identity,
            )
        })
        .collect();

    // Build batch keys.
    let mut enrich_keys: Vec<(&str, &str, &str, &str)> = Vec::with_capacity(tracks.len());
    let stratum_identities: Vec<_> = norm_keys
        .iter()
        .filter_map(|(_, _, _, _, identity)| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = norm_keys
        .iter()
        .filter_map(|(_, _, _, _, identity)| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    for (a, t, al, _, _) in &norm_keys {
        let album = al.as_deref().unwrap_or("");
        enrich_keys.push(("discogs", a, t, album));
    }

    // Batch load — 3 queries total instead of 4N.
    let (enrich_map, stratum_map, essentia_map, profile_registry) = {
        let enrich_map = state::batch_get_enrichment(store_conn, &enrich_keys)?;
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
        let registry = if load_profiles {
            state::classification::load_from_db(store_conn, None)?.registry
        } else {
            None
        };
        (enrich_map, stratum_map, essentia_map, registry)
    };

    let mut results = Vec::with_capacity(tracks.len());

    for (track, (norm_artist, norm_title, norm_album, audio_key, _)) in
        tracks.iter().zip(&norm_keys)
    {
        let album = norm_album.as_deref().unwrap_or("");
        let discogs_key = (
            "discogs".to_string(),
            norm_artist.clone(),
            norm_title.clone(),
            album.to_string(),
        );
        let discogs_cache = enrich_map.get(&discogs_key);
        let stratum_cache = stratum_map.get(audio_key);
        let essentia_cache = essentia_map.get(audio_key);

        let evidence = build_track_evidence(
            track,
            discogs_cache,
            stratum_cache,
            essentia_cache,
            overrides,
        );
        results.push(classify_track_with_profiles(
            &evidence,
            profile_registry.as_ref(),
        ));
    }

    Ok((results, 0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::{ClassificationAction, ClassificationConfidence};
    use crate::domain::library::FileKind;

    fn track() -> Track {
        Track {
            id: "boundary-track".to_string(),
            title: "Signal Path".to_string(),
            artist: "Boundary Artist".to_string(),
            album: String::new(),
            genre: String::new(),
            bpm: 132.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 0,
            file_path: "/missing/application-boundary.flac".to_string(),
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    #[test]
    fn in_memory_state_evidence_reaches_domain_classifier_without_mcp_types() {
        let conn = Connection::open_in_memory().unwrap();
        state::migrate(&conn).unwrap();
        let track = track();
        let artist = normalize::normalize_for_matching(&track.artist);
        let title = normalize::normalize_for_matching(&track.title);
        state::set_enrichment(
            &conn,
            "discogs",
            &artist,
            &title,
            None,
            Some("exact"),
            Some(r#"{"styles":["Techno"]}"#),
        )
        .unwrap();

        let (results, cache_errors) = classify_batch(&conn, &[track], &[]).unwrap();

        assert_eq!(cache_errors, 0);
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert_eq!(result.genre, Some("Techno"));
        assert_ne!(result.confidence, ClassificationConfidence::Insufficient);
        assert_eq!(result.action, ClassificationAction::Suggest);
        assert!(
            result
                .evidence
                .iter()
                .any(|line| line.contains("discogs") && line.contains("Techno")),
            "provider evidence should survive state loading and evidence construction"
        );
    }
}
