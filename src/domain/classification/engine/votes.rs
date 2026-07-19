use std::collections::HashMap;

use super::super::profiles::{self as audio_profile, ProfileRegistry};
use super::super::{GenreCandidate, LabelProvenance, TrackEvidence, taxonomy as genre};
use super::audio::{AudioProfile, BpmContext, CharFlag, has_flag};

/// BPM tolerance: +/-5 BPM around the defined genre range.
const BPM_TOLERANCE: f64 = 5.0;
/// Low-weight conjunctive boost, aligned with the audio-profile vote cap.
const AUDIO_RULE_BOOST: f32 = 0.5;

/// A single vote for a genre from one evidence source.
pub(super) struct GenreVote {
    pub(super) genre: &'static str,
    pub(super) weight: f32,
    pub(super) source: &'static str,
    pub(super) group: EvidenceSourceGroup,
    pub(super) bpm_plausible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum EvidenceSourceGroup {
    Discogs,
    RekordboxLabel,
    Audio,
}

pub(super) struct VoteCollection {
    pub(super) votes: Vec<GenreVote>,
    pub(super) affinities: Vec<audio_profile::AudioAffinity>,
    pub(super) calibrated_coverage_missing: bool,
}

pub(super) fn gather(
    evidence: &TrackEvidence,
    audio_profile: Option<&AudioProfile>,
    profile_registry: Option<&ProfileRegistry>,
    bpm_context: BpmContext,
) -> VoteCollection {
    let mut votes = Vec::new();
    let effective_bpm = bpm_context.effective_bpm;

    // Discogs: proportional to style_count with steeper decay for diverse releases.
    // Album-level data is less informative per-genre when spread across many genres.
    let total_styles: usize = evidence.discogs_mapped.iter().map(|m| m.style_count).sum();
    let n_mapped_genres = evidence.discogs_mapped.len();
    let diversity_decay = if n_mapped_genres <= 1 {
        1.0_f32
    } else {
        1.0 / (n_mapped_genres as f32).powf(0.4)
    };
    for mapped in &evidence.discogs_mapped {
        let proportion = if total_styles > 0 {
            (mapped.style_count as f32 / total_styles as f32).min(1.0)
        } else {
            0.5
        };
        let base_weight = proportion * 0.9 * diversity_decay;
        let plausible = bpm_plausible(mapped.genre, effective_bpm);
        votes.push(GenreVote {
            genre: mapped.genre,
            weight: if plausible {
                base_weight
            } else {
                base_weight * 0.5
            },
            source: "discogs",
            group: EvidenceSourceGroup::Discogs,
            bpm_plausible: plausible,
        });
    }

    // Label: weight 0.6, reduced to 0.4 when confirming to avoid double-counting.
    if let Some(label_genre) = evidence.label_genre {
        let plausible = bpm_plausible(label_genre, effective_bpm);
        let confirms = votes.iter().any(|vote| vote.genre == label_genre);
        let weight = if confirms { 0.4 } else { 0.6 };
        votes.push(GenreVote {
            genre: label_genre,
            weight: if plausible { weight } else { weight * 0.5 },
            source: "label",
            group: match evidence.label_provenance {
                Some(LabelProvenance::Discogs | LabelProvenance::CorrelatedDiscogs) => {
                    EvidenceSourceGroup::Discogs
                }
                Some(LabelProvenance::Rekordbox) | None => EvidenceSourceGroup::RekordboxLabel,
            },
            bpm_plausible: plausible,
        });
    }

    // Audio-profile votes: Fisher discriminant scoring against calibrated prototypes.
    let (affinities, calibrated_coverage_missing) =
        if let (Some(audio), Some(registry)) = (&evidence.audio, profile_registry) {
            let scored = audio_profile::score_all(audio, registry);
            let affinities = scored.affinities;
            for affinity in &affinities {
                if affinity.vote_weight < 0.05 {
                    continue;
                }
                votes.push(GenreVote {
                    genre: affinity.genre,
                    weight: affinity.vote_weight,
                    source: "audio-profile",
                    group: EvidenceSourceGroup::Audio,
                    bpm_plausible: bpm_plausible(affinity.genre, effective_bpm),
                });
            }
            (
                affinities,
                !registry.prototypes.is_empty() && !scored.had_sufficient_coverage,
            )
        } else {
            (vec![], false)
        };

    if let Some(profile) = audio_profile
        && has_flag(profile, CharFlag::LongTail)
        && has_flag(profile, CharFlag::Atonal)
        && votes.iter().any(|vote| vote.genre == "Drone Techno")
    {
        votes.push(GenreVote {
            genre: "Drone Techno",
            weight: AUDIO_RULE_BOOST,
            source: "audio-long-tail-atonal",
            group: EvidenceSourceGroup::Audio,
            bpm_plausible: bpm_plausible("Drone Techno", effective_bpm),
        });
    }

    VoteCollection {
        votes,
        affinities,
        calibrated_coverage_missing,
    }
}

pub(super) fn bpm_plausible(genre_name: &str, bpm: f64) -> bool {
    if bpm <= 0.0 {
        return true;
    }
    match genre::genre_bpm_range(genre_name) {
        Some(range) => {
            bpm >= (range.typical_min - BPM_TOLERANCE) && bpm <= (range.typical_max + BPM_TOLERANCE)
        }
        None => true,
    }
}

pub(super) fn candidates(votes: &[GenreVote], top_genre: Option<&str>) -> Vec<GenreCandidate> {
    let mut tally: HashMap<&'static str, (f32, bool)> = HashMap::new();
    for vote in votes {
        let entry = tally.entry(vote.genre).or_insert((0.0, true));
        entry.0 += vote.weight;
        if !vote.bpm_plausible {
            entry.1 = false;
        }
    }

    let mut candidates: Vec<GenreCandidate> = tally
        .into_iter()
        .map(|(genre, (score, bpm_plausible))| GenreCandidate {
            genre,
            score,
            bpm_plausible,
            chosen: Some(genre) == top_genre,
        })
        .collect();
    // Chosen first, then by score desc, then deterministic tiebreak.
    candidates.sort_by(|a, b| {
        b.chosen
            .cmp(&a.chosen)
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| b.bpm_plausible.cmp(&a.bpm_plausible))
            .then_with(|| a.genre.cmp(b.genre))
    });
    candidates.truncate(4);
    candidates
}
