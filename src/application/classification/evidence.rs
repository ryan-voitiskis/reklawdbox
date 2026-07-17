//! Build domain classification evidence from cached provider and audio records.

use std::collections::HashMap;

use serde::Deserialize;
use tracing::warn;

use crate::adapters::{
    audio,
    state::{CachedAudioAnalysis, EnrichmentCacheEntry},
};
use crate::domain::classification::taxonomy::map_genre_through_taxonomy;
use crate::domain::{
    classification::{
        AudioBackendStatus, AudioFeatures, ClassificationDegradedReason, ClassificationMode,
        DiscogsMatchQuality, DiscogsReadiness, LabelProvenance, MappedGenre, TrackEvidence,
        classification_readiness, taxonomy as genre,
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

pub(crate) struct ClassificationAudioEvidence {
    pub(crate) features: Option<AudioFeatures>,
    pub(crate) stratum_status: AudioBackendStatus,
    pub(crate) essentia_status: AudioBackendStatus,
}

/// Classification-local Stratum view. Optional fields preserve a valid sparse
/// payload, while serde still rejects wrong field types instead of silently
/// erasing them as missing evidence.
#[derive(Debug, Deserialize)]
struct ClassificationStratumOutput {
    #[serde(default)]
    bpm: Option<f64>,
    #[serde(default)]
    duration_seconds: Option<f64>,
    #[serde(default)]
    decay_mid_tau: Option<f64>,
    #[serde(default)]
    decay_high_tau: Option<f64>,
    #[serde(default)]
    key_clarity: Option<f64>,
    #[serde(default)]
    key_confidence: Option<f64>,
    #[serde(default)]
    kick_pattern: Option<String>,
    #[serde(default)]
    kick_pattern_confidence: Option<f64>,
    #[serde(default)]
    kick_kicks_per_bar: Option<f64>,
    #[serde(default)]
    kick_onset_count: Option<u32>,
    #[serde(default)]
    kick_rate_basis: Option<String>,
    #[serde(default)]
    kick_histogram: Option<Vec<f64>>,
}

impl ClassificationAudioEvidence {
    pub(crate) fn readiness(&self) -> (ClassificationMode, Vec<ClassificationDegradedReason>) {
        classification_readiness(self.stratum_status, self.essentia_status)
    }
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

    let audio_evidence = extract_classification_audio(track, stratum_cache, essentia_cache);
    let has_audio = audio_evidence.features.is_some();

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
        audio: audio_evidence.features,
        has_discogs: !matches!(discogs.readiness, DiscogsReadiness::NotSearched),
        discogs_match_quality: discogs.match_quality,
        has_audio,
        stratum_status: audio_evidence.stratum_status,
        essentia_status: audio_evidence.essentia_status,
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

pub(crate) fn extract_classification_audio(
    track: &Track,
    stratum_cache: Option<&CachedAudioAnalysis>,
    essentia_cache: Option<&CachedAudioAnalysis>,
) -> ClassificationAudioEvidence {
    let (stratum_data, stratum_status) = match stratum_cache {
        None => (None, AudioBackendStatus::Missing),
        Some(cache) => {
            match serde_json::from_str::<ClassificationStratumOutput>(&cache.features_json) {
                Ok(value) => (Some(value), AudioBackendStatus::Fresh),
                Err(error) => {
                    warn!(
                        file = track.file_path.as_str(),
                        "Stratum features_json failed to parse: {error}"
                    );
                    (None, AudioBackendStatus::Invalid)
                }
            }
        }
    };
    let (essentia_data, essentia_status) = match essentia_cache {
        None => (None, AudioBackendStatus::Missing),
        Some(cache) => match serde_json::from_str::<audio::EssentiaOutput>(&cache.features_json) {
            Ok(value) => match audio::validate_runtime_manifest(&value) {
                Ok(()) => (Some(value), AudioBackendStatus::Fresh),
                Err(error) => {
                    warn!(
                        file = track.file_path.as_str(),
                        "Essentia features_json failed runtime validation: {error}"
                    );
                    (None, AudioBackendStatus::Invalid)
                }
            },
            Err(error) => {
                warn!(
                    file = track.file_path.as_str(),
                    "Essentia features_json failed to parse: {error}"
                );
                (None, AudioBackendStatus::Invalid)
            }
        },
    };

    if stratum_data.is_none() && essentia_data.is_none() {
        return ClassificationAudioEvidence {
            features: None,
            stratum_status,
            essentia_status,
        };
    }

    let stratum_bpm = stratum_data.as_ref().and_then(|data| data.bpm);
    let bpm_agreement = stratum_bpm.map(|sb| (sb - track.bpm).abs() <= 2.0);

    let features = AudioFeatures {
        rekordbox_bpm: track.bpm,
        stratum_bpm,
        bpm_agreement,
        essentia_bpm: essentia_data.as_ref().and_then(|e| e.bpm_essentia),
        duration_seconds: stratum_data.as_ref().and_then(|data| data.duration_seconds),
        danceability: essentia_data.as_ref().and_then(|e| e.danceability),
        dynamic_complexity: essentia_data.as_ref().and_then(|e| e.dynamic_complexity),
        rhythm_regularity: essentia_data.as_ref().and_then(|e| e.rhythm_regularity),
        spectral_centroid_mean: essentia_data
            .as_ref()
            .and_then(|e| e.spectral_centroid_mean),
        // Scalar features from Stratum
        decay_mid_tau: stratum_data.as_ref().and_then(|data| data.decay_mid_tau),
        decay_high_tau: stratum_data.as_ref().and_then(|data| data.decay_high_tau),
        key_clarity: stratum_data.as_ref().and_then(|data| data.key_clarity),
        key_confidence: stratum_data.as_ref().and_then(|data| data.key_confidence),
        kick_pattern: stratum_data
            .as_ref()
            .and_then(|data| data.kick_pattern.clone()),
        kick_pattern_confidence: stratum_data
            .as_ref()
            .and_then(|data| data.kick_pattern_confidence),
        kick_kicks_per_bar: stratum_data
            .as_ref()
            .and_then(|data| data.kick_kicks_per_bar),
        kick_onset_count: stratum_data.as_ref().and_then(|data| data.kick_onset_count),
        kick_rate_basis: stratum_data
            .as_ref()
            .and_then(|data| data.kick_rate_basis.clone()),
        kick_histogram: stratum_data
            .as_ref()
            .and_then(|data| data.kick_histogram.clone()),
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
    };
    ClassificationAudioEvidence {
        features: Some(features),
        stratum_status,
        essentia_status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::{ClassificationDegradedReason, ClassificationMode};
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

    fn audio_entry(analyzer: &str, features_json: impl Into<String>) -> CachedAudioAnalysis {
        CachedAudioAnalysis {
            file_path: "/missing/evidence.flac".into(),
            analyzer: analyzer.into(),
            file_size: 1,
            file_mtime: 2,
            analysis_version: "current".into(),
            input_fingerprint: String::new(),
            features_json: features_json.into(),
            created_at: String::new(),
        }
    }

    fn valid_essentia_entry(extra: serde_json::Value) -> CachedAudioAnalysis {
        let mut payload = serde_json::json!({
            "analyzer_version": audio::SUPPORTED_ESSENTIA_VERSION,
            "runtime_manifest": {
                "python_version": "3.14.6",
                "python_implementation": "cpython",
                "essentia_version": audio::SUPPORTED_ESSENTIA_VERSION,
                "essentia_module_version": audio::SUPPORTED_ESSENTIA_MODULE_VERSION,
                "numpy_version": audio::SUPPORTED_NUMPY_VERSION,
                "pyyaml_version": audio::SUPPORTED_PYYAML_VERSION,
                "six_version": audio::SUPPORTED_SIX_VERSION,
                "analyzer_contract": audio::ESSENTIA_CONTRACT_ID,
            },
        });
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        audio_entry(
            audio::ANALYZER_ESSENTIA,
            serde_json::to_string(&payload).unwrap(),
        )
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

    #[test]
    fn classification_audio_readiness_combination_matrix() {
        let track = track("");
        let stratum = audio_entry(audio::ANALYZER_STRATUM, r#"{"bpm":128.0}"#);
        let sparse_stratum = audio_entry(audio::ANALYZER_STRATUM, "{}");
        let essentia = valid_essentia_entry(serde_json::json!({"danceability": 2.0}));
        let sparse_essentia = valid_essentia_entry(serde_json::json!({
            "danceability": null,
            "onset_rate": null,
        }));
        let invalid_stratum = audio_entry(audio::ANALYZER_STRATUM, r#"{"bpm":"invalid"}"#);
        let invalid_essentia = audio_entry(audio::ANALYZER_ESSENTIA, "not-json");
        let mismatched_essentia = audio_entry(
            audio::ANALYZER_ESSENTIA,
            r#"{"analyzer_version":"wrong","runtime_manifest":{}}"#,
        );

        let cases = [
            (
                None,
                None,
                AudioBackendStatus::Missing,
                AudioBackendStatus::Missing,
                vec![
                    ClassificationDegradedReason::MissingStratum,
                    ClassificationDegradedReason::MissingEssentia,
                ],
            ),
            (
                Some(&stratum),
                None,
                AudioBackendStatus::Fresh,
                AudioBackendStatus::Missing,
                vec![ClassificationDegradedReason::MissingEssentia],
            ),
            (
                None,
                Some(&essentia),
                AudioBackendStatus::Missing,
                AudioBackendStatus::Fresh,
                vec![ClassificationDegradedReason::MissingStratum],
            ),
            (
                Some(&invalid_stratum),
                Some(&essentia),
                AudioBackendStatus::Invalid,
                AudioBackendStatus::Fresh,
                vec![ClassificationDegradedReason::InvalidStratum],
            ),
            (
                Some(&stratum),
                Some(&invalid_essentia),
                AudioBackendStatus::Fresh,
                AudioBackendStatus::Invalid,
                vec![ClassificationDegradedReason::InvalidEssentia],
            ),
            (
                Some(&invalid_stratum),
                Some(&invalid_essentia),
                AudioBackendStatus::Invalid,
                AudioBackendStatus::Invalid,
                vec![
                    ClassificationDegradedReason::InvalidStratum,
                    ClassificationDegradedReason::InvalidEssentia,
                ],
            ),
        ];

        for (stratum, essentia, expected_stratum, expected_essentia, expected_reasons) in cases {
            let extracted = extract_classification_audio(&track, stratum, essentia);
            assert_eq!(extracted.stratum_status, expected_stratum);
            assert_eq!(extracted.essentia_status, expected_essentia);
            assert_eq!(extracted.readiness().0, ClassificationMode::Degraded);
            assert_eq!(extracted.readiness().1, expected_reasons);
        }

        let complete = extract_classification_audio(&track, Some(&stratum), Some(&essentia));
        assert_eq!(complete.readiness().0, ClassificationMode::Full);
        assert!(complete.features.is_some());

        let sparse =
            extract_classification_audio(&track, Some(&sparse_stratum), Some(&sparse_essentia));
        assert_eq!(sparse.readiness().0, ClassificationMode::Full);
        let features = sparse.features.expect("sparse valid rows remain present");
        assert!(features.danceability.is_none());
        assert!(features.stratum_bpm.is_none());

        let mismatched =
            extract_classification_audio(&track, Some(&stratum), Some(&mismatched_essentia));
        assert_eq!(mismatched.essentia_status, AudioBackendStatus::Invalid);
        assert_eq!(mismatched.readiness().0, ClassificationMode::Degraded);
    }
}
