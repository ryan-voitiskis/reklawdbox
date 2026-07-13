use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClassificationConfidence {
    High,
    Medium,
    Low,
    Insufficient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ClassificationAction {
    /// Recommendation matches current genre — no change needed.
    Confirm,
    /// Recommendation differs from current genre.
    Conflict,
    /// Current genre is empty — suggesting a new genre.
    Suggest,
    /// Insufficient evidence for recommendation — needs human review.
    Manual,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GenreCandidate {
    pub(crate) genre: &'static str,
    pub(crate) score: f32,
    pub(crate) bpm_plausible: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub(crate) chosen: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ClassificationResult {
    pub(crate) track_id: String,
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) current_genre: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) genre: Option<&'static str>,
    pub(crate) confidence: ClassificationConfidence,
    pub(crate) action: ClassificationAction,
    pub(crate) evidence: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) candidates: Vec<GenreCandidate>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) flags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) review_hint: Option<String>,
}

/// Minimal view for roster collection — omits evidence, candidates, flags,
/// and review_hint.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompactClassificationResult {
    pub(crate) track_id: String,
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) current_genre: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) genre: Option<&'static str>,
    pub(crate) confidence: ClassificationConfidence,
    pub(crate) action: ClassificationAction,
}

impl ClassificationResult {
    /// Destructured so adding a field to [`ClassificationResult`] produces a
    /// compile error here, forcing a conscious decision about the compact view.
    pub(crate) fn to_compact(&self) -> CompactClassificationResult {
        let ClassificationResult {
            ref track_id,
            ref artist,
            ref title,
            ref current_genre,
            genre,
            confidence,
            action,
            evidence: _,
            candidates: _,
            flags: _,
            review_hint: _,
        } = *self;
        CompactClassificationResult {
            track_id: track_id.clone(),
            artist: artist.clone(),
            title: title.clone(),
            current_genre: current_genre.clone(),
            genre,
            confidence,
            action,
        }
    }
}

/// Mapped genre from an enrichment source.
pub(crate) struct MappedGenre {
    pub(crate) genre: &'static str,
    pub(crate) style_count: usize,
}

/// Pre-extracted audio features from cache.
#[derive(Clone)]
pub(crate) struct AudioFeatures {
    pub(crate) rekordbox_bpm: f64,
    pub(crate) stratum_bpm: Option<f64>,
    pub(crate) bpm_agreement: Option<bool>,
    pub(crate) essentia_bpm: Option<f64>,
    pub(crate) duration_seconds: Option<f64>,
    pub(crate) danceability: Option<f64>,
    pub(crate) dynamic_complexity: Option<f64>,
    pub(crate) rhythm_regularity: Option<f64>,
    pub(crate) spectral_centroid_mean: Option<f64>,
    // Scalar features for Genre Audio Profiles (Item 6).
    #[allow(dead_code)]
    pub(crate) decay_mid_tau: Option<f64>,
    #[allow(dead_code)]
    pub(crate) decay_high_tau: Option<f64>,
    #[allow(dead_code)]
    pub(crate) onset_rate: Option<f64>,
    #[allow(dead_code)]
    pub(crate) loudness_integrated: Option<f64>,
    pub(crate) loudness_range: Option<f64>,
    #[allow(dead_code)]
    pub(crate) spectral_centroid_cv: Option<f64>,
    #[allow(dead_code)]
    pub(crate) spectral_flux_mean: Option<f64>,
    #[allow(dead_code)]
    pub(crate) dissonance_mean: Option<f64>,
    #[allow(dead_code)]
    pub(crate) key_clarity: Option<f64>,
    /// Stratum's tonal-content confidence. `0.0` is a sentinel for "detection
    /// failed"; values in `(0.0, 0.1)` indicate atonal/noise-dominated material.
    pub(crate) key_confidence: Option<f64>,
    #[allow(dead_code)]
    pub(crate) kick_pattern: Option<String>,
    #[allow(dead_code)]
    pub(crate) kick_pattern_confidence: Option<f64>,
    #[allow(dead_code)]
    pub(crate) kick_kicks_per_bar: Option<f64>,
    #[allow(dead_code)]
    pub(crate) kick_onset_count: Option<u32>,
    #[allow(dead_code)]
    pub(crate) kick_rate_basis: Option<String>,
    #[allow(dead_code)]
    pub(crate) kick_histogram: Option<Vec<f64>>,
    // Vector features for timbral distances (Item 6).
    #[allow(dead_code)]
    pub(crate) mfcc_mean: Option<Vec<f64>>,
    #[allow(dead_code)]
    pub(crate) mfcc_std: Option<Vec<f64>>,
    #[allow(dead_code)]
    pub(crate) spectral_contrast_mean: Option<Vec<f64>>,
}

/// All inputs needed for classification of a single track.
pub(crate) struct TrackEvidence {
    pub(crate) track_id: String,
    pub(crate) artist: String,
    pub(crate) title: String,
    pub(crate) current_genre: String,
    /// Rekordbox BPM — always available from the DB, independent of audio analysis.
    pub(crate) bpm: f64,
    pub(crate) discogs_mapped: Vec<MappedGenre>,
    pub(crate) beatport_genre: Option<&'static str>,
    pub(crate) beatport_raw: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) label_genre: Option<&'static str>,
    pub(crate) audio: Option<AudioFeatures>,
    pub(crate) has_discogs: bool,
    pub(crate) has_beatport: bool,
    pub(crate) has_audio: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_result_serializes_exact_wire_shape() {
        let result = ClassificationResult {
            track_id: "track-1".into(),
            artist: "Artist".into(),
            title: "Title".into(),
            current_genre: "House".into(),
            genre: Some("Techno"),
            confidence: ClassificationConfidence::High,
            action: ClassificationAction::Conflict,
            evidence: vec!["beatport: Techno".into()],
            candidates: vec![GenreCandidate {
                genre: "Techno",
                score: 2.5,
                bpm_plausible: true,
                chosen: false,
            }],
            flags: Vec::new(),
            review_hint: None,
        };

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "track_id": "track-1",
                "artist": "Artist",
                "title": "Title",
                "current_genre": "House",
                "genre": "Techno",
                "confidence": "high",
                "action": "conflict",
                "evidence": ["beatport: Techno"],
                "candidates": [{
                    "genre": "Techno",
                    "score": 2.5,
                    "bpm_plausible": true
                }]
            })
        );
    }
}
