//! Build domain classification evidence from cached provider and audio records.

use std::collections::HashMap;

use tracing::warn;

use crate::adapters::{
    audio,
    state::{CachedAudioAnalysis, EnrichmentCacheEntry},
};
use crate::domain::classification::taxonomy::map_genre_through_taxonomy;
use crate::domain::{
    classification::{AudioFeatures, MappedGenre, TrackEvidence, taxonomy as genre},
    library::Track,
};

pub(crate) fn build_track_evidence(
    track: &Track,
    discogs_cache: Option<&EnrichmentCacheEntry>,
    beatport_cache: Option<&EnrichmentCacheEntry>,
    stratum_cache: Option<&CachedAudioAnalysis>,
    essentia_cache: Option<&CachedAudioAnalysis>,
    overrides: &[(String, String)],
) -> TrackEvidence {
    let discogs_val = parse_response_json(discogs_cache);
    let discogs_mapped = extract_discogs_genres(discogs_val.as_ref(), overrides);

    let beatport_val = parse_response_json(beatport_cache);
    let (beatport_genre, beatport_raw) = extract_beatport_genre(beatport_val.as_ref(), overrides);

    let effective_label = if !track.label.is_empty() {
        Some(track.label.clone())
    } else {
        discogs_val
            .as_ref()
            .and_then(|v| v.get("label"))
            .and_then(|v| v.as_str())
            .filter(|l| !l.is_empty())
            .map(std::string::ToString::to_string)
    };
    let label_genre_val = effective_label.as_deref().and_then(genre::label_genre);

    let audio = extract_audio_features(track, stratum_cache, essentia_cache);
    let has_audio = audio.is_some();

    TrackEvidence {
        track_id: track.id.clone(),
        artist: track.artist.clone(),
        title: track.title.clone(),
        current_genre: track.genre.clone(),
        bpm: track.bpm,
        discogs_mapped,
        beatport_genre,
        beatport_raw,
        label: effective_label,
        label_genre: label_genre_val,
        audio,
        has_discogs: discogs_cache.is_some(),
        has_beatport: beatport_cache.is_some(),
        has_audio,
    }
}

pub(crate) fn parse_response_json(
    cache: Option<&EnrichmentCacheEntry>,
) -> Option<serde_json::Value> {
    cache.and_then(|c| {
        c.response_json.as_ref().and_then(|json_str| {
            match serde_json::from_str::<serde_json::Value>(json_str) {
                Ok(val) => Some(val),
                Err(e) => {
                    warn!(
                        provider = c.provider.as_str(),
                        artist = c.query_artist.as_str(),
                        title = c.query_title.as_str(),
                        "Cached response_json failed to parse: {e}"
                    );
                    None
                }
            }
        })
    })
}

fn apply_override(raw: &str, overrides: &[(String, String)]) -> Option<String> {
    let lower = raw.trim().to_ascii_lowercase();
    overrides
        .iter()
        .find(|(from, _)| *from == lower)
        .map(|(_, to)| to.clone())
}

fn extract_discogs_genres(
    discogs_val: Option<&serde_json::Value>,
    overrides: &[(String, String)],
) -> Vec<MappedGenre> {
    let Some(styles) = discogs_val
        .and_then(|v| v.get("styles"))
        .and_then(|v| v.as_array())
    else {
        return vec![];
    };

    let mut genre_counts: HashMap<&'static str, usize> = HashMap::new();

    for style in styles.iter().filter_map(|s| s.as_str()) {
        if let Some(override_genre) = apply_override(style, overrides) {
            if let Some(canonical) = genre::canonical_genre_name(&override_genre) {
                *genre_counts.entry(canonical).or_insert(0) += 1;
                continue;
            } else {
                warn!(
                    from = style,
                    to = override_genre.as_str(),
                    "Genre override target is not a canonical genre — override ignored"
                );
            }
        }
        let (maps_to, mapping_type) = map_genre_through_taxonomy(style);
        if mapping_type != "unknown"
            && let Some(genre_name) = maps_to
            && let Some(canonical) = genre::canonical_genre_name(&genre_name)
        {
            *genre_counts.entry(canonical).or_insert(0) += 1;
        }
    }

    genre_counts
        .into_iter()
        .map(|(genre, style_count)| MappedGenre { genre, style_count })
        .collect()
}

fn extract_beatport_genre(
    beatport_val: Option<&serde_json::Value>,
    overrides: &[(String, String)],
) -> (Option<&'static str>, Option<String>) {
    let raw_str = beatport_val
        .and_then(|v| v.get("genre"))
        .and_then(|v| v.as_str())
        .filter(|g| !g.is_empty());

    let Some(raw) = raw_str else {
        return (None, None);
    };

    if let Some(override_genre) = apply_override(raw, overrides) {
        if let Some(canonical) = genre::canonical_genre_name(&override_genre) {
            return (Some(canonical), Some(raw.to_string()));
        } else {
            warn!(
                from = raw,
                to = override_genre.as_str(),
                "Genre override target is not a canonical genre — override ignored"
            );
        }
    }

    let (maps_to, mapping_type) = map_genre_through_taxonomy(raw);
    let canonical = if mapping_type != "unknown" && maps_to.is_some() {
        maps_to.and_then(|g| genre::canonical_genre_name(&g))
    } else {
        None
    };

    (canonical, Some(raw.to_string()))
}

pub(crate) fn extract_audio_features(
    track: &Track,
    stratum_cache: Option<&CachedAudioAnalysis>,
    essentia_cache: Option<&CachedAudioAnalysis>,
) -> Option<AudioFeatures> {
    let stratum_json = stratum_cache.and_then(|sc| {
        match serde_json::from_str::<serde_json::Value>(&sc.features_json) {
            Ok(val) => Some(val),
            Err(e) => {
                warn!(
                    file = track.file_path.as_str(),
                    "Stratum features_json failed to parse: {e}"
                );
                None
            }
        }
    });
    let essentia_data = essentia_cache.and_then(|ec| {
        match serde_json::from_str::<audio::EssentiaOutput>(&ec.features_json) {
            Ok(val) => Some(val),
            Err(e) => {
                warn!(
                    file = track.file_path.as_str(),
                    "Essentia features_json failed to parse: {e}"
                );
                None
            }
        }
    });

    if stratum_json.is_none() && essentia_data.is_none() {
        return None;
    }

    let stratum_bpm = stratum_json
        .as_ref()
        .and_then(|sj| sj.get("bpm"))
        .and_then(serde_json::Value::as_f64);
    let bpm_agreement = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);

    Some(AudioFeatures {
        rekordbox_bpm: track.bpm,
        stratum_bpm,
        bpm_agreement,
        essentia_bpm: essentia_data.as_ref().and_then(|e| e.bpm_essentia),
        duration_seconds: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("duration_seconds"))
            .and_then(serde_json::Value::as_f64),
        danceability: essentia_data.as_ref().and_then(|e| e.danceability),
        dynamic_complexity: essentia_data.as_ref().and_then(|e| e.dynamic_complexity),
        rhythm_regularity: essentia_data.as_ref().and_then(|e| e.rhythm_regularity),
        spectral_centroid_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_centroid_mean),
        // Scalar features from Stratum
        decay_mid_tau: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("decay_mid_tau"))
            .and_then(serde_json::Value::as_f64),
        decay_high_tau: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("decay_high_tau"))
            .and_then(serde_json::Value::as_f64),
        key_clarity: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("key_clarity"))
            .and_then(serde_json::Value::as_f64),
        key_confidence: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("key_confidence"))
            .and_then(serde_json::Value::as_f64),
        kick_pattern: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_pattern"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        kick_pattern_confidence: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_pattern_confidence"))
            .and_then(serde_json::Value::as_f64),
        kick_kicks_per_bar: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_kicks_per_bar"))
            .and_then(serde_json::Value::as_f64),
        kick_onset_count: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_onset_count"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok()),
        kick_rate_basis: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_rate_basis"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        kick_histogram: stratum_json
            .as_ref()
            .and_then(|sj| sj.get("kick_histogram"))
            .and_then(serde_json::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(serde_json::Value::as_f64)
                    .collect()
            }),
        // Scalar features from Essentia
        onset_rate: essentia_data.as_ref().and_then(|e| e.onset_rate),
        loudness_integrated: essentia_data.as_ref().and_then(|e| e.loudness_integrated),
        loudness_range: essentia_data.as_ref().and_then(|e| e.loudness_range),
        spectral_centroid_cv: essentia_data.as_ref().and_then(|e| e.spectral_centroid_cv),
        spectral_flux_mean: essentia_data.as_ref().and_then(|e| e.spectral_flux_mean),
        dissonance_mean: essentia_data.as_ref().and_then(|e| e.dissonance_mean),
        // Vector features from Essentia (for timbral distances)
        mfcc_mean: essentia_data.as_ref().and_then(|e| e.mfcc_mean.clone()),
        mfcc_std: essentia_data.as_ref().and_then(|e| e.mfcc_std.clone()),
        spectral_contrast_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_contrast_mean.clone()),
    })
}
