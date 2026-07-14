//! Build domain classification evidence from cached provider and audio records.

use std::collections::HashMap;

use tracing::warn;

use crate::adapters::{
    audio,
    state::{CachedAudioAnalysis, EnrichmentCacheEntry},
};
use crate::domain::classification::taxonomy::map_genre_through_taxonomy;
use crate::domain::{
    classification::{
        AudioFeatures, DiscogsMatchQuality, DiscogsReadiness, LabelProvenance, MappedGenre,
        TrackEvidence, taxonomy as genre,
    },
    library::Track,
    metadata as normalize,
};

#[derive(Debug)]
pub(crate) struct DiscogsInterpretation {
    pub(crate) readiness: DiscogsReadiness,
    pub(crate) match_quality: Option<DiscogsMatchQuality>,
    pub(crate) mapped_genres: Vec<MappedGenre>,
    pub(crate) label: Option<String>,
    pub(crate) diagnostic: Option<String>,
}

pub(crate) fn build_track_evidence(
    track: &Track,
    discogs_cache: Option<&EnrichmentCacheEntry>,
    stratum_cache: Option<&CachedAudioAnalysis>,
    essentia_cache: Option<&CachedAudioAnalysis>,
    overrides: &[(String, String)],
) -> TrackEvidence {
    let discogs = interpret_discogs(discogs_cache, overrides);

    let (effective_label, label_provenance) = if !track.label.is_empty() {
        let correlated = discogs.label.as_deref().is_some_and(|discogs_label| {
            normalize::normalize_for_matching(discogs_label)
                == normalize::normalize_for_matching(&track.label)
        });
        (
            Some(track.label.clone()),
            Some(if correlated {
                LabelProvenance::CorrelatedDiscogs
            } else {
                LabelProvenance::Rekordbox
            }),
        )
    } else if let Some(label) = discogs.label.clone() {
        (Some(label), Some(LabelProvenance::Discogs))
    } else {
        (None, None)
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
        discogs_mapped: discogs.mapped_genres,
        label: effective_label,
        label_genre: label_genre_val,
        label_provenance,
        audio,
        has_discogs: !matches!(discogs.readiness, DiscogsReadiness::NotSearched),
        discogs_match_quality: discogs.match_quality,
        has_audio,
    }
}

/// Interpret one exact, album-aware Discogs cache row into the shared genre
/// readiness state. Unknown qualities and malformed payloads fail closed.
pub(crate) fn interpret_discogs(
    cache: Option<&EnrichmentCacheEntry>,
    overrides: &[(String, String)],
) -> DiscogsInterpretation {
    let Some(cache) = cache else {
        return DiscogsInterpretation {
            readiness: DiscogsReadiness::NotSearched,
            match_quality: None,
            mapped_genres: Vec::new(),
            label: None,
            diagnostic: None,
        };
    };

    match cache.match_quality.as_deref() {
        Some("none") => DiscogsInterpretation {
            readiness: DiscogsReadiness::NoMatch,
            match_quality: None,
            mapped_genres: Vec::new(),
            label: None,
            diagnostic: None,
        },
        Some(quality @ ("exact" | "fuzzy")) => {
            let match_quality = if quality == "exact" {
                DiscogsMatchQuality::Exact
            } else {
                DiscogsMatchQuality::Fuzzy
            };
            let Some(raw) = cache.response_json.as_deref() else {
                return DiscogsInterpretation {
                    readiness: DiscogsReadiness::MatchedUnmapped,
                    match_quality: Some(match_quality),
                    mapped_genres: Vec::new(),
                    label: None,
                    diagnostic: Some("discogs-response-missing".into()),
                };
            };
            let value = match serde_json::from_str::<serde_json::Value>(raw) {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        artist = cache.query_artist.as_str(),
                        title = cache.query_title.as_str(),
                        "Cached Discogs response_json failed to parse: {error}"
                    );
                    return DiscogsInterpretation {
                        readiness: DiscogsReadiness::MatchedUnmapped,
                        match_quality: Some(match_quality),
                        mapped_genres: Vec::new(),
                        label: None,
                        diagnostic: Some("discogs-response-invalid".into()),
                    };
                }
            };
            let mapped_genres = extract_discogs_genres(Some(&value), overrides);
            let label = value
                .get("label")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .map(str::to_string);
            DiscogsInterpretation {
                readiness: if mapped_genres.is_empty() {
                    DiscogsReadiness::MatchedUnmapped
                } else {
                    DiscogsReadiness::UsableGenre
                },
                match_quality: Some(match_quality),
                mapped_genres,
                label,
                diagnostic: None,
            }
        }
        unknown => DiscogsInterpretation {
            readiness: DiscogsReadiness::MatchedUnmapped,
            match_quality: Some(DiscogsMatchQuality::Invalid),
            mapped_genres: Vec::new(),
            label: None,
            diagnostic: Some(format!(
                "discogs-match-quality-invalid:{}",
                unknown.unwrap_or("missing")
            )),
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::library::FileKind;

    fn entry(quality: Option<&str>, response: Option<&str>) -> EnrichmentCacheEntry {
        EnrichmentCacheEntry {
            provider: "discogs".into(),
            query_artist: "artist".into(),
            query_title: "title".into(),
            query_album: "album".into(),
            match_quality: quality.map(str::to_string),
            response_json: response.map(str::to_string),
            created_at: String::new(),
        }
    }

    fn track(label: &str) -> Track {
        Track {
            id: "track-1".into(),
            title: "Title".into(),
            artist: "Artist".into(),
            album: "Album".into(),
            genre: String::new(),
            bpm: 128.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: label.into(),
            remixer: String::new(),
            year: 0,
            length: 0,
            file_path: "/missing/evidence.flac".into(),
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
    fn discogs_interpreter_distinguishes_readiness_states() {
        assert_eq!(
            interpret_discogs(None, &[]).readiness,
            DiscogsReadiness::NotSearched
        );
        assert_eq!(
            interpret_discogs(Some(&entry(Some("none"), None)), &[]).readiness,
            DiscogsReadiness::NoMatch
        );
        assert_eq!(
            interpret_discogs(
                Some(&entry(Some("exact"), Some(r#"{"styles":["Unknown"]}"#))),
                &[],
            )
            .readiness,
            DiscogsReadiness::MatchedUnmapped
        );
        let usable = interpret_discogs(
            Some(&entry(
                Some("fuzzy"),
                Some(r#"{"styles":["Techno"],"label":"Tresor"}"#),
            )),
            &[],
        );
        assert_eq!(usable.readiness, DiscogsReadiness::UsableGenre);
        assert_eq!(usable.match_quality, Some(DiscogsMatchQuality::Fuzzy));
        assert_eq!(usable.mapped_genres[0].genre, "Techno");
    }

    #[test]
    fn malformed_or_unknown_discogs_data_fails_closed() {
        let malformed = interpret_discogs(Some(&entry(Some("exact"), Some("not-json"))), &[]);
        assert_eq!(malformed.readiness, DiscogsReadiness::MatchedUnmapped);
        assert!(malformed.diagnostic.is_some());
        assert!(malformed.mapped_genres.is_empty());

        let unknown = interpret_discogs(
            Some(&entry(Some("surprise"), Some(r#"{"styles":["Techno"]}"#))),
            &[],
        );
        assert_eq!(unknown.match_quality, Some(DiscogsMatchQuality::Invalid));
        assert!(unknown.mapped_genres.is_empty());
    }

    #[test]
    fn library_label_duplicate_of_discogs_is_correlated() {
        let cached = entry(
            Some("exact"),
            Some(r#"{"styles":["Techno"],"label":"Same Label"}"#),
        );
        let correlated = build_track_evidence(&track("same label"), Some(&cached), None, None, &[]);
        assert_eq!(
            correlated.label_provenance,
            Some(LabelProvenance::CorrelatedDiscogs)
        );

        let distinct = build_track_evidence(&track("Other Label"), Some(&cached), None, None, &[]);
        assert_eq!(distinct.label_provenance, Some(LabelProvenance::Rekordbox));

        let fallback = build_track_evidence(&track(""), Some(&cached), None, None, &[]);
        assert_eq!(fallback.label_provenance, Some(LabelProvenance::Discogs));
    }
}
