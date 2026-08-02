//! Conservative broad-genre projection and selective parent consensus.
//!
//! This is separate from [`super::taxonomy::GenreFamily`], which groups genres
//! for mixing and fine-classifier decisions rather than user-facing parent
//! classification.

use super::{ClassificationConfidence, ClassificationMode, ClassificationResult};

pub(crate) const RULE_VERSION: &str = "broad-parent-consensus-v1";

/// Map a canonical fine genre to the frozen Plan 062 broad target.
pub(crate) fn broad_genre(canonical: &str) -> Option<&'static str> {
    match canonical {
        "Afro House" | "Deep House" | "Gospel House" | "House" | "Progressive House" => {
            Some("House")
        }
        "Ambient Techno" | "Deep Techno" | "Dub Techno" | "Hard Techno" | "Techno" => {
            Some("Techno")
        }
        "Hard Trance" | "Psytrance" | "Trance" => Some("Trance"),
        "2-Step Garage" | "Bassline" | "Future Garage" | "Garage" | "Speed Garage" | "UK Funky" => {
            Some("Garage")
        }
        "Breakbeat" | "Broken Beat" => Some("Breakbeat"),
        "Drum & Bass" | "Jungle" => Some("Drum & Bass"),
        "Dancehall" | "Dub" | "Reggae" => Some("Reggae"),
        "Disco" | "Italo Disco" => Some("Disco"),
        "Gabber" | "Happy Hardcore" | "Hardcore" | "Hardstyle" => Some("Hardcore"),
        "Downtempo" | "Trip-Hop" => Some("Downtempo"),
        "Italodance" | "Pop" | "Synth-pop" => Some("Pop"),
        "Acid" | "Ambient" | "Dubstep" | "EBM" | "Electro" | "Footwork" | "Grime" | "Highlife"
        | "Hip Hop" | "IDM" | "Jazz" | "Minimal" | "R&B" | "Rock" | "Tech House" => {
            super::taxonomy::canonical_genre_name(canonical)
        }
        "Experimental" => None,
        _ => None,
    }
}

pub(crate) fn unselective_projection(result: &ClassificationResult) -> Option<&'static str> {
    result.genre.and_then(broad_genre)
}

pub(crate) fn confident_projection(result: &ClassificationResult) -> Option<&'static str> {
    if result.mode != ClassificationMode::Full
        || !matches!(
            result.confidence,
            ClassificationConfidence::High | ClassificationConfidence::Medium
        )
    {
        return None;
    }
    unselective_projection(result)
}

/// Offer a broad target only when the frozen Plan 062 parent-consensus rule
/// finds no cross-parent or unmapped fine candidate.
pub(crate) fn parent_consensus(result: &ClassificationResult) -> Option<&'static str> {
    if result.mode != ClassificationMode::Full {
        return None;
    }
    let final_broad = unselective_projection(result)?;

    if result.candidates.is_empty() {
        return confident_projection(result);
    }

    for candidate in &result.candidates {
        if broad_genre(candidate.genre) != Some(final_broad) {
            return None;
        }
    }

    if result.candidates.len() >= 2 {
        Some(final_broad)
    } else {
        confident_projection(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::classification::{
        ClassificationAction, ClassificationDegradedReason, GenreCandidate,
    };

    fn result(
        genre: Option<&'static str>,
        confidence: ClassificationConfidence,
        mode: ClassificationMode,
        candidates: Vec<&'static str>,
    ) -> ClassificationResult {
        ClassificationResult {
            track_id: "track-1".into(),
            artist: "Artist".into(),
            title: "Title".into(),
            current_genre: String::new(),
            genre,
            confidence,
            action: ClassificationAction::Suggest,
            mode,
            degraded_reasons: if mode == ClassificationMode::Degraded {
                vec![ClassificationDegradedReason::MissingEssentia]
            } else {
                Vec::new()
            },
            evidence: Vec::new(),
            candidates: candidates
                .into_iter()
                .map(|candidate| GenreCandidate {
                    genre: candidate,
                    score: 1.0,
                    bpm_plausible: true,
                    chosen: candidate == genre.unwrap_or_default(),
                })
                .collect(),
            flags: Vec::new(),
            review_hint: None,
        }
    }

    #[test]
    fn broad_mapping_covers_every_canonical_genre_deliberately() {
        for genre in super::super::taxonomy::GENRES {
            if *genre == "Experimental" {
                assert_eq!(broad_genre(genre), None);
            } else {
                assert!(
                    broad_genre(genre).is_some(),
                    "missing broad mapping for {genre}"
                );
            }
        }
    }

    #[test]
    fn broad_mapping_preserves_boundary_targets() {
        assert_eq!(broad_genre("Deep House"), Some("House"));
        assert_eq!(broad_genre("Dub Techno"), Some("Techno"));
        assert_eq!(broad_genre("Hard Trance"), Some("Trance"));
        assert_eq!(broad_genre("Jungle"), Some("Drum & Bass"));
        assert_eq!(broad_genre("Tech House"), Some("Tech House"));
        assert_eq!(broad_genre("Minimal"), Some("Minimal"));
        assert_eq!(broad_genre("Electro"), Some("Electro"));
    }

    #[test]
    fn parent_consensus_accepts_low_confidence_same_parent_disagreement() {
        let result = result(
            Some("Deep House"),
            ClassificationConfidence::Low,
            ClassificationMode::Full,
            vec!["Deep House", "House", "Progressive House"],
        );
        assert_eq!(parent_consensus(&result), Some("House"));
    }

    #[test]
    fn parent_consensus_rejects_cross_parent_disagreement() {
        let result = result(
            Some("Deep House"),
            ClassificationConfidence::High,
            ClassificationMode::Full,
            vec!["Deep House", "Techno"],
        );
        assert_eq!(parent_consensus(&result), None);
    }

    #[test]
    fn parent_consensus_rejects_unmapped_candidate() {
        let result = result(
            Some("House"),
            ClassificationConfidence::High,
            ClassificationMode::Full,
            vec!["House", "Experimental"],
        );
        assert_eq!(parent_consensus(&result), None);
    }

    #[test]
    fn parent_consensus_rejects_degraded_results() {
        let result = result(
            Some("House"),
            ClassificationConfidence::High,
            ClassificationMode::Degraded,
            vec!["House", "Deep House"],
        );
        assert_eq!(parent_consensus(&result), None);
    }

    #[test]
    fn parent_consensus_requires_confidence_without_disagreement_evidence() {
        let low_audio_veto = result(
            Some("Ambient"),
            ClassificationConfidence::Low,
            ClassificationMode::Full,
            vec![],
        );
        let medium_audio_veto = result(
            Some("Ambient"),
            ClassificationConfidence::Medium,
            ClassificationMode::Full,
            vec![],
        );
        assert_eq!(parent_consensus(&low_audio_veto), None);
        assert_eq!(parent_consensus(&medium_audio_veto), Some("Ambient"));
    }

    #[test]
    fn parent_consensus_requires_confidence_for_one_candidate() {
        let low = result(
            Some("House"),
            ClassificationConfidence::Low,
            ClassificationMode::Full,
            vec!["House"],
        );
        let high = result(
            Some("House"),
            ClassificationConfidence::High,
            ClassificationMode::Full,
            vec!["House"],
        );
        assert_eq!(parent_consensus(&low), None);
        assert_eq!(parent_consensus(&high), Some("House"));
    }
}
