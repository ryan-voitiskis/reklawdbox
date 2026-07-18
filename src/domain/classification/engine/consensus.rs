use std::collections::{HashMap, HashSet};

use super::super::taxonomy::GenreFamily;
use super::super::{
    ClassificationConfidence, DiscogsMatchQuality, LabelProvenance, TrackEvidence,
    taxonomy as genre,
};
use super::audio::{
    AudioProfile, BpmContext, CharFlag, EnergyBucket, clearly_favors_family, has_flag,
};
use super::votes::{EvidenceSourceGroup, GenreVote, bpm_plausible};
use super::{ClassificationDecision, current_genre_tokens};

struct RankedGenre {
    genre: &'static str,
    score: f32,
    bpm_plausible: bool,
}

pub(super) fn resolve(
    evidence: &TrackEvidence,
    votes: &[GenreVote],
    audio_profile: Option<&AudioProfile>,
    bpm_context: BpmContext,
) -> ClassificationDecision {
    let mut tally: HashMap<&'static str, (f32, bool)> = HashMap::new();
    for vote in votes {
        let entry = tally.entry(vote.genre).or_insert((0.0, true));
        entry.0 += vote.weight;
        if !vote.bpm_plausible {
            entry.1 = false;
        }
    }

    let mut ranked: Vec<RankedGenre> = tally
        .into_iter()
        .map(|(genre, (score, bpm_plausible))| RankedGenre {
            genre,
            score,
            bpm_plausible,
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.bpm_plausible.cmp(&a.bpm_plausible))
            .then_with(|| a.genre.cmp(b.genre))
    });

    if ranked.is_empty() {
        return ClassificationDecision::empty();
    }

    let mut top_genre = ranked[0].genre;
    let top_score = ranked[0].score;
    let second = ranked.get(1);
    let margin = top_score - second.map_or(0.0, |candidate| candidate.score);
    let total_weight: f32 = ranked.iter().map(|candidate| candidate.score).sum();

    let mut evidence_lines = Vec::new();
    let mut flags = Vec::new();

    if let Some(fallback) = bpm_context.fallback {
        evidence_lines.push(format!(
            "bpm-fallback: rekordbox {:.1} → detector consensus {:.1}",
            fallback.rekordbox_bpm, fallback.detector_bpm
        ));
        flags.push("bpm-rekordbox-disagrees".into());
    }

    if !evidence.discogs_mapped.is_empty() {
        let match_quality = match evidence.discogs_match_quality {
            Some(DiscogsMatchQuality::Exact) => "exact",
            Some(DiscogsMatchQuality::Fuzzy) => "fuzzy",
            Some(DiscogsMatchQuality::Invalid) => "invalid",
            None => "unknown",
        };
        let parts: Vec<String> = evidence
            .discogs_mapped
            .iter()
            .map(|mapped| {
                let bpm_note = if !bpm_plausible(mapped.genre, bpm_context.effective_bpm) {
                    " [bpm-implausible]"
                } else {
                    ""
                };
                format!("{}(x{}){}", mapped.genre, mapped.style_count, bpm_note)
            })
            .collect();
        evidence_lines.push(format!(
            "discogs: {} [match={match_quality}]",
            parts.join(", ")
        ));
    }

    if let Some(label_genre) = evidence.label_genre {
        let label_name = evidence.label.as_deref().unwrap_or("?");
        let provenance = match evidence.label_provenance {
            Some(LabelProvenance::Rekordbox) => "rekordbox",
            Some(LabelProvenance::Discogs) => "discogs",
            Some(LabelProvenance::CorrelatedDiscogs) => "correlated-discogs",
            None => "unknown",
        };
        evidence_lines.push(format!(
            "label: {label_name} → {label_genre} [source={provenance}]"
        ));
    }

    if let Some(profile) = audio_profile {
        let bucket_name = match profile.bucket {
            Some(EnergyBucket::NonDancefloor) => "non-dancefloor",
            Some(EnergyBucket::LowEnergy) => "low-energy",
            Some(EnergyBucket::Dancefloor) => "dancefloor",
            Some(EnergyBucket::HighEnergy) => "high-energy",
            None => "energy-unknown",
        };
        let flag_names: Vec<&str> = profile
            .flags
            .iter()
            .map(|flag| match flag {
                CharFlag::Ambient => "ambient",
                CharFlag::Atmospheric => "atmospheric",
                CharFlag::Broken => "broken",
                CharFlag::Irregular => "irregular",
                CharFlag::Fast => "fast",
                CharFlag::Slow => "slow",
                CharFlag::Atonal => "atonal",
                CharFlag::LongTail => "long-tail",
                CharFlag::Compressed => "compressed",
            })
            .collect();
        let audio_line = if flag_names.is_empty() {
            format!("audio: {} {}bpm", bucket_name, profile.bpm as i32)
        } else {
            format!(
                "audio: {} {} {}bpm",
                bucket_name,
                flag_names.join("+"),
                profile.bpm as i32
            )
        };
        evidence_lines.push(audio_line);
    }

    let mut confidence = if ranked.len() == 1 && top_score >= 1.0 {
        if votes
            .iter()
            .filter(|vote| vote.genre == top_genre)
            .all(|vote| vote.bpm_plausible)
        {
            ClassificationConfidence::High
        } else {
            flags.push("bpm-implausible".into());
            ClassificationConfidence::Medium
        }
    } else if ranked.len() >= 2 {
        let second_genre = second.expect("second exists when ranked.len() >= 2").genre;
        let same_family = genre::genre_family(top_genre) == genre::genre_family(second_genre);

        if margin / total_weight > 0.4 {
            if votes
                .iter()
                .filter(|vote| vote.genre == top_genre)
                .all(|vote| vote.bpm_plausible)
            {
                ClassificationConfidence::High
            } else {
                flags.push("bpm-implausible".into());
                ClassificationConfidence::Medium
            }
        } else if margin / total_weight > 0.15 {
            if same_family {
                top_genre = resolve_same_family_specificity(
                    top_genre,
                    second_genre,
                    audio_profile,
                    &mut evidence_lines,
                );
            }
            ClassificationConfidence::Medium
        } else if same_family {
            top_genre = resolve_same_family_specificity(
                top_genre,
                second_genre,
                audio_profile,
                &mut evidence_lines,
            );
            ClassificationConfidence::Low
        } else if let Some(profile) = audio_profile {
            let top_favored = clearly_favors_family(profile, top_genre);
            let second_favored = clearly_favors_family(profile, second_genre);
            if top_favored && !second_favored {
                flags.push("audio-assisted-tiebreak".into());
                ClassificationConfidence::Low
            } else if second_favored && !top_favored {
                top_genre = second_genre;
                flags.push("audio-assisted-tiebreak".into());
                ClassificationConfidence::Low
            } else {
                ClassificationConfidence::Insufficient
            }
        } else {
            ClassificationConfidence::Insufficient
        }
    } else if !votes
        .iter()
        .filter(|vote| vote.genre == top_genre)
        .all(|vote| vote.bpm_plausible)
    {
        flags.push("bpm-implausible".into());
        ClassificationConfidence::Low
    } else {
        ClassificationConfidence::Medium
    };

    let mut final_genre = top_genre;

    // Swap to a BPM-plausible alternative if the winner is implausible.
    let effective_bpm = bpm_context.effective_bpm;
    if !bpm_plausible(final_genre, effective_bpm)
        && let Some(alternative) = ranked
            .iter()
            .skip(1)
            .find(|candidate| bpm_plausible(candidate.genre, effective_bpm))
    {
        evidence_lines.push(format!(
            "bpm-override: {} implausible at {}bpm → {}",
            final_genre, effective_bpm as i32, alternative.genre
        ));
        flags.push("bpm-override".into());
        let same_family = genre::genre_family(final_genre)
            == genre::genre_family(alternative.genre)
            && genre::genre_family(final_genre) != GenreFamily::Other;
        final_genre = alternative.genre;
        // Downgrade: runner-up was elevated by BPM elimination, not evidence weight.
        // Same-family swaps floor at Medium - the family evidence is intact.
        // Note: High + BPM override is rare for same-family because BPM-implausible
        // votes prevent High assignment in the consensus scoring above.
        confidence = match confidence {
            ClassificationConfidence::High => ClassificationConfidence::Medium,
            ClassificationConfidence::Medium if !same_family => ClassificationConfidence::Low,
            other => other,
        };
        if same_family {
            flags.push("bpm-override-same-family".into());
        }
    }

    // HighEnergy always demotes deep variants (e.g. Deep Techno -> Techno).
    // Dancefloor demotes only when the shallower variant also has votes,
    // and not when the track is Atmospheric, Atonal, LongTail, or Compressed
    // - each signals a deeper read.
    if let Some(profile) = audio_profile
        && let Some(shallower) = shallower_alternative(final_genre)
    {
        let demote = match profile.bucket {
            Some(EnergyBucket::HighEnergy) => true,
            Some(EnergyBucket::Dancefloor) => {
                ranked.iter().any(|candidate| candidate.genre == shallower)
                    && !has_flag(profile, CharFlag::Atmospheric)
                    && !has_flag(profile, CharFlag::Atonal)
                    && !has_flag(profile, CharFlag::LongTail)
                    && !has_flag(profile, CharFlag::Compressed)
            }
            _ => false,
        };
        if demote {
            evidence_lines.push(format!(
                "depth: {}-energy audio → {} over {}",
                if profile.bucket == Some(EnergyBucket::HighEnergy) {
                    "high"
                } else {
                    "dancefloor"
                },
                shallower,
                final_genre
            ));
            final_genre = shallower;
        }
    }

    let primary_family = genre::genre_family(final_genre);
    for mapped in &evidence.discogs_mapped {
        if genre::genre_family(mapped.genre) != primary_family && mapped.style_count >= 2 {
            evidence_lines.push(format!(
                "influence: {} (discogs x{})",
                mapped.genre, mapped.style_count
            ));
        }
    }

    let tie_ratio = if total_weight > 0.0 {
        margin / total_weight
    } else {
        1.0
    };
    if confidence == ClassificationConfidence::Insufficient
        && ranked.len() >= 2
        && tie_ratio <= 0.15
    {
        let current_tokens = current_genre_tokens(&evidence.current_genre);
        let matching: Vec<_> = current_tokens
            .into_iter()
            .filter(|token| ranked.iter().any(|candidate| candidate.genre == *token))
            .collect();
        if matching.len() == 1 && bpm_plausible(matching[0], effective_bpm) {
            final_genre = matching[0];
            evidence_lines.push(format!(
                "current-genre tie-break: \"{}\" → {}",
                evidence.current_genre, final_genre
            ));
            flags.push("current-genre-tiebreak".into());
        }
    }

    if confidence == ClassificationConfidence::High {
        let independent_groups: HashSet<EvidenceSourceGroup> = votes
            .iter()
            .filter(|vote| vote.genre == final_genre)
            .map(|vote| vote.group)
            .collect();
        if independent_groups.len() < 2 {
            confidence = ClassificationConfidence::Medium;
            flags.push("single-source-confidence-cap".into());
        }
    }

    ClassificationDecision {
        genre: Some(final_genre),
        confidence,
        evidence: evidence_lines,
        flags,
    }
}

/// Picks between two same-family genres using genre depth and audio energy.
fn resolve_same_family_specificity(
    top: &'static str,
    second: &'static str,
    audio_profile: Option<&AudioProfile>,
    evidence: &mut Vec<String>,
) -> &'static str {
    let top_depth = genre::genre_depth(top);
    let second_depth = genre::genre_depth(second);

    if top_depth == second_depth {
        evidence.push(format!("same-family same-depth: {top} vs {second}"));
        return top;
    }

    let (deeper, shallower) = if top_depth > second_depth {
        (top, second)
    } else {
        (second, top)
    };

    if let Some(profile) = audio_profile {
        let family = genre::genre_family(deeper);
        let atonal = has_flag(profile, CharFlag::Atonal);
        let long_tail = has_flag(profile, CharFlag::LongTail);
        let compressed = has_flag(profile, CharFlag::Compressed);

        if atonal && family == GenreFamily::House {
            evidence.push(format!(
                "depth: audio atonal → {shallower} over {deeper} (chord-driven Deep House unlikely)"
            ));
            return shallower;
        }

        if atonal && family == GenreFamily::Techno {
            evidence.push(format!(
                "depth: audio atonal → {deeper} over {shallower} (noise/drone-driven)"
            ));
            return deeper;
        }

        if compressed
            && family == GenreFamily::Techno
            && profile.bucket == Some(EnergyBucket::Dancefloor)
        {
            evidence.push(format!(
                "depth: audio compressed dancefloor → {deeper} over {shallower} (club-master signal)"
            ));
            return deeper;
        }

        if has_flag(profile, CharFlag::Atmospheric)
            || profile.bucket == Some(EnergyBucket::LowEnergy)
            || long_tail
        {
            evidence.push(format!(
                "depth: audio atmospheric/low-energy/long-tail → {deeper} over {shallower}"
            ));
            deeper
        } else if profile.bucket == Some(EnergyBucket::HighEnergy) {
            evidence.push(format!(
                "depth: audio high-energy → {shallower} over {deeper}"
            ));
            shallower
        } else {
            evidence.push(format!(
                "depth: {} (depth {}) vs {} (depth {}), no strong audio signal",
                deeper,
                genre::genre_depth(deeper),
                shallower,
                genre::genre_depth(shallower),
            ));
            top
        }
    } else {
        evidence.push(format!("depth: {top} vs {second}, no audio to resolve"));
        top
    }
}

fn shallower_alternative(genre_name: &str) -> Option<&'static str> {
    match genre_name {
        "Deep Techno" | "Dub Techno" | "Ambient Techno" => Some("Techno"),
        "Deep House" => Some("House"),
        _ => None,
    }
}
