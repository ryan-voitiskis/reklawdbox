//! Server-side genre decision tree.
//!
//! Applies evidence-based genre classification using cached enrichment data
//! (Discogs, Beatport, label mapping) and audio analysis features (Essentia).
//!
//! The `ClassificationConfidence` enum here is distinct from `crate::types::Confidence`
//! which models normalization state (alias/unknown/canonical).

use std::collections::HashMap;

use serde::Serialize;

use crate::genre::{self, GenreFamily};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GenreCandidate {
    pub(crate) genre: &'static str,
    pub(crate) score: f32,
    pub(crate) bpm_plausible: bool,
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

// ---------------------------------------------------------------------------
// Input types — pure data, no DB handles
// ---------------------------------------------------------------------------

/// Mapped genre from an enrichment source.
pub(crate) struct MappedGenre {
    pub(crate) genre: &'static str,
    pub(crate) style_count: usize,
}

/// Pre-extracted audio features from cache.
pub(crate) struct AudioFeatures {
    pub(crate) rekordbox_bpm: f64,
    #[allow(dead_code)]
    pub(crate) stratum_bpm: Option<f64>,
    #[allow(dead_code)]
    pub(crate) bpm_agreement: Option<bool>,
    pub(crate) danceability: Option<f64>,
    pub(crate) dynamic_complexity: Option<f64>,
    pub(crate) rhythm_regularity: Option<f64>,
    pub(crate) spectral_centroid_mean: Option<f64>,
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

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnergyBucket {
    NonDancefloor,
    LowEnergy,
    Dancefloor,
    HighEnergy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CharFlag {
    Ambient,
    Atmospheric,
    Broken,
    Irregular,
    Fast,
    Slow,
}

struct AudioProfile {
    bucket: EnergyBucket,
    flags: Vec<CharFlag>,
    bpm: f64, // effective BPM for thresholds (rekordbox preferred)
}

/// A single vote for a genre from one evidence source.
struct GenreVote {
    genre: &'static str,
    weight: f32,
    #[allow(dead_code)]
    source: &'static str,
    bpm_plausible: bool,
}

/// BPM tolerance: ±5 BPM around the defined genre range.
const BPM_TOLERANCE: f64 = 5.0;

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

pub(crate) fn classify_track(evidence: &TrackEvidence) -> ClassificationResult {
    let audio_profile = evidence.audio.as_ref().map(compute_audio_profile);

    // Step B.3: Check audio vetoes first
    if let Some(ref profile) = audio_profile {
        if let Some(result) = check_audio_vetoes(evidence, profile) {
            return result;
        }
    }

    // Gather votes from all sources (Step A)
    let votes = gather_votes(evidence, audio_profile.as_ref());

    // Steps C/D: Find consensus or do audio-only inference
    let (genre, confidence, ev_lines, mut flags) = if votes.is_empty() {
        audio_only_inference(evidence, audio_profile.as_ref())
    } else {
        find_consensus(evidence, &votes, audio_profile.as_ref())
    };

    // Step E: Compare to current genre
    let current_canonical = resolve_current_canonical(&evidence.current_genre);
    let action = compare_to_current(current_canonical, genre);

    // Build candidates list from votes
    let candidates = build_candidates(&votes, genre);

    // Add data source flags (before review hint so it can see them)
    if !evidence.has_discogs && !evidence.has_beatport {
        flags.push("no-enrichment".into());
    }
    if !evidence.has_audio {
        flags.push("no-audio".into());
    }

    // Build review hint for low/insufficient
    let review_hint = match confidence {
        ClassificationConfidence::Low | ClassificationConfidence::Insufficient => {
            Some(build_review_hint(evidence, &flags))
        }
        _ => None,
    };

    ClassificationResult {
        track_id: evidence.track_id.clone(),
        artist: evidence.artist.clone(),
        title: evidence.title.clone(),
        current_genre: evidence.current_genre.clone(),
        genre,
        confidence,
        action,
        evidence: ev_lines,
        candidates,
        flags,
        review_hint,
    }
}

// ---------------------------------------------------------------------------
// Step B: Audio profile computation
// ---------------------------------------------------------------------------

fn compute_audio_profile(audio: &AudioFeatures) -> AudioProfile {
    let bpm = audio.rekordbox_bpm;
    let danceability = audio.danceability.unwrap_or(1.5);

    // B.1: Energy bucket
    let bucket = if danceability < 1.0 {
        EnergyBucket::NonDancefloor
    } else if danceability < 1.5 {
        EnergyBucket::LowEnergy
    } else if danceability <= 2.5 {
        EnergyBucket::Dancefloor
    } else {
        EnergyBucket::HighEnergy
    };

    // B.2: Character flags
    let mut flags = Vec::new();
    let dc = audio.dynamic_complexity.unwrap_or(0.0);
    let rr = audio.rhythm_regularity.unwrap_or(0.85);

    if dc > 10.0 {
        flags.push(CharFlag::Ambient);
    }
    if dc > 5.0 {
        flags.push(CharFlag::Atmospheric);
    }
    // Caveat: atmospheric + broken/irregular → lower confidence on rhythm flags
    // We still set them but the decision tree checks for this combination
    if rr < 0.5 {
        flags.push(CharFlag::Broken);
    } else if rr < 0.8 {
        flags.push(CharFlag::Irregular);
    }
    if bpm > 155.0 {
        flags.push(CharFlag::Fast);
    }
    if bpm < 115.0 {
        flags.push(CharFlag::Slow);
    }

    AudioProfile { bucket, flags, bpm }
}

fn has_flag(profile: &AudioProfile, flag: CharFlag) -> bool {
    profile.flags.contains(&flag)
}

// ---------------------------------------------------------------------------
// Step B.3: Audio vetoes
// ---------------------------------------------------------------------------

fn veto_result(
    evidence: &TrackEvidence,
    genre: &'static str,
    confidence: ClassificationConfidence,
    action: ClassificationAction,
    ev_lines: Vec<String>,
    review_hint: Option<String>,
) -> ClassificationResult {
    ClassificationResult {
        track_id: evidence.track_id.clone(),
        artist: evidence.artist.clone(),
        title: evidence.title.clone(),
        current_genre: evidence.current_genre.clone(),
        genre: Some(genre),
        confidence,
        action,
        evidence: ev_lines,
        candidates: vec![],
        flags: vec!["audio-vetoed".into()],
        review_hint,
    }
}

fn check_audio_vetoes(
    evidence: &TrackEvidence,
    profile: &AudioProfile,
) -> Option<ClassificationResult> {
    let current_canonical = resolve_current_canonical(&evidence.current_genre);

    // non_dancefloor + ambient → Ambient
    if profile.bucket == EnergyBucket::NonDancefloor && has_flag(profile, CharFlag::Ambient) {
        let action = compare_to_current(current_canonical, Some("Ambient"));
        return Some(veto_result(
            evidence,
            "Ambient",
            ClassificationConfidence::Medium,
            action,
            vec!["audio veto: non-dancefloor + ambient → Ambient".into()],
            None,
        ));
    }

    // non_dancefloor + slow → Downtempo family
    if profile.bucket == EnergyBucket::NonDancefloor && has_flag(profile, CharFlag::Slow) {
        let dt_genre = find_family_genre(evidence, GenreFamily::Downtempo).unwrap_or("Downtempo");
        let action = compare_to_current(current_canonical, Some(dt_genre));
        return Some(veto_result(
            evidence,
            dt_genre,
            ClassificationConfidence::Low,
            action,
            vec![format!(
                "audio veto: non-dancefloor + slow → Downtempo family ({})",
                dt_genre
            )],
            Some("Artist/title context may refine within Downtempo family.".into()),
        ));
    }

    // non_dancefloor + neither ambient nor slow → Downtempo or Other
    if profile.bucket == EnergyBucket::NonDancefloor {
        let dt_genre = find_family_genre(evidence, GenreFamily::Downtempo).unwrap_or("Downtempo");
        let action = compare_to_current(current_canonical, Some(dt_genre));
        return Some(veto_result(
            evidence,
            dt_genre,
            ClassificationConfidence::Low,
            action,
            vec![format!(
                "audio veto: non-dancefloor → Downtempo/Other family ({})",
                dt_genre
            )],
            Some("Non-dancefloor track — review genre assignment.".into()),
        ));
    }

    // fast + at least low-energy → Bass family (only if enrichment agrees or is absent)
    if has_flag(profile, CharFlag::Fast) && profile.bucket != EnergyBucket::NonDancefloor {
        let has_enrichment =
            !evidence.discogs_mapped.is_empty() || evidence.beatport_genre.is_some();
        let enrichment_supports_bass = evidence
            .discogs_mapped
            .iter()
            .any(|mg| genre::genre_family(mg.genre) == GenreFamily::Bass)
            || evidence
                .beatport_genre
                .is_some_and(|g| genre::genre_family(g) == GenreFamily::Bass);

        if !has_enrichment || enrichment_supports_bass {
            let bass_genre =
                find_family_genre(evidence, GenreFamily::Bass).unwrap_or(if profile.bpm >= 168.0 {
                    "Drum & Bass"
                } else {
                    "Breakbeat"
                });
            let action = compare_to_current(current_canonical, Some(bass_genre));
            return Some(veto_result(
                evidence,
                bass_genre,
                ClassificationConfidence::Medium,
                action,
                vec![format!(
                    "audio veto: fast ({}bpm) + dancefloor → Bass family ({})",
                    profile.bpm as i32, bass_genre
                )],
                None,
            ));
        }
    }

    // low_energy + atmospheric + all enrichment dancefloor → prefer Downtempo family
    if profile.bucket == EnergyBucket::LowEnergy
        && has_flag(profile, CharFlag::Atmospheric)
        && all_enrichment_dancefloor(evidence)
    {
        let dt_genre = find_family_genre(evidence, GenreFamily::Downtempo).unwrap_or("Downtempo");
        let action = compare_to_current(current_canonical, Some(dt_genre));
        return Some(veto_result(
            evidence,
            dt_genre,
            ClassificationConfidence::Low,
            action,
            vec![
                "audio: low-energy + atmospheric but enrichment suggests dancefloor".into(),
                format!("audio suggests non-dancefloor → {}", dt_genre),
            ],
            Some(
                "Enrichment says dancefloor but audio suggests otherwise. Artist context may help."
                    .into(),
            ),
        ));
    }

    None
}

/// Check if all enrichment genres are from dancefloor families (House/Techno/Bass).
fn all_enrichment_dancefloor(evidence: &TrackEvidence) -> bool {
    let has_any = !evidence.discogs_mapped.is_empty() || evidence.beatport_genre.is_some();
    if !has_any {
        return false;
    }
    let all_dance = evidence.discogs_mapped.iter().all(|mg| {
        matches!(
            genre::genre_family(mg.genre),
            GenreFamily::House | GenreFamily::Techno | GenreFamily::Bass
        )
    });
    let bp_dance = evidence.beatport_genre.map_or(true, |g| {
        matches!(
            genre::genre_family(g),
            GenreFamily::House | GenreFamily::Techno | GenreFamily::Bass
        )
    });
    all_dance && bp_dance
}

/// Find the best genre from enrichment in a specific family.
fn find_family_genre(evidence: &TrackEvidence, family: GenreFamily) -> Option<&'static str> {
    // Check Beatport first (single, precise)
    if let Some(bp) = evidence.beatport_genre {
        if genre::genre_family(bp) == family {
            return Some(bp);
        }
    }
    // Check Discogs (highest style_count in family)
    evidence
        .discogs_mapped
        .iter()
        .filter(|mg| genre::genre_family(mg.genre) == family)
        .max_by_key(|mg| mg.style_count)
        .map(|mg| mg.genre)
}

// ---------------------------------------------------------------------------
// Step A: Gather genre votes
// ---------------------------------------------------------------------------

fn gather_votes(evidence: &TrackEvidence, audio_profile: Option<&AudioProfile>) -> Vec<GenreVote> {
    let mut votes = Vec::new();
    let effective_bpm = audio_profile.map(|p| p.bpm).unwrap_or(evidence.bpm);

    // Beatport: weight 1.0 (halved if BPM-implausible), single precise genre
    if let Some(bp) = evidence.beatport_genre {
        let plausible = bpm_plausible(bp, effective_bpm);
        votes.push(GenreVote {
            genre: bp,
            weight: if plausible { 1.0 } else { 0.5 },
            source: "beatport",
            bpm_plausible: plausible,
        });
    }

    // Discogs: weight based on style_count, normalized
    let total_styles: usize = evidence.discogs_mapped.iter().map(|m| m.style_count).sum();
    for mg in &evidence.discogs_mapped {
        let base_weight = if total_styles > 0 {
            (mg.style_count as f32 / total_styles as f32).min(1.0)
        } else {
            0.5
        };
        let plausible = bpm_plausible(mg.genre, effective_bpm);
        votes.push(GenreVote {
            genre: mg.genre,
            weight: if plausible {
                base_weight * 0.9
            } else {
                base_weight * 0.45
            },
            source: "discogs",
            bpm_plausible: plausible,
        });
    }

    // Label: weight 0.6 (or 0.4 when confirming an existing vote; halved if BPM-implausible)
    if let Some(lg) = evidence.label_genre {
        let plausible = bpm_plausible(lg, effective_bpm);
        // Check if label confirms an existing vote
        let confirms = votes.iter().any(|v| v.genre == lg);
        let weight = if confirms {
            0.4 // Reduced when confirming — avoids double-counting while still adding weight
        } else {
            0.6
        };
        votes.push(GenreVote {
            genre: lg,
            weight: if plausible { weight } else { weight * 0.5 },
            source: "label",
            bpm_plausible: plausible,
        });
    }

    votes
}

fn bpm_plausible(genre: &str, bpm: f64) -> bool {
    if bpm <= 0.0 {
        return true; // no BPM data → don't penalize
    }
    match genre::genre_bpm_range(genre) {
        Some(range) => {
            bpm >= (range.typical_min - BPM_TOLERANCE) && bpm <= (range.typical_max + BPM_TOLERANCE)
        }
        None => true, // no range defined → always plausible
    }
}

// ---------------------------------------------------------------------------
// Step C: Find consensus
// ---------------------------------------------------------------------------

fn find_consensus(
    evidence: &TrackEvidence,
    votes: &[GenreVote],
    audio_profile: Option<&AudioProfile>,
) -> (
    Option<&'static str>,
    ClassificationConfidence,
    Vec<String>,
    Vec<String>,
) {
    // Tally votes by genre
    let mut tally: HashMap<&'static str, f32> = HashMap::new();
    for v in votes {
        *tally.entry(v.genre).or_default() += v.weight;
    }

    let mut ranked: Vec<(&'static str, f32)> = tally.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    if ranked.is_empty() {
        return (None, ClassificationConfidence::Insufficient, vec![], vec![]);
    }

    let (mut top_genre, top_score) = ranked[0];
    let second = ranked.get(1);
    let margin = top_score - second.map_or(0.0, |s| s.1);
    let total_weight: f32 = ranked.iter().map(|(_, w)| w).sum();

    // Build evidence lines
    let mut ev = Vec::new();
    let mut flags = Vec::new();

    // Discogs evidence
    if !evidence.discogs_mapped.is_empty() {
        let parts: Vec<String> = evidence
            .discogs_mapped
            .iter()
            .map(|mg| {
                let bpm_note = if !bpm_plausible(mg.genre, audio_profile.map_or(0.0, |p| p.bpm)) {
                    " [bpm-implausible]"
                } else {
                    ""
                };
                format!("{}(x{}){}", mg.genre, mg.style_count, bpm_note)
            })
            .collect();
        ev.push(format!("discogs: {}", parts.join(", ")));
    }

    // Beatport evidence
    if let Some(bp) = evidence.beatport_genre {
        let raw = evidence.beatport_raw.as_deref().unwrap_or(bp);
        let bpm_note = if !bpm_plausible(bp, audio_profile.map_or(0.0, |p| p.bpm)) {
            " [bpm-implausible]"
        } else {
            ""
        };
        if raw != bp {
            ev.push(format!("beatport: {} (raw: {}){}", bp, raw, bpm_note));
        } else {
            ev.push(format!("beatport: {}{}", bp, bpm_note));
        }
    }

    // Label evidence
    if let Some(lg) = evidence.label_genre {
        let label_name = evidence.label.as_deref().unwrap_or("?");
        ev.push(format!("label: {} → {}", label_name, lg));
    }

    // Audio evidence
    if let Some(ref profile) = audio_profile {
        let bucket_name = match profile.bucket {
            EnergyBucket::NonDancefloor => "non-dancefloor",
            EnergyBucket::LowEnergy => "low-energy",
            EnergyBucket::Dancefloor => "dancefloor",
            EnergyBucket::HighEnergy => "high-energy",
        };
        let flag_names: Vec<&str> = profile
            .flags
            .iter()
            .map(|f| match f {
                CharFlag::Ambient => "ambient",
                CharFlag::Atmospheric => "atmospheric",
                CharFlag::Broken => "broken",
                CharFlag::Irregular => "irregular",
                CharFlag::Fast => "fast",
                CharFlag::Slow => "slow",
            })
            .collect();
        let audio_str = if flag_names.is_empty() {
            format!("audio: {} {}bpm", bucket_name, profile.bpm as i32)
        } else {
            format!(
                "audio: {} {} {}bpm",
                bucket_name,
                flag_names.join("+"),
                profile.bpm as i32
            )
        };
        ev.push(audio_str);
    }

    // Determine confidence
    let mut confidence = if ranked.len() == 1 && top_score >= 1.0 {
        // Single genre, strong signal
        if votes
            .iter()
            .filter(|v| v.genre == top_genre)
            .all(|v| v.bpm_plausible)
        {
            ClassificationConfidence::High
        } else {
            flags.push("bpm-implausible".into());
            ClassificationConfidence::Medium
        }
    } else if ranked.len() >= 2 {
        let (second_genre, _) = second.unwrap();
        let same_family = genre::genre_family(top_genre) == genre::genre_family(second_genre);

        if margin / total_weight > 0.4 {
            // Clear winner
            if votes
                .iter()
                .filter(|v| v.genre == top_genre)
                .all(|v| v.bpm_plausible)
            {
                ClassificationConfidence::High
            } else {
                flags.push("bpm-implausible".into());
                ClassificationConfidence::Medium
            }
        } else if margin / total_weight > 0.15 {
            // Moderate winner — check same-family specificity (C rule 9a)
            if same_family {
                top_genre = resolve_same_family_specificity(
                    top_genre,
                    second_genre,
                    audio_profile,
                    &mut ev,
                );
                ClassificationConfidence::Medium
            } else {
                ClassificationConfidence::Medium
            }
        } else {
            // Close contest
            if same_family {
                // C rule 9a/9b: depth-based resolution
                top_genre = resolve_same_family_specificity(
                    top_genre,
                    second_genre,
                    audio_profile,
                    &mut ev,
                );
                ClassificationConfidence::Low
            } else {
                // C rule 9c/9d: cross-family
                if let Some(ref profile) = audio_profile {
                    if audio_clearly_favors_family(profile, top_genre) {
                        flags.push("audio-assisted-tiebreak".into());
                        ClassificationConfidence::Low
                    } else {
                        ClassificationConfidence::Insufficient
                    }
                } else {
                    ClassificationConfidence::Insufficient
                }
            }
        }
    } else {
        // Single genre, weak signal — check BPM plausibility
        if !votes
            .iter()
            .filter(|v| v.genre == top_genre)
            .all(|v| v.bpm_plausible)
        {
            flags.push("bpm-implausible".into());
            ClassificationConfidence::Low
        } else {
            ClassificationConfidence::Medium
        }
    };

    let mut final_genre = top_genre;

    // Post-consensus: BPM preference override
    // If the top genre is BPM-implausible and there's a plausible alternative, swap.
    let effective_bpm = audio_profile.map(|p| p.bpm).unwrap_or(evidence.bpm);
    if !bpm_plausible(final_genre, effective_bpm) {
        if let Some((alt_genre, _)) = ranked
            .iter()
            .skip(1)
            .find(|(g, _)| bpm_plausible(g, effective_bpm))
        {
            ev.push(format!(
                "bpm-override: {} implausible at {}bpm → {}",
                final_genre, effective_bpm as i32, alt_genre
            ));
            flags.push("bpm-override".into());
            final_genre = alt_genre;
            // Downgrade confidence by one level — the runner-up was elevated by BPM
            // elimination, not by evidence weight
            confidence = match confidence {
                ClassificationConfidence::High => ClassificationConfidence::Medium,
                ClassificationConfidence::Medium => ClassificationConfidence::Low,
                other => other,
            };
        }
    }

    // Post-consensus: energy-based depth demotion
    // HighEnergy always demotes deep variants to their shallower parent.
    // Dancefloor demotes only when the shallower alternative has its own votes
    // (i.e. enrichment evidence supports both, and audio doesn't favour depth).
    if let Some(ref profile) = audio_profile {
        if let Some(shallower) = shallower_alternative(final_genre) {
            let demote = match profile.bucket {
                EnergyBucket::HighEnergy => true,
                EnergyBucket::Dancefloor => {
                    // Only demote if the shallower variant is also in the ranked list
                    ranked.iter().any(|(g, _)| *g == shallower)
                        && !has_flag(profile, CharFlag::Atmospheric)
                }
                _ => false,
            };
            if demote {
                ev.push(format!(
                    "depth: {}-energy audio → {} over {}",
                    if profile.bucket == EnergyBucket::HighEnergy {
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
    }

    // Step C2: Note secondary influences
    let primary_family = genre::genre_family(final_genre);
    for mg in &evidence.discogs_mapped {
        if genre::genre_family(mg.genre) != primary_family && mg.style_count >= 2 {
            ev.push(format!(
                "influence: {} (discogs x{})",
                mg.genre, mg.style_count
            ));
        }
    }
    if let Some(bp) = evidence.beatport_genre {
        if genre::genre_family(bp) != primary_family {
            ev.push(format!("influence: {} (beatport)", bp));
        }
    }

    (Some(final_genre), confidence, ev, flags)
}

/// C rule 9a: Same-family, different specificity.
/// Uses genre depth and audio profile to pick between two same-family genres.
/// Returns the preferred genre (may be different from `top`).
fn resolve_same_family_specificity(
    top: &'static str,
    second: &'static str,
    audio_profile: Option<&AudioProfile>,
    evidence: &mut Vec<String>,
) -> &'static str {
    let top_depth = genre::genre_depth(top);
    let second_depth = genre::genre_depth(second);

    if top_depth == second_depth {
        evidence.push(format!("same-family same-depth: {} vs {}", top, second));
        return top;
    }

    let (deeper, shallower) = if top_depth > second_depth {
        (top, second)
    } else {
        (second, top)
    };

    if let Some(profile) = audio_profile {
        if has_flag(profile, CharFlag::Atmospheric) || profile.bucket == EnergyBucket::LowEnergy {
            evidence.push(format!(
                "depth: audio atmospheric/low-energy → {} over {}",
                deeper, shallower
            ));
            deeper
        } else if profile.bucket == EnergyBucket::HighEnergy {
            evidence.push(format!(
                "depth: audio high-energy → {} over {}",
                shallower, deeper
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
        evidence.push(format!("depth: {} vs {}, no audio to resolve", top, second));
        top
    }
}

/// Map "deep" genre variants to their shallower parent genre.
/// Used by the high-energy depth demotion: if audio is HighEnergy, a "deep" genre
/// is unlikely — prefer the shallower driving variant.
fn shallower_alternative(genre: &str) -> Option<&'static str> {
    match genre {
        "Deep Techno" | "Dub Techno" | "Ambient Techno" | "Drone Techno" => Some("Techno"),
        "Deep House" => Some("House"),
        _ => None,
    }
}

/// C rule 9c: Check if audio flags clearly favor one family.
fn audio_clearly_favors_family(profile: &AudioProfile, candidate: &str) -> bool {
    let family = genre::genre_family(candidate);
    match family {
        GenreFamily::Downtempo => {
            profile.bucket == EnergyBucket::LowEnergy && has_flag(profile, CharFlag::Atmospheric)
        }
        GenreFamily::Bass => {
            has_flag(profile, CharFlag::Fast)
                || (has_flag(profile, CharFlag::Broken)
                    && profile.bucket >= EnergyBucket::Dancefloor)
        }
        GenreFamily::Techno => {
            profile.bucket >= EnergyBucket::Dancefloor
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 125.0
        }
        GenreFamily::House => {
            profile.bucket == EnergyBucket::Dancefloor
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 118.0
                && profile.bpm <= 132.0
        }
        _ => false,
    }
}

// Implement PartialOrd for EnergyBucket for comparisons
impl PartialOrd for EnergyBucket {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EnergyBucket {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let rank = |b: &EnergyBucket| -> u8 {
            match b {
                EnergyBucket::NonDancefloor => 0,
                EnergyBucket::LowEnergy => 1,
                EnergyBucket::Dancefloor => 2,
                EnergyBucket::HighEnergy => 3,
            }
        };
        rank(self).cmp(&rank(other))
    }
}

// ---------------------------------------------------------------------------
// Step D: Audio-only inference
// ---------------------------------------------------------------------------

fn audio_only_inference(
    evidence: &TrackEvidence,
    audio_profile: Option<&AudioProfile>,
) -> (
    Option<&'static str>,
    ClassificationConfidence,
    Vec<String>,
    Vec<String>,
) {
    let Some(profile) = audio_profile else {
        return (
            None,
            ClassificationConfidence::Insufficient,
            vec!["no enrichment data, no audio analysis".into()],
            vec!["no-data".into()],
        );
    };

    let dc = evidence
        .audio
        .as_ref()
        .and_then(|a| a.dynamic_complexity)
        .unwrap_or(0.0);
    let rr = evidence
        .audio
        .as_ref()
        .and_then(|a| a.rhythm_regularity)
        .unwrap_or(0.85);
    let sc = evidence
        .audio
        .as_ref()
        .and_then(|a| a.spectral_centroid_mean);
    // danceability is used indirectly via the audio profile's energy bucket

    let mut candidates: Vec<&'static str> = Vec::new();
    let mut ev = Vec::new();

    // D.1: Broad bucket
    match profile.bucket {
        EnergyBucket::NonDancefloor => {
            if dc > 10.0 {
                candidates.push("Ambient");
                ev.push("D.1: non-dancefloor + high dynamic complexity → Ambient".into());
            } else if dc > 5.0 {
                candidates.extend_from_slice(&["Experimental", "Ambient"]);
                ev.push("D.1: non-dancefloor + moderate complexity → Experimental/Ambient".into());
            } else {
                candidates.extend_from_slice(&["Downtempo", "Experimental"]);
                ev.push("D.1: non-dancefloor + low complexity → Downtempo/Experimental".into());
            }
        }
        EnergyBucket::LowEnergy => {
            if profile.bpm > 145.0 {
                // High BPM + low energy → still likely bass family, not Electro/IDM
                candidates.extend_from_slice(&["Jungle", "Breakbeat"]);
                ev.push(format!(
                    "D.1: low-energy but fast ({}bpm) → Jungle/Breakbeat",
                    profile.bpm as i32
                ));
            } else if dc > 5.0 {
                candidates.extend_from_slice(&["Downtempo", "Ambient Techno"]);
                ev.push("D.1: low-energy + atmospheric → Downtempo/Ambient Techno".into());
            } else {
                candidates.extend_from_slice(&["Electro", "IDM"]);
                ev.push("D.1: low-energy + low complexity → Electro/IDM".into());
            }
        }
        EnergyBucket::Dancefloor | EnergyBucket::HighEnergy => {
            // D.2: Subgenre by BPM x rhythm regularity
            let bpm = profile.bpm;
            if bpm > 155.0 {
                candidates.extend_from_slice(&["Drum & Bass", "Jungle"]);
                ev.push(format!("D.2: fast ({}bpm) → D&B/Jungle", bpm as i32));
            } else if bpm >= 135.0 && rr > 0.9 {
                candidates.extend_from_slice(&["Trance", "Hard Techno"]);
                ev.push(format!(
                    "D.2: {}bpm + regular rhythm → Trance/Hard Techno",
                    bpm as i32
                ));
            } else if bpm >= 128.0 && rr > 0.9 {
                candidates.push("Techno");
                ev.push(format!("D.2: {}bpm + regular rhythm → Techno", bpm as i32));
            } else if bpm >= 120.0 && bpm <= 135.0 && rr > 0.9 {
                candidates.extend_from_slice(&["Techno", "Tech House", "House"]);
                ev.push(format!(
                    "D.2: {}bpm + regular → Techno/Tech House/House",
                    bpm as i32
                ));
            } else if bpm >= 118.0 && bpm <= 130.0 && rr >= 0.8 {
                candidates.extend_from_slice(&["House", "Deep House"]);
                ev.push(format!(
                    "D.2: {}bpm + moderate rhythm → House/Deep House",
                    bpm as i32
                ));
            } else if bpm >= 120.0 && bpm <= 140.0 && rr < 0.8 {
                candidates.extend_from_slice(&["Breakbeat", "Garage"]);
                ev.push(format!(
                    "D.2: {}bpm + broken rhythm → Breakbeat/Garage",
                    bpm as i32
                ));
            } else if bpm < 120.0 {
                candidates.extend_from_slice(&["Deep House", "Downtempo"]);
                ev.push(format!(
                    "D.2: slow ({}bpm) → Deep House/Downtempo",
                    bpm as i32
                ));
            } else {
                candidates.push("House");
                ev.push(format!(
                    "D.2: {}bpm, unmatched → House (fallback)",
                    bpm as i32
                ));
            }
        }
    }

    // D.3: Spectral centroid refinement — reorder candidates by tonal character.
    // Preference lists are aesthetic/provisional — replace with empirically derived
    // rankings once issue #19 produces real classification data by centroid range.
    if let Some(centroid) = sc
        && candidates.len() > 1
    {
        let first_before = candidates[0];
        let (centroid_hint, reordered) = if centroid < 1200.0 {
            // Dark/warm — prefer deeper, darker genres
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
            candidates
                .sort_by_key(|g| preferred.iter().position(|p| p == g).unwrap_or(usize::MAX));
            (
                if centroid < 600.0 {
                    "very-low centroid"
                } else {
                    "low centroid"
                },
                candidates[0] != first_before,
            )
        } else if centroid >= 2500.0 {
            // Bright/aggressive — prefer energetic, brighter genres
            let preferred: &[&str] = &[
                "Hard Techno",
                "Breakbeat",
                "Tech House",
                "Drum & Bass",
                "House",
                "Electro",
            ];
            candidates
                .sort_by_key(|g| preferred.iter().position(|p| p == g).unwrap_or(usize::MAX));
            (
                if centroid >= 4000.0 {
                    "high centroid"
                } else {
                    "mid-high centroid"
                },
                candidates[0] != first_before,
            )
        } else {
            // Mid centroid (1200–2500): D.2 order is appropriate
            ("mid centroid", false)
        };

        if reordered {
            ev.push(format!(
                "D.3: {} → {} over {}",
                centroid_hint, candidates[0], first_before
            ));
        } else {
            ev.push(format!("D.3: {} (confirms D.2 order)", centroid_hint));
        }
    }

    // D.4: Confidence
    let (genre, confidence) = if candidates.len() == 1 {
        (
            genre::canonical_genre_name(candidates[0]).or(Some(candidates[0])),
            ClassificationConfidence::Low,
        )
    } else if candidates.is_empty() {
        (None, ClassificationConfidence::Insufficient)
    } else {
        // Pick the most plausible by BPM, or first if all equal
        let best = candidates
            .iter()
            .find(|&&g| bpm_plausible(g, profile.bpm))
            .or(candidates.first())
            .copied();
        (best, ClassificationConfidence::Insufficient)
    };

    let mut flags = vec!["audio-only".into()];
    if !evidence.has_discogs && !evidence.has_beatport {
        flags.push("no-enrichment".into());
    }

    (genre, confidence, ev, flags)
}

// ---------------------------------------------------------------------------
// Step E: Compare to current genre
// ---------------------------------------------------------------------------

fn resolve_current_canonical(current_genre: &str) -> Option<&'static str> {
    if current_genre.is_empty() {
        return None;
    }
    genre::canonical_genre_name(current_genre)
        .or_else(|| genre::canonical_genre_from_alias(current_genre))
}

fn compare_to_current(
    current_canonical: Option<&str>,
    recommended: Option<&str>,
) -> ClassificationAction {
    match (current_canonical, recommended) {
        (Some(cur), Some(rec)) if cur == rec => ClassificationAction::Confirm,
        (Some(_), Some(_)) => ClassificationAction::Conflict,
        (None, Some(_)) => ClassificationAction::Suggest,
        (_, None) => ClassificationAction::Manual,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_candidates(votes: &[GenreVote], top_genre: Option<&str>) -> Vec<GenreCandidate> {
    let mut tally: HashMap<&'static str, (f32, bool)> = HashMap::new();
    for v in votes {
        let entry = tally.entry(v.genre).or_insert((0.0, true));
        entry.0 += v.weight;
        if !v.bpm_plausible {
            entry.1 = false;
        }
    }

    let mut candidates: Vec<GenreCandidate> = tally
        .into_iter()
        .filter(|(g, _)| Some(*g) != top_genre) // exclude the primary recommendation
        .map(|(g, (score, bpm))| GenreCandidate {
            genre: g,
            score,
            bpm_plausible: bpm,
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(3);
    candidates
}

fn build_review_hint(evidence: &TrackEvidence, _flags: &[String]) -> String {
    let mut hints = Vec::new();
    if !evidence.has_discogs && !evidence.has_beatport {
        hints.push("No enrichment data available");
    }
    if !evidence.has_audio {
        hints.push("No audio analysis available");
    }
    if !evidence.artist.is_empty() {
        hints.push("Artist/title context may help disambiguate");
    }
    if evidence.label.is_some() && evidence.label_genre.is_none() {
        hints.push("Label not in mapping table — LLM may recognize it");
    }
    if hints.is_empty() {
        "Conflicting evidence — review candidates".into()
    } else {
        hints.join(". ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_evidence(current: &str) -> TrackEvidence {
        TrackEvidence {
            track_id: "test-1".into(),
            artist: "Test Artist".into(),
            title: "Test Track".into(),
            current_genre: current.into(),
            bpm: 0.0,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: false,
            has_beatport: false,
            has_audio: false,
        }
    }

    #[test]
    fn no_data_returns_insufficient() {
        let ev = make_evidence("");
        let result = classify_track(&ev);
        assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
        assert_eq!(result.action, ClassificationAction::Manual);
        assert!(result.genre.is_none());
    }

    #[test]
    fn beatport_only_returns_medium() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 132.0,
            stratum_bpm: Some(132.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert!(matches!(
            result.confidence,
            ClassificationConfidence::High | ClassificationConfidence::Medium
        ));
        assert_eq!(result.action, ClassificationAction::Suggest);
    }

    #[test]
    fn full_consensus_returns_high() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Techno",
            style_count: 3,
        }];
        ev.has_discogs = true;
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 132.0,
            stratum_bpm: Some(132.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
        assert_eq!(result.action, ClassificationAction::Suggest);
    }

    #[test]
    fn confirms_correct_current_genre() {
        let mut ev = make_evidence("Techno");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 132.0,
            stratum_bpm: Some(132.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.action, ClassificationAction::Confirm);
    }

    #[test]
    fn detects_conflict() {
        let mut ev = make_evidence("House");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Techno",
            style_count: 3,
        }];
        ev.has_discogs = true;
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 132.0,
            stratum_bpm: Some(132.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.action, ClassificationAction::Conflict);
    }

    #[test]
    fn bpm_implausible_downgrades_confidence() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Deep House");
        ev.has_beatport = true;
        // BPM 140 is way outside Deep House range (118-126)
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 140.0,
            stratum_bpm: Some(140.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        // Should still suggest Deep House but at lower confidence due to BPM
        assert_eq!(result.genre, Some("Deep House"));
        assert!(matches!(
            result.confidence,
            ClassificationConfidence::Medium | ClassificationConfidence::Low
        ));
    }

    #[test]
    fn audio_veto_ambient() {
        let mut ev = make_evidence("");
        // Enrichment says Techno, but audio is clearly non-dancefloor ambient
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 100.0,
            stratum_bpm: Some(100.0),
            bpm_agreement: Some(true),
            danceability: Some(0.5),        // non-dancefloor
            dynamic_complexity: Some(12.0), // ambient
            rhythm_regularity: Some(0.3),
            spectral_centroid_mean: Some(400.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Ambient"));
        assert!(result.flags.contains(&"audio-vetoed".to_string()));
    }

    #[test]
    fn label_confirms_enrichment() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.label = Some("Tresor".into());
        ev.label_genre = Some("Techno"); // Tresor → Techno
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 132.0,
            stratum_bpm: Some(132.0),
            bpm_agreement: Some(true),
            danceability: Some(2.0),
            dynamic_complexity: Some(3.0),
            rhythm_regularity: Some(0.92),
            spectral_centroid_mean: Some(1800.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
    }

    #[test]
    fn depth_prefers_shallower_when_high_energy() {
        let mut ev = make_evidence("");
        // Discogs says Deep Techno, Beatport says Techno
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        // Audio is high energy — should prefer Techno over Deep Techno
        ev.audio = Some(AudioFeatures {
            rekordbox_bpm: 135.0,
            stratum_bpm: Some(135.0),
            bpm_agreement: Some(true),
            danceability: Some(2.8), // high energy
            dynamic_complexity: Some(2.0),
            rhythm_regularity: Some(0.95),
            spectral_centroid_mean: Some(2500.0),
        });
        ev.has_audio = true;
        let result = classify_track(&ev);
        // With high energy, should prefer Techno (shallower) over Deep Techno
        assert_eq!(result.genre, Some("Techno"));
        assert!(result.evidence.iter().any(|e| e.contains("depth:")));
    }

    // -----------------------------------------------------------------------
    // Collection-derived test cases
    // -----------------------------------------------------------------------

    // 1. Marcel Dettmann - Aim: full consensus Techno
    // Discogs Techno(x1), Beatport Techno, label Ostgut Ton → Techno, BPM 130
    #[test]
    fn collection_dettmann_aim_full_consensus_techno() {
        let ev = TrackEvidence {
            track_id: "59114728".into(),
            artist: "Marcel Dettmann".into(),
            title: "Aim".into(),
            current_genre: "".into(),
            bpm: 130.0,
            discogs_mapped: vec![MappedGenre {
                genre: "Techno",
                style_count: 1,
            }],
            beatport_genre: Some("Techno"),
            beatport_raw: Some("Techno (Peak Time / Driving)".into()),
            label: Some("Ostgut Ton".into()),
            label_genre: Some("Techno"),
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
        assert_eq!(result.action, ClassificationAction::Suggest);
    }

    // 2. Kassian - Actions: BPM-implausible Deep House (132 > 126+5=131)
    // Beatport Deep House only, BPM 132, no Discogs genres
    #[test]
    fn collection_kassian_actions_bpm_implausible_deep_house() {
        let ev = TrackEvidence {
            track_id: "73755639".into(),
            artist: "Kassian".into(),
            title: "Actions".into(),
            current_genre: "".into(),
            bpm: 132.0,
            discogs_mapped: vec![],
            beatport_genre: Some("Deep House"),
            beatport_raw: Some("Deep House".into()),
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        // Deep House is BPM-implausible at 132 (range 118-126+5=131)
        // Should NOT be high confidence
        assert!(
            !matches!(result.confidence, ClassificationConfidence::High),
            "BPM-implausible single vote should not be High confidence"
        );
        assert!(
            result.flags.contains(&"bpm-implausible".to_string()),
            "Should flag BPM implausibility"
        );
    }

    // 3. Vadz - Abstraction: Beatport breaks 5-way Discogs split
    // Discogs: Electro, IDM, Minimal, Techno, Trance. Beatport: Techno. BPM 129
    #[test]
    fn collection_vadz_abstraction_beatport_breaks_split() {
        let ev = TrackEvidence {
            track_id: "121445057".into(),
            artist: "Vadz".into(),
            title: "Abstraction".into(),
            current_genre: "".into(),
            bpm: 129.31,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Electro",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "IDM",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Minimal",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Techno",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Trance",
                    style_count: 1,
                },
            ],
            beatport_genre: Some("Techno"),
            beatport_raw: Some("Techno (Peak Time / Driving)".into()),
            label: Some("Russian Techno".into()),
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
    }

    // 4. Dead Man's Chest - All About U: audio veto to bass family
    // No enrichment genres, audio: 157bpm, danceability 1.01, dc 3.95, rr 1.04, sc 1395
    #[test]
    fn collection_dmc_all_about_u_audio_veto_bass() {
        let ev = TrackEvidence {
            track_id: "141838084".into(),
            artist: "Dead Man's Chest".into(),
            title: "All About U (Pt.1 Dreamscapes)".into(),
            current_genre: "".into(),
            bpm: 156.97,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 156.97,
                stratum_bpm: Some(156.77),
                bpm_agreement: Some(true),
                danceability: Some(1.01),
                dynamic_complexity: Some(3.95),
                rhythm_regularity: Some(1.04),
                spectral_centroid_mean: Some(1395.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Breakbeat"));
        assert!(result.flags.contains(&"audio-vetoed".to_string()));
    }

    // 5. Gacha Bakradze - Attention: Beatport D&B breaks Discogs Electro/IDM
    // Discogs: Electro, IDM. Beatport: Drum & Bass. BPM 180
    #[test]
    fn collection_bakradze_attention_beatport_dnb() {
        let ev = TrackEvidence {
            track_id: "10208805".into(),
            artist: "Gacha Bakradze".into(),
            title: "Attention".into(),
            current_genre: "".into(),
            bpm: 180.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Electro",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "IDM",
                    style_count: 1,
                },
            ],
            beatport_genre: Some("Drum & Bass"),
            beatport_raw: Some("Drum & Bass".into()),
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Drum & Bass"));
    }

    // 6. Alarico - 0 Kelvin: Deep Techno BPM-implausible, Techno plausible
    // Discogs: Techno(x1). Beatport: Deep Techno. BPM 145. No audio.
    // Deep Techno range 120-132+5=137 < 145 → implausible
    // Techno range 128-140+5=145 → plausible (just barely)
    #[test]
    fn collection_alarico_0_kelvin_depth_bpm_override() {
        let ev = TrackEvidence {
            track_id: "146271440".into(),
            artist: "Alarico".into(),
            title: "0 Kelvin".into(),
            current_genre: "".into(),
            bpm: 145.0,
            discogs_mapped: vec![MappedGenre {
                genre: "Techno",
                style_count: 1,
            }],
            beatport_genre: Some("Deep Techno"),
            beatport_raw: Some("Techno (Raw / Deep / Hypnotic)".into()),
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("Techno"),
            "Deep Techno is BPM-implausible at 145, should prefer Techno"
        );
    }

    // 7. Efdemin - Aachen: label tips depth to Techno
    // Discogs: Techno(x1). Beatport: Deep Techno. Label: Ostgut Ton → Techno. BPM 135.
    #[test]
    fn collection_efdemin_aachen_label_tips_depth() {
        let ev = TrackEvidence {
            track_id: "102211531".into(),
            artist: "Efdemin".into(),
            title: "Aachen".into(),
            current_genre: "".into(),
            bpm: 135.0,
            discogs_mapped: vec![MappedGenre {
                genre: "Techno",
                style_count: 1,
            }],
            beatport_genre: Some("Deep Techno"),
            beatport_raw: Some("Techno (Raw / Deep / Hypnotic)".into()),
            label: Some("Ostgut Ton".into()),
            label_genre: Some("Techno"),
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("Techno"),
            "Label Ostgut Ton → Techno should tip depth resolution to Techno"
        );
    }

    // 8. Busy Twist - Auntie Fatty: audio-only 134bpm regular → Techno
    // No enrichment, audio: 134bpm, danceability 1.74, dc 3.64, rr 1.07, sc 1244
    #[test]
    fn collection_busy_twist_auntie_fatty_audio_only() {
        let ev = TrackEvidence {
            track_id: "37348712".into(),
            artist: "Busy Twist".into(),
            title: "Auntie Fatty (DrumTalk Remix)".into(),
            current_genre: "".into(),
            bpm: 134.31,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 134.31,
                stratum_bpm: Some(134.0),
                bpm_agreement: Some(true),
                danceability: Some(1.74),
                dynamic_complexity: Some(3.64),
                rhythm_regularity: Some(1.07),
                spectral_centroid_mean: Some(1244.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert!(result.flags.contains(&"audio-only".to_string()));
    }

    // 9. prince of denmark - (in the end): 4-way Discogs split, insufficient
    // Discogs: Ambient, Dub Techno, House, Techno — all x1. No Beatport. BPM 126.
    #[test]
    fn collection_pod_ghost_4way_split_insufficient() {
        let ev = TrackEvidence {
            track_id: "5886970".into(),
            artist: "prince of denmark".into(),
            title: "(in the end) the ghost ran out of memory".into(),
            current_genre: "".into(),
            bpm: 126.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Ambient",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Dub Techno",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "House",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Techno",
                    style_count: 1,
                },
            ],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert!(
            matches!(
                result.confidence,
                ClassificationConfidence::Low | ClassificationConfidence::Insufficient
            ),
            "4-way even split should be Low or Insufficient depending on sort order, got {:?}",
            result.confidence
        );
    }

    // 10. Hojo - 16 O's: no data → manual
    #[test]
    fn collection_hojo_16_os_no_data_manual() {
        let ev = TrackEvidence {
            track_id: "22105191".into(),
            artist: "Hojo feat. Novelist".into(),
            title: "16 O's".into(),
            current_genre: "".into(),
            bpm: 145.0,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(result.action, ClassificationAction::Manual);
        assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
    }

    // 11. Daniel Stefanik - #four: BPM 119 makes Techno implausible
    // Beatport: Techno. Discogs: House, Tech House, Techno. BPM 119. Current: Tech House.
    // Techno range 128-140, with tolerance 123-145. BPM 119 < 123 → implausible
    // Tech House range 124-132, with tolerance 119-137. BPM 119 ≥ 119 → plausible
    #[test]
    fn collection_stefanik_four_bpm_prefers_tech_house() {
        let ev = TrackEvidence {
            track_id: "109176001".into(),
            artist: "Daniel Stefanik".into(),
            title: "#four".into(),
            current_genre: "Tech House".into(),
            bpm: 119.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "House",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Tech House",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Techno",
                    style_count: 1,
                },
            ],
            beatport_genre: Some("Techno"),
            beatport_raw: Some("Techno (Peak Time / Driving)".into()),
            label: None,
            label_genre: None,
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_ne!(
            result.genre,
            Some("Techno"),
            "Techno is BPM-implausible at 119, should prefer a plausible alternative"
        );
        // Tech House or House are both acceptable
        assert!(
            result.genre == Some("Tech House") || result.genre == Some("House"),
            "Should prefer BPM-plausible Tech House or House over implausible Techno, got {:?}",
            result.genre
        );
    }

    // 12. Baltra - 16 Pads: confirms existing Breakbeat
    // Beatport: Breakbeat. Audio: 130bpm, danceability 1.15. Current: Breakbeat.
    #[test]
    fn collection_baltra_16_pads_confirms_breakbeat() {
        let ev = TrackEvidence {
            track_id: "21091446".into(),
            artist: "Baltra".into(),
            title: "16 Pads [update 26092017] v2 (FIX 4 MPC)".into(),
            current_genre: "Breakbeat".into(),
            bpm: 130.0,
            discogs_mapped: vec![],
            beatport_genre: Some("Breakbeat"),
            beatport_raw: Some("Breaks / Breakbeat / UK Bass".into()),
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 130.0,
                stratum_bpm: Some(162.92),
                bpm_agreement: Some(false),
                danceability: Some(1.15),
                dynamic_complexity: Some(3.35),
                rhythm_regularity: Some(1.16),
                spectral_centroid_mean: Some(1585.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Breakbeat"));
        assert_eq!(result.action, ClassificationAction::Confirm);
    }

    // 13. Flying Lotus - ...And The World Laughs With You: enrichment overrides bass veto
    // Discogs: Downtempo, Experimental, IDM. Beatport: unmapped. Audio: 165bpm, low-energy, irregular.
    // Bass veto should NOT fire because enrichment points to Downtempo/IDM family, not bass.
    #[test]
    fn collection_flylo_enrichment_overrides_bass_veto() {
        let ev = TrackEvidence {
            track_id: "192049791".into(),
            artist: "Flying Lotus".into(),
            title: "...And The World Laughs With You".into(),
            current_genre: "IDM".into(),
            bpm: 164.7,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Downtempo",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Experimental",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "IDM",
                    style_count: 1,
                },
            ],
            beatport_genre: None, // "Electronica" doesn't map
            beatport_raw: Some("Electronica".into()),
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 164.7,
                stratum_bpm: Some(164.79),
                bpm_agreement: Some(true),
                danceability: Some(1.19),
                dynamic_complexity: Some(4.18),
                rhythm_regularity: Some(0.68),
                spectral_centroid_mean: Some(1919.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_ne!(
            result.genre,
            Some("Breakbeat"),
            "Bass veto should not override enrichment that says IDM/Experimental/Downtempo"
        );
        // IDM, Experimental, or Downtempo are all acceptable
        let acceptable = [Some("IDM"), Some("Experimental"), Some("Downtempo")];
        assert!(
            acceptable.contains(&result.genre),
            "Expected IDM/Experimental/Downtempo, got {:?}",
            result.genre
        );
    }

    // 14. Dub Tractor - 104 Dub: Downtempo consensus
    // Discogs: Downtempo. Audio: 104bpm, low-energy, atmospheric (dc=7.0), irregular.
    #[test]
    fn collection_dub_tractor_104_dub_downtempo() {
        let ev = TrackEvidence {
            track_id: "44891033".into(),
            artist: "Dub Tractor".into(),
            title: "104 Dub".into(),
            current_genre: "Breakbeat".into(),
            bpm: 104.02,
            discogs_mapped: vec![MappedGenre {
                genre: "Downtempo",
                style_count: 1,
            }],
            beatport_genre: None, // "Electronica" unmapped
            beatport_raw: Some("Electronica".into()),
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 104.02,
                stratum_bpm: Some(51.89),
                bpm_agreement: Some(false),
                danceability: Some(1.36),
                dynamic_complexity: Some(7.02),
                rhythm_regularity: Some(0.97),
                spectral_centroid_mean: Some(1161.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Downtempo"));
        assert_eq!(result.action, ClassificationAction::Conflict);
    }

    // 15. Gallery S - 100 Skyward Fist: BPM filters Jungle, House wins
    // Discogs: House, Jungle (BPM-implausible at 128), Techno. Beatport: House. Audio: 128bpm.
    #[test]
    fn collection_gallery_s_100_skyward_fist_house() {
        let ev = TrackEvidence {
            track_id: "230625882".into(),
            artist: "Gallery S".into(),
            title: "100 Skyward Fist".into(),
            current_genre: "Broken Beat".into(),
            bpm: 128.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "House",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Jungle",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Techno",
                    style_count: 1,
                },
            ],
            beatport_genre: Some("House"),
            beatport_raw: Some("House".into()),
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 128.0,
                stratum_bpm: Some(127.91),
                bpm_agreement: Some(true),
                danceability: Some(1.42),
                dynamic_complexity: Some(1.76),
                rhythm_regularity: Some(0.84),
                spectral_centroid_mean: Some(1923.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("House"));
        assert_eq!(result.action, ClassificationAction::Conflict);
    }

    // 16. Bjarki - ( . )_( . ): 154bpm should not give Electro
    // No enrichment, audio: 154bpm, low-energy (1.36), dc 3.47, rr 1.10, sc 1341
    // At 154bpm, even low energy, Electro/IDM are unlikely. Should be bass family.
    #[test]
    fn collection_bjarki_dots_high_bpm_not_electro() {
        let ev = TrackEvidence {
            track_id: "111299487".into(),
            artist: "Bjarki".into(),
            title: "( . )_( . )".into(),
            current_genre: "Techno".into(),
            bpm: 154.0,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 154.0,
                stratum_bpm: Some(153.93),
                bpm_agreement: Some(true),
                danceability: Some(1.36),
                dynamic_complexity: Some(3.47),
                rhythm_regularity: Some(1.10),
                spectral_centroid_mean: Some(1341.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_ne!(
            result.genre,
            Some("Electro"),
            "At 154bpm, Electro is implausible even with low energy"
        );
        // Jungle, Breakbeat, or Drum & Bass would be acceptable
        let bass_family = [Some("Jungle"), Some("Breakbeat"), Some("Drum & Bass")];
        assert!(
            bass_family.contains(&result.genre),
            "At 154bpm, should suggest bass family, got {:?}",
            result.genre
        );
    }

    // 17. Slim Steve - 3pm Rave: label-driven House
    // No enrichment genres, label Lobster Theremin → House, audio: 136bpm dancefloor
    #[test]
    fn collection_slim_steve_3pm_rave_label_house() {
        let ev = TrackEvidence {
            track_id: "195038453".into(),
            artist: "Slim Steve".into(),
            title: "3pm Rave".into(),
            current_genre: "Techno".into(),
            bpm: 136.0,
            discogs_mapped: vec![],
            beatport_genre: None,
            beatport_raw: None,
            label: Some("Lobster Theremin".into()),
            label_genre: Some("House"),
            audio: Some(AudioFeatures {
                rekordbox_bpm: 136.0,
                stratum_bpm: Some(136.58),
                bpm_agreement: Some(true),
                danceability: Some(1.69),
                dynamic_complexity: Some(6.62),
                rhythm_regularity: Some(0.95),
                spectral_centroid_mean: Some(2205.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("House"));
        assert_eq!(result.action, ClassificationAction::Conflict);
    }

    // 18. Anthony Parasole - 7EVEN: high energy should veto Deep Techno
    // Beatport: Deep Techno. Audio: 132bpm, danceability 3.62 (very high energy!)
    // High energy contradicts "deep" — should prefer Techno
    #[test]
    fn collection_parasole_7even_high_energy_vetoes_deep() {
        let ev = TrackEvidence {
            track_id: "165247703".into(),
            artist: "Anthony Parasole".into(),
            title: "7EVEN".into(),
            current_genre: "Techno".into(),
            bpm: 132.0,
            discogs_mapped: vec![],
            beatport_genre: Some("Deep Techno"),
            beatport_raw: Some("Techno (Raw / Deep / Hypnotic)".into()),
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 132.0,
                stratum_bpm: Some(164.97),
                bpm_agreement: Some(false),
                danceability: Some(3.62),
                dynamic_complexity: Some(2.71),
                rhythm_regularity: Some(1.00),
                spectral_centroid_mean: Some(1572.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("Techno"),
            "High energy (danceability 3.62) should veto Deep Techno in favor of Techno"
        );
        assert_eq!(result.action, ClassificationAction::Confirm);
    }

    // 19. Skee Mask - 50 Euro To Break Boost: confirms Breakbeat
    // Discogs: Ambient, Breakbeat. Beatport: Breakbeat. Label: Ilian Tape → Techno. BPM 132.
    #[test]
    fn collection_skee_mask_50_euro_confirms_breakbeat() {
        let ev = TrackEvidence {
            track_id: "52711233".into(),
            artist: "Skee Mask".into(),
            title: "50 Euro To Break Boost".into(),
            current_genre: "Breakbeat".into(),
            bpm: 132.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Ambient",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Breakbeat",
                    style_count: 1,
                },
            ],
            beatport_genre: Some("Breakbeat"),
            beatport_raw: Some("Breaks / Breakbeat / UK Bass".into()),
            label: Some("Ilian Tape".into()),
            label_genre: Some("Techno"),
            audio: None,
            has_discogs: true,
            has_beatport: true,
            has_audio: false,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Breakbeat"));
        assert_eq!(result.action, ClassificationAction::Confirm);
    }

    // 20. Soulphiction - 24-7 Love Affair: House over Deep House
    // Discogs: Deep House, House. Audio: 122bpm, dancefloor, very low centroid (601).
    #[test]
    fn collection_soulphiction_24_7_house_over_deep_house() {
        let ev = TrackEvidence {
            track_id: "144593998".into(),
            artist: "Soulphiction".into(),
            title: "24-7 Love Affair".into(),
            current_genre: "Deep House".into(),
            bpm: 122.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Deep House",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "House",
                    style_count: 1,
                },
            ],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: Some(AudioFeatures {
                rekordbox_bpm: 122.0,
                stratum_bpm: Some(121.60),
                bpm_agreement: Some(true),
                danceability: Some(2.06),
                dynamic_complexity: Some(4.30),
                rhythm_regularity: Some(0.92),
                spectral_centroid_mean: Some(601.0),
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("House"),
            "User confirms this track is House, not Deep House"
        );
        assert_eq!(result.action, ClassificationAction::Conflict);
    }
}
