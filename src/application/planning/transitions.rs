//! Transition-profile and weight orchestration.

use rusqlite::Connection;
use serde::Deserialize;

use crate::adapters::state::{self as state, CachedAudioAnalysis};
use crate::application::analysis::identity::{
    AudioCacheIdentity, audio_cache_identities_with_current_stratum_input,
};
use crate::domain::classification::taxonomy::GenreFamily;
use crate::domain::planning::{
    EnergyPhase, HarmonicMixingStyle, PriorityWeights, ScoringContext, SequencingPriority,
    TimbralFeatures, TrackProfile, TransitionScores, canonicalize_genre, compute_track_energy,
    format_camelot, genre_family_for, key_to_camelot, parse_camelot_key, priority_weights,
    renormalize_transition, score_transition_profiles,
};

pub(crate) fn build_track_profile(
    track: crate::types::Track,
    store_conn: &Connection,
) -> Result<TrackProfile, String> {
    build_track_profiles(vec![track], store_conn)?
        .pop()
        .ok_or_else(|| "Track profile batch unexpectedly returned no result".to_string())
}

pub(crate) fn build_track_profiles(
    tracks: Vec<crate::types::Track>,
    store_conn: &Connection,
) -> Result<Vec<TrackProfile>, String> {
    let identities = audio_cache_identities_with_current_stratum_input(
        tracks.iter().map(|track| track.file_path.as_str()),
    );
    let stratum_identities: Vec<_> = identities
        .iter()
        .filter_map(|identity| identity.as_ref()?.as_stratum_store_identity())
        .collect();
    let essentia_identities: Vec<_> = identities
        .iter()
        .filter_map(|identity| {
            identity
                .as_ref()
                .map(AudioCacheIdentity::as_essentia_store_identity)
        })
        .collect();
    let stratum = state::batch_get_fresh_audio_analysis(
        store_conn,
        &stratum_identities,
        crate::audio::ANALYZER_STRATUM,
        crate::audio::STRATUM_SCHEMA_VERSION,
    )
    .map_err(|error| format!("Stratum cache read error: {error}"))?;
    let essentia = state::batch_get_fresh_audio_analysis(
        store_conn,
        &essentia_identities,
        crate::audio::ANALYZER_ESSENTIA,
        crate::audio::ESSENTIA_SCHEMA_VERSION,
    )
    .map_err(|error| format!("Essentia cache read error: {error}"))?;

    Ok(tracks
        .into_iter()
        .zip(identities)
        .map(|(track, identity)| {
            let cache_key = identity
                .as_ref()
                .map(|identity| identity.cache_key.as_str());
            build_track_profile_from_cache(
                track,
                cache_key.and_then(|key| stratum.get(key)),
                cache_key.and_then(|key| essentia.get(key)),
            )
        })
        .collect())
}

fn build_track_profile_from_cache(
    track: crate::types::Track,
    stratum_entry: Option<&CachedAudioAnalysis>,
    essentia_entry: Option<&CachedAudioAnalysis>,
) -> TrackProfile {
    let stratum_json = stratum_entry
        .and_then(|cached| serde_json::from_str::<serde_json::Value>(&cached.features_json).ok());
    let essentia_data = essentia_entry.and_then(|cached| {
        serde_json::from_str::<crate::audio::EssentiaOutput>(&cached.features_json).ok()
    });

    // Prefer Rekordbox BPM — it's the value the DJ sees and can manually correct.
    // Fall back to stratum-dsp's estimate for tracks Rekordbox hasn't analyzed.
    // (Key uses the opposite strategy: stratum preferred, Rekordbox fallback.)
    const BPM_PLAUSIBLE_MIN: f64 = 30.0;
    let bpm = if track.bpm >= BPM_PLAUSIBLE_MIN {
        track.bpm
    } else {
        stratum_json
            .as_ref()
            .and_then(|value| value.get("bpm"))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0)
    };

    let camelot_key = stratum_json
        .as_ref()
        .and_then(|value| value.get("key_camelot").and_then(serde_json::Value::as_str))
        .and_then(parse_camelot_key)
        .or_else(|| key_to_camelot(&track.key));

    let key_display = camelot_key.map_or_else(
        || match track.key.trim() {
            "" => "Unknown".to_string(),
            _ => track.key.clone(),
        },
        format_camelot,
    );

    let energy = compute_track_energy(
        essentia_data.as_ref().map(|essentia| {
            (
                essentia.danceability,
                essentia.loudness_integrated,
                essentia.onset_rate,
            )
        }),
        bpm,
    );
    let brightness = essentia_data
        .as_ref()
        .and_then(|essentia| essentia.spectral_centroid_mean);
    let rhythm_regularity = essentia_data
        .as_ref()
        .and_then(|essentia| essentia.rhythm_regularity);
    let loudness_range = essentia_data
        .as_ref()
        .and_then(|essentia| essentia.loudness_range);
    let canonical_genre = canonicalize_genre(&track.genre);
    let genre_family = canonical_genre
        .as_deref()
        .map_or(GenreFamily::Other, genre_family_for);

    let timbral = essentia_data.as_ref().and_then(|essentia| {
        Some(TimbralFeatures {
            mfcc_mean: essentia.mfcc_mean.clone()?,
            mfcc_std: essentia.mfcc_std.clone()?,
            spectral_contrast_mean: essentia.spectral_contrast_mean.clone()?,
            spectral_centroid_cv: essentia.spectral_centroid_cv?,
            dissonance_mean: essentia.dissonance_mean?,
        })
    });

    TrackProfile {
        track,
        camelot_key,
        key_display,
        bpm,
        energy,
        brightness,
        rhythm_regularity,
        loudness_range,
        canonical_genre,
        genre_family,
        timbral,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTransitionWeights {
    key: Option<f64>,
    bpm: Option<f64>,
    energy: Option<f64>,
    genre: Option<f64>,
    brightness: Option<f64>,
    rhythm: Option<f64>,
}

fn saved_transition_weights(input: SavedTransitionWeights) -> Result<PriorityWeights, String> {
    let base = priority_weights(SequencingPriority::Balanced);
    let mut weights = PriorityWeights {
        key: input.key.unwrap_or(base.key),
        bpm: input.bpm.unwrap_or(base.bpm),
        energy: input.energy.unwrap_or(base.energy),
        genre: input.genre.unwrap_or(base.genre),
        brightness: input.brightness.unwrap_or(base.brightness),
        rhythm: input.rhythm.unwrap_or(base.rhythm),
    };
    renormalize_transition(&mut weights)?;
    Ok(weights)
}

fn transition_builtin(name: &str) -> Option<PriorityWeights> {
    let priority = match name {
        "balanced" => SequencingPriority::Balanced,
        "harmonic" => SequencingPriority::Harmonic,
        "energy" => SequencingPriority::Energy,
        "genre" => SequencingPriority::Genre,
        _ => return None,
    };
    Some(priority_weights(priority))
}

fn resolve_transition_named_with_loader(
    name: &str,
    load_saved: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<PriorityWeights, String> {
    if let Some(weights) = transition_builtin(name) {
        return Ok(weights);
    }
    let json = load_saved()?.ok_or_else(|| {
        format!("Unknown transition preset '{name}'. Built-in: balanced, harmonic, energy, genre")
    })?;
    let input: SavedTransitionWeights =
        serde_json::from_str(&json).map_err(|error| format!("Invalid saved preset: {error}"))?;
    saved_transition_weights(input)
}

pub(crate) fn resolve_transition_named(
    name: &str,
    store: &Connection,
) -> Result<PriorityWeights, String> {
    resolve_transition_named_with_loader(name, || {
        state::get_weight_preset(store, name, "transition")
            .map_err(|error| format!("DB error: {error}"))
    })
}

pub(crate) struct TransitionEvaluation {
    pub(crate) from: TrackProfile,
    pub(crate) to: TrackProfile,
    pub(crate) scores: TransitionScores,
}

pub(crate) fn evaluate_transition(
    from_track: crate::types::Track,
    to_track: crate::types::Track,
    store: &Connection,
    phase: Option<EnergyPhase>,
    weights: &PriorityWeights,
    master_tempo: bool,
    harmonic_style: HarmonicMixingStyle,
) -> Result<TransitionEvaluation, String> {
    let mut profiles = build_track_profiles(vec![from_track, to_track], store)?;
    let to = profiles
        .pop()
        .expect("two input tracks produce two profiles");
    let from = profiles
        .pop()
        .expect("two input tracks produce two profiles");
    let scores = score_transition_profiles(
        &from,
        &to,
        phase,
        phase,
        weights,
        master_tempo,
        Some(harmonic_style),
        &ScoringContext::default(),
        None,
    );
    Ok(TransitionEvaluation { from, to, scores })
}

pub(crate) struct RankedTransitionCandidates {
    pub(crate) from: TrackProfile,
    pub(crate) candidates: Vec<(TrackProfile, TransitionScores)>,
    pub(crate) total_pool_size: usize,
    pub(crate) reference_bpm: f64,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rank_transition_candidates(
    from_track: crate::types::Track,
    pool_tracks: Vec<crate::types::Track>,
    store: &Connection,
    phase: Option<EnergyPhase>,
    weights: &PriorityWeights,
    master_tempo: bool,
    harmonic_style: HarmonicMixingStyle,
    target_bpm: Option<f64>,
    limit: usize,
) -> Result<RankedTransitionCandidates, String> {
    let from_id = from_track.id.clone();
    let mut tracks = Vec::with_capacity(pool_tracks.len() + 1);
    tracks.push(from_track);
    tracks.extend(pool_tracks.into_iter().filter(|track| track.id != from_id));
    let mut profiles = build_track_profiles(tracks, store)?;
    let from = profiles.remove(0);
    let reference_bpm = target_bpm.unwrap_or(from.bpm);
    let play_bpms = target_bpm.map(|target| (from.bpm, target));
    let context = ScoringContext::default();
    let mut candidates: Vec<_> = profiles
        .into_iter()
        .map(|to| {
            let scores = score_transition_profiles(
                &from,
                &to,
                phase,
                phase,
                weights,
                master_tempo,
                Some(harmonic_style),
                &context,
                play_bpms,
            );
            (to, scores)
        })
        .collect();
    candidates.sort_by(|left, right| {
        right
            .1
            .composite
            .partial_cmp(&left.1.composite)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.track.id.cmp(&right.0.track.id))
    });
    let total_pool_size = candidates.len();
    candidates.truncate(limit);
    Ok(RankedTransitionCandidates {
        from,
        candidates,
        total_pool_size,
        reference_bpm,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn planning_transition_use_case_preserves_preset_precedence() {
        let loaded = Cell::new(false);
        let weights = resolve_transition_named_with_loader("balanced", || {
            loaded.set(true);
            Ok(Some(r#"{"key":1.0}"#.to_string()))
        })
        .unwrap();
        assert!(!loaded.get());
        assert_eq!(weights.key, 0.30);
    }
}
