use super::super::{ClassificationConfidence, DiscogsReadiness, TrackEvidence, taxonomy as genre};
use super::ClassificationDecision;
use super::audio::{AudioProfile, EnergyBucket};
use super::votes::bpm_plausible;

pub(super) fn resolve(
    evidence: &TrackEvidence,
    audio_profile: Option<&AudioProfile>,
) -> ClassificationDecision {
    let Some(profile) = audio_profile else {
        return ClassificationDecision {
            genre: None,
            confidence: ClassificationConfidence::Insufficient,
            evidence: vec!["no enrichment data, no audio analysis".into()],
            flags: vec!["no-data".into()],
        };
    };

    let dynamic_complexity = evidence
        .audio
        .as_ref()
        .and_then(|audio| audio.dynamic_complexity)
        .filter(|value| value.is_finite());
    let rhythm_regularity = evidence
        .audio
        .as_ref()
        .and_then(|audio| audio.rhythm_regularity)
        .filter(|value| value.is_finite());
    let spectral_centroid = evidence
        .audio
        .as_ref()
        .and_then(|audio| audio.spectral_centroid_mean)
        .filter(|value| value.is_finite());

    let mut candidates: Vec<&'static str> = Vec::new();
    let mut evidence_lines = Vec::new();

    // D.1: Broad bucket.
    match profile.bucket {
        None => {
            evidence_lines
                .push("D.1: energy evidence missing; no audio-only recommendation".into());
        }
        Some(EnergyBucket::NonDancefloor) => {
            if dynamic_complexity.is_some_and(|value| value > 10.0) {
                candidates.push("Ambient");
                evidence_lines
                    .push("D.1: non-dancefloor + high dynamic complexity → Ambient".into());
            } else if dynamic_complexity.is_some_and(|value| value > 5.0) {
                candidates.extend_from_slice(&["Experimental", "Ambient"]);
                evidence_lines.push(
                    "D.1: non-dancefloor + moderate complexity → Experimental/Ambient".into(),
                );
            } else if dynamic_complexity.is_some() {
                candidates.extend_from_slice(&["Downtempo", "Experimental"]);
                evidence_lines
                    .push("D.1: non-dancefloor + low complexity → Downtempo/Experimental".into());
            } else {
                evidence_lines.push(
                    "D.1: dynamic-complexity evidence missing; no non-dancefloor refinement".into(),
                );
            }
        }
        Some(EnergyBucket::LowEnergy) => {
            if profile.bpm > 145.0 {
                candidates.extend_from_slice(&["Jungle", "Breakbeat"]);
                evidence_lines.push(format!(
                    "D.1: low-energy but fast ({}bpm) → Jungle/Breakbeat",
                    profile.bpm as i32
                ));
            } else if dynamic_complexity.is_some_and(|value| value > 5.0) {
                candidates.extend_from_slice(&["Downtempo", "Ambient Techno"]);
                evidence_lines
                    .push("D.1: low-energy + atmospheric → Downtempo/Ambient Techno".into());
            } else if dynamic_complexity.is_some() {
                candidates.extend_from_slice(&["Electro", "IDM"]);
                evidence_lines.push("D.1: low-energy + low complexity → Electro/IDM".into());
            } else {
                evidence_lines.push(
                    "D.1: dynamic-complexity evidence missing; no low-energy refinement".into(),
                );
            }
        }
        Some(EnergyBucket::Dancefloor | EnergyBucket::HighEnergy) => {
            // D.2: Subgenre by BPM x rhythm regularity.
            let bpm = profile.bpm;
            let Some(rhythm_regularity) = rhythm_regularity else {
                evidence_lines.push(
                    "D.2: rhythm-regularity evidence missing; no dancefloor refinement".into(),
                );
                return finish(evidence, profile, candidates, evidence_lines);
            };
            if bpm > 155.0 {
                candidates.extend_from_slice(&["Drum & Bass", "Jungle"]);
                evidence_lines.push(format!("D.2: fast ({}bpm) → D&B/Jungle", bpm as i32));
            } else if bpm >= 135.0 && rhythm_regularity > 0.9 {
                candidates.extend_from_slice(&["Trance", "Hard Techno"]);
                evidence_lines.push(format!(
                    "D.2: {}bpm + regular rhythm → Trance/Hard Techno",
                    bpm as i32
                ));
            } else if bpm >= 128.0 && rhythm_regularity > 0.9 {
                candidates.push("Techno");
                evidence_lines.push(format!("D.2: {}bpm + regular rhythm → Techno", bpm as i32));
            } else if (120.0..=135.0).contains(&bpm) && rhythm_regularity > 0.9 {
                candidates.extend_from_slice(&["Techno", "Tech House", "House"]);
                evidence_lines.push(format!(
                    "D.2: {}bpm + regular → Techno/Tech House/House",
                    bpm as i32
                ));
            } else if (118.0..=130.0).contains(&bpm) && rhythm_regularity >= 0.8 {
                candidates.extend_from_slice(&["House", "Deep House"]);
                evidence_lines.push(format!(
                    "D.2: {}bpm + moderate rhythm → House/Deep House",
                    bpm as i32
                ));
            } else if (120.0..=140.0).contains(&bpm) && rhythm_regularity < 0.8 {
                candidates.extend_from_slice(&["Breakbeat", "Garage"]);
                evidence_lines.push(format!(
                    "D.2: {}bpm + broken rhythm → Breakbeat/Garage",
                    bpm as i32
                ));
            } else if bpm < 120.0 {
                candidates.extend_from_slice(&["Deep House", "Downtempo"]);
                evidence_lines.push(format!(
                    "D.2: slow ({}bpm) → Deep House/Downtempo",
                    bpm as i32
                ));
            } else {
                candidates.push("House");
                evidence_lines.push(format!(
                    "D.2: {}bpm, unmatched → House (fallback)",
                    bpm as i32
                ));
            }
        }
    }

    // D.3: Spectral centroid refinement.
    // Preference lists are provisional - refine once issue #19 has empirical data.
    if let Some(centroid) = spectral_centroid
        && candidates.len() > 1
    {
        let first_before = candidates[0];
        let (centroid_hint, reordered) = if centroid < 1200.0 {
            let preferred: &[&str] = &[
                "Downtempo",
                "Ambient",
                "Ambient Techno",
                "Deep House",
                "Minimal",
                "Techno",
                "Jungle",
                "Trance",
                "Garage",
                "IDM",
                "Experimental",
            ];
            candidates.sort_by_key(|genre_name| {
                preferred
                    .iter()
                    .position(|preferred| preferred == genre_name)
                    .unwrap_or(usize::MAX)
            });
            (
                if centroid < 600.0 {
                    "very-low centroid"
                } else {
                    "low centroid"
                },
                candidates[0] != first_before,
            )
        } else if centroid >= 2500.0 {
            let preferred: &[&str] = &[
                "Hard Techno",
                "Breakbeat",
                "Tech House",
                "Drum & Bass",
                "House",
                "Electro",
            ];
            candidates.sort_by_key(|genre_name| {
                preferred
                    .iter()
                    .position(|preferred| preferred == genre_name)
                    .unwrap_or(usize::MAX)
            });
            (
                if centroid >= 4000.0 {
                    "high centroid"
                } else {
                    "mid-high centroid"
                },
                candidates[0] != first_before,
            )
        } else {
            ("mid centroid", false)
        };

        if reordered {
            evidence_lines.push(format!(
                "D.3: {} → {} over {}",
                centroid_hint, candidates[0], first_before
            ));
        } else {
            evidence_lines.push(format!("D.3: {centroid_hint} (confirms D.2 order)"));
        }
    }

    finish(evidence, profile, candidates, evidence_lines)
}

fn finish(
    evidence: &TrackEvidence,
    profile: &AudioProfile,
    candidates: Vec<&'static str>,
    evidence_lines: Vec<String>,
) -> ClassificationDecision {
    // D.4: Confidence.
    let (genre, confidence) = if candidates.len() == 1 {
        (
            genre::canonical_genre_name(candidates[0]).or(Some(candidates[0])),
            ClassificationConfidence::Low,
        )
    } else if candidates.is_empty() {
        (None, ClassificationConfidence::Insufficient)
    } else {
        let best = candidates
            .iter()
            .find(|&&candidate| bpm_plausible(candidate, profile.bpm))
            .or(candidates.first())
            .copied();
        (best, ClassificationConfidence::Insufficient)
    };

    let mut flags = vec!["audio-only".into()];
    if evidence.discogs_readiness() != DiscogsReadiness::UsableGenre {
        flags.push("no-enrichment".into());
    }

    ClassificationDecision {
        genre,
        confidence,
        evidence: evidence_lines,
        flags,
    }
}
