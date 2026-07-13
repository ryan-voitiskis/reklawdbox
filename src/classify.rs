//! Server-side genre decision tree.
//!
//! Applies evidence-based genre classification using cached enrichment data
//! (Discogs, Beatport, label mapping) and audio analysis features (Essentia).

use std::collections::HashMap;

use crate::audio_profile::{self, ProfileRegistry};
#[allow(unused_imports)]
pub(crate) use crate::domain::classification::{
    AudioFeatures, ClassificationAction, ClassificationConfidence, ClassificationResult,
    CompactClassificationResult, GenreCandidate, MappedGenre, TrackEvidence,
};
use crate::genre::{self, GenreFamily};

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
    /// `key_confidence` in `(0.0, 0.1)`: no clear tonal centre.
    Atonal,
    /// `decay_mid_tau > 200ms`: lingering mid-band decay/reverb tail.
    LongTail,
    /// `loudness_range < 1 LU` on a full-length track: compressed club master.
    Compressed,
}

struct AudioProfile {
    bucket: Option<EnergyBucket>,
    flags: Vec<CharFlag>,
    bpm: f64,
    centroid: Option<f64>,
    rhythm_regularity: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct BpmContext {
    effective_bpm: f64,
    fallback: Option<BpmFallback>,
}

#[derive(Debug, Clone, Copy)]
struct BpmFallback {
    rekordbox_bpm: f64,
    detector_bpm: f64,
}

/// A single vote for a genre from one evidence source.
struct GenreVote {
    genre: &'static str,
    weight: f32,
    source: &'static str,
    bpm_plausible: bool,
}

/// BPM tolerance: ±5 BPM around the defined genre range.
const BPM_TOLERANCE: f64 = 5.0;

/// Spectral centroid threshold: sub-bass dominated (ambient, drone, dub techno).
const CENTROID_VERY_LOW: f64 = 600.0;
/// Spectral centroid threshold: dark timbre (dub techno, deep techno).
const CENTROID_DARK: f64 = 1200.0;
/// Mid-band decay threshold for reverb-heavy long-tail material.
const LONG_TAIL_DECAY_MS: f64 = 200.0;
/// Loudness-range threshold for heavily mastered club tracks.
const COMPRESSED_LOUDNESS_RANGE_LU: f64 = 1.0;
/// Short clips can report artificially narrow loudness range.
const COMPRESSED_MIN_DURATION_SECONDS: f64 = 60.0;
/// Maximum relative difference for Stratum/Essentia BPM detector consensus.
const BPM_DETECTOR_CONSENSUS_TOLERANCE: f64 = 0.03;
/// Low-weight conjunctive boost, aligned with the audio-profile vote cap.
const AUDIO_RULE_BOOST: f32 = 0.5;

#[cfg(test)]
pub(crate) fn classify_track(evidence: &TrackEvidence) -> ClassificationResult {
    classify_track_with_profiles(evidence, None)
}

pub(crate) fn classify_track_with_profiles(
    evidence: &TrackEvidence,
    profile_registry: Option<&ProfileRegistry>,
) -> ClassificationResult {
    let audio_profile = evidence.audio.as_ref().map(compute_audio_profile);
    let bpm_context = compute_bpm_context(
        evidence.audio.as_ref(),
        audio_profile.as_ref(),
        evidence.bpm,
    );

    if let Some(profile) = audio_profile.as_ref()
        && let Some(mut result) = check_audio_vetoes(evidence, profile)
    {
        add_missing_audio_flags(evidence, &mut result.flags);
        return result;
    }

    let (votes, affinities, calibrated_coverage_missing) = gather_votes(
        evidence,
        audio_profile.as_ref(),
        profile_registry,
        bpm_context,
    );

    let (genre, confidence, mut ev_lines, mut flags) = if votes.is_empty() {
        audio_only_inference(evidence, audio_profile.as_ref())
    } else {
        find_consensus(evidence, &votes, audio_profile.as_ref(), bpm_context)
    };

    // Add audio-profile evidence strings for affinities that contributed
    for a in &affinities {
        if a.vote_weight >= 0.1 {
            ev_lines.push(audio_profile::format_evidence(a));
        }
    }
    if calibrated_coverage_missing {
        ev_lines.push("audio-profile: insufficient optional-feature coverage".into());
        push_unique_flag(&mut flags, "calibrated-audio-insufficient-coverage");
    }

    let current_canonical = resolve_current_canonical(&evidence.current_genre);
    let action = compare_to_current(current_canonical, genre);

    let candidates = build_candidates(&votes, genre);

    if !evidence.has_discogs && !evidence.has_beatport {
        push_unique_flag(&mut flags, "no-enrichment");
    }
    if !evidence.has_audio {
        push_unique_flag(&mut flags, "no-audio");
    }
    add_missing_audio_flags(evidence, &mut flags);

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

fn compute_audio_profile(audio: &AudioFeatures) -> AudioProfile {
    let bpm = audio.rekordbox_bpm;
    let danceability = audio.danceability.filter(|value| value.is_finite());
    let dynamic_complexity = audio.dynamic_complexity.filter(|value| value.is_finite());
    let rhythm_regularity = audio.rhythm_regularity.filter(|value| value.is_finite());
    let bucket = danceability.map(|danceability| {
        if danceability < 1.0 {
            EnergyBucket::NonDancefloor
        } else if danceability < 1.5 {
            EnergyBucket::LowEnergy
        } else if danceability <= 2.5 {
            EnergyBucket::Dancefloor
        } else {
            EnergyBucket::HighEnergy
        }
    });

    let mut flags = Vec::new();
    if let Some(dc) = dynamic_complexity {
        if dc > 10.0 {
            flags.push(CharFlag::Ambient);
        }
        if dc > 5.0 {
            flags.push(CharFlag::Atmospheric);
        }
    }
    // Caveat: atmospheric + broken/irregular → lower confidence on rhythm flags
    // We still set them but the decision tree checks for this combination
    if let Some(rr) = rhythm_regularity {
        if rr < 0.5 {
            flags.push(CharFlag::Broken);
        } else if rr < 0.8 {
            flags.push(CharFlag::Irregular);
        }
    }
    if bpm > 155.0 {
        flags.push(CharFlag::Fast);
    }
    if bpm < 115.0 {
        flags.push(CharFlag::Slow);
    }
    // `key_confidence == 0.0` is stratum's sentinel for "key detection failed",
    // not atonal music — exclude it so analysis failures aren't relabelled.
    if audio.key_confidence.is_some_and(|kc| kc > 0.0 && kc < 0.1) {
        flags.push(CharFlag::Atonal);
    }
    if audio.decay_mid_tau.is_some_and(|t| t > LONG_TAIL_DECAY_MS) {
        flags.push(CharFlag::LongTail);
    }
    if audio
        .duration_seconds
        .is_some_and(|d| d > COMPRESSED_MIN_DURATION_SECONDS)
        && audio
            .loudness_range
            .is_some_and(|lr| lr < COMPRESSED_LOUDNESS_RANGE_LU)
    {
        flags.push(CharFlag::Compressed);
    }

    AudioProfile {
        bucket,
        flags,
        bpm,
        centroid: audio
            .spectral_centroid_mean
            .filter(|value| value.is_finite()),
        rhythm_regularity,
    }
}

fn compute_bpm_context(
    audio: Option<&AudioFeatures>,
    audio_profile: Option<&AudioProfile>,
    fallback_bpm: f64,
) -> BpmContext {
    let default_bpm = audio_profile.map_or(fallback_bpm, |p| p.bpm);
    if audio_profile
        .and_then(|profile| profile.bucket)
        .is_none_or(|bucket| bucket < EnergyBucket::Dancefloor)
    {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    }

    let Some(audio) = audio else {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    };

    let (Some(false), Some(stratum_bpm), Some(essentia_bpm)) = (
        audio.bpm_agreement,
        audio.stratum_bpm.filter(|bpm| *bpm > 0.0),
        audio.essentia_bpm.filter(|bpm| *bpm > 0.0),
    ) else {
        return BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        };
    };

    let detector_delta = relative_delta(stratum_bpm, essentia_bpm);
    let consensus_bpm = (stratum_bpm + essentia_bpm) / 2.0;
    let rekordbox_delta = relative_delta(default_bpm, consensus_bpm);
    if detector_delta < BPM_DETECTOR_CONSENSUS_TOLERANCE
        && rekordbox_delta > BPM_DETECTOR_CONSENSUS_TOLERANCE
        && !is_near_double_time(default_bpm, consensus_bpm)
    {
        BpmContext {
            effective_bpm: consensus_bpm,
            fallback: Some(BpmFallback {
                rekordbox_bpm: default_bpm,
                detector_bpm: consensus_bpm,
            }),
        }
    } else {
        BpmContext {
            effective_bpm: default_bpm,
            fallback: None,
        }
    }
}

fn relative_delta(a: f64, b: f64) -> f64 {
    if a <= 0.0 || b <= 0.0 {
        return f64::INFINITY;
    }
    (a - b).abs() / a.min(b)
}

fn is_near_double_time(a: f64, b: f64) -> bool {
    if a <= 0.0 || b <= 0.0 {
        return false;
    }
    let ratio = a.max(b) / a.min(b);
    ((ratio - 2.0).abs() / 2.0) < BPM_DETECTOR_CONSENSUS_TOLERANCE
}

fn has_flag(profile: &AudioProfile, flag: CharFlag) -> bool {
    profile.flags.contains(&flag)
}

fn push_unique_flag(flags: &mut Vec<String>, flag: &str) {
    if !flags.iter().any(|existing| existing == flag) {
        flags.push(flag.to_string());
    }
}

fn add_missing_audio_flags(evidence: &TrackEvidence, flags: &mut Vec<String>) {
    let Some(audio) = evidence.audio.as_ref() else {
        return;
    };
    if audio.danceability.is_none_or(|value| !value.is_finite()) {
        push_unique_flag(flags, "missing-danceability");
    }
    if audio
        .rhythm_regularity
        .is_none_or(|value| !value.is_finite())
    {
        push_unique_flag(flags, "missing-rhythm-regularity");
    }
}

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

    if profile.bucket == Some(EnergyBucket::NonDancefloor) && has_flag(profile, CharFlag::Ambient) {
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

    // Expanded ambient veto: NonDancefloor + Atmospheric (dc > 5.0) catches ambient
    // tracks below the dc > 10.0 Ambient flag threshold. Lower confidence than above.
    if profile.bucket == Some(EnergyBucket::NonDancefloor)
        && has_flag(profile, CharFlag::Atmospheric)
        && !has_flag(profile, CharFlag::Ambient)
        && !has_flag(profile, CharFlag::Compressed)
    // don't double-fire with the veto above
    {
        let action = compare_to_current(current_canonical, Some("Ambient"));
        return Some(veto_result(
            evidence,
            "Ambient",
            ClassificationConfidence::Low,
            action,
            vec!["audio veto: non-dancefloor + atmospheric → Ambient".into()],
            Some("Atmospheric non-dancefloor track — review genre assignment.".into()),
        ));
    }

    if profile.bucket == Some(EnergyBucket::NonDancefloor) && has_flag(profile, CharFlag::Slow) {
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

    if profile.bucket == Some(EnergyBucket::NonDancefloor) {
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

    // Bass veto only fires if enrichment agrees or is absent
    if has_flag(profile, CharFlag::Fast)
        && matches!(
            profile.bucket,
            Some(EnergyBucket::LowEnergy | EnergyBucket::Dancefloor | EnergyBucket::HighEnergy)
        )
    {
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

    if profile.bucket == Some(EnergyBucket::LowEnergy)
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

fn all_enrichment_dancefloor(evidence: &TrackEvidence) -> bool {
    let has_any = !evidence.discogs_mapped.is_empty() || evidence.beatport_genre.is_some();
    if !has_any {
        return false;
    }
    let all_dance = evidence.discogs_mapped.iter().all(|mg| {
        matches!(
            genre::genre_family(mg.genre),
            GenreFamily::House | GenreFamily::Techno | GenreFamily::Bass | GenreFamily::Hardcore
        )
    });
    let bp_dance = evidence.beatport_genre.is_none_or(|g| {
        matches!(
            genre::genre_family(g),
            GenreFamily::House | GenreFamily::Techno | GenreFamily::Bass | GenreFamily::Hardcore
        )
    });
    all_dance && bp_dance
}

fn find_family_genre(evidence: &TrackEvidence, family: GenreFamily) -> Option<&'static str> {
    if let Some(bp) = evidence.beatport_genre
        && genre::genre_family(bp) == family
    {
        return Some(bp);
    }
    evidence
        .discogs_mapped
        .iter()
        .filter(|mg| genre::genre_family(mg.genre) == family)
        .max_by_key(|mg| mg.style_count)
        .map(|mg| mg.genre)
}

fn gather_votes(
    evidence: &TrackEvidence,
    audio_profile: Option<&AudioProfile>,
    profile_registry: Option<&ProfileRegistry>,
    bpm_context: BpmContext,
) -> (Vec<GenreVote>, Vec<audio_profile::AudioAffinity>, bool) {
    let mut votes = Vec::new();
    let effective_bpm = bpm_context.effective_bpm;

    // Beatport: weight 1.0, halved if BPM-implausible
    if let Some(bp) = evidence.beatport_genre {
        let plausible = bpm_plausible(bp, effective_bpm);
        votes.push(GenreVote {
            genre: bp,
            weight: if plausible { 1.0 } else { 0.5 },
            source: "beatport",
            bpm_plausible: plausible,
        });
    }

    // Discogs: proportional to style_count with steeper decay for diverse releases.
    // Album-level data is less informative per-genre when spread across many genres.
    // Also discounted when track-level sources (Beatport) exist (confirmatory role).
    let total_styles: usize = evidence.discogs_mapped.iter().map(|m| m.style_count).sum();
    let n_mapped_genres = evidence.discogs_mapped.len();
    let diversity_decay = if n_mapped_genres <= 1 {
        1.0_f32
    } else {
        1.0 / (n_mapped_genres as f32).powf(0.4)
    };
    let confirmatory = if evidence.beatport_genre.is_some() {
        0.75_f32
    } else {
        1.0
    };
    for mg in &evidence.discogs_mapped {
        let proportion = if total_styles > 0 {
            (mg.style_count as f32 / total_styles as f32).min(1.0)
        } else {
            0.5
        };
        let base_weight = proportion * 0.9 * diversity_decay * confirmatory;
        let plausible = bpm_plausible(mg.genre, effective_bpm);
        votes.push(GenreVote {
            genre: mg.genre,
            weight: if plausible {
                base_weight
            } else {
                base_weight * 0.5
            },
            source: "discogs",
            bpm_plausible: plausible,
        });
    }

    // Label: weight 0.6, reduced to 0.4 when confirming to avoid double-counting
    if let Some(lg) = evidence.label_genre {
        let plausible = bpm_plausible(lg, effective_bpm);
        let confirms = votes.iter().any(|v| v.genre == lg);
        let weight = if confirms { 0.4 } else { 0.6 };
        votes.push(GenreVote {
            genre: lg,
            weight: if plausible { weight } else { weight * 0.5 },
            source: "label",
            bpm_plausible: plausible,
        });
    }

    // Current genre string: low-weight token-based evidence for non-canonical genres.
    // Weight is inversely proportional to the number of candidate genres matched,
    // capped at 0.5 total. Vague strings produce no votes.
    let tokens = genre::extract_genre_tokens(&evidence.current_genre);
    if !tokens.is_empty() {
        let n = tokens.len();
        let weight_per = (0.5 / n as f32).min(0.5);
        for g in tokens {
            let plausible = bpm_plausible(g, effective_bpm);
            votes.push(GenreVote {
                genre: g,
                weight: if plausible {
                    weight_per
                } else {
                    weight_per * 0.5
                },
                source: "current-genre",
                bpm_plausible: plausible,
            });
        }
    }

    // Audio-profile votes: Fisher discriminant scoring against calibrated prototypes.
    let (affinities, calibrated_coverage_missing) =
        if let (Some(audio), Some(registry)) = (&evidence.audio, profile_registry) {
            let scored = audio_profile::score_all(audio, registry);
            let aff = scored.affinities;
            for a in &aff {
                if a.vote_weight < 0.05 {
                    continue;
                }
                votes.push(GenreVote {
                    genre: a.genre,
                    weight: a.vote_weight,
                    source: "audio-profile",
                    bpm_plausible: bpm_plausible(a.genre, effective_bpm),
                });
            }
            (
                aff,
                !registry.prototypes.is_empty() && !scored.had_sufficient_coverage,
            )
        } else {
            (vec![], false)
        };

    if let Some(profile) = audio_profile
        && has_flag(profile, CharFlag::LongTail)
        && has_flag(profile, CharFlag::Atonal)
        && votes.iter().any(|v| v.genre == "Drone Techno")
    {
        votes.push(GenreVote {
            genre: "Drone Techno",
            weight: AUDIO_RULE_BOOST,
            source: "audio-long-tail-atonal",
            bpm_plausible: bpm_plausible("Drone Techno", effective_bpm),
        });
    }

    (votes, affinities, calibrated_coverage_missing)
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

fn find_consensus(
    evidence: &TrackEvidence,
    votes: &[GenreVote],
    audio_profile: Option<&AudioProfile>,
    bpm_context: BpmContext,
) -> (
    Option<&'static str>,
    ClassificationConfidence,
    Vec<String>,
    Vec<String>,
) {
    let mut tally: HashMap<&'static str, (f32, bool)> = HashMap::new();
    for v in votes {
        let entry = tally.entry(v.genre).or_insert((0.0, true));
        entry.0 += v.weight;
        if !v.bpm_plausible {
            entry.1 = false;
        }
    }

    let mut ranked: Vec<(&'static str, f32, bool)> =
        tally.into_iter().map(|(g, (w, p))| (g, w, p)).collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.2.cmp(&a.2)) // plausible before implausible
            .then_with(|| a.0.cmp(b.0)) // alphabetical
    });

    if ranked.is_empty() {
        return (None, ClassificationConfidence::Insufficient, vec![], vec![]);
    }

    let (mut top_genre, top_score, _) = ranked[0];
    let second = ranked.get(1);
    let margin = top_score - second.map_or(0.0, |s| s.1);
    let total_weight: f32 = ranked.iter().map(|(_, w, _)| w).sum();

    let mut ev = Vec::new();
    let mut flags = Vec::new();

    if let Some(fallback) = bpm_context.fallback {
        ev.push(format!(
            "bpm-fallback: rekordbox {:.1} → detector consensus {:.1}",
            fallback.rekordbox_bpm, fallback.detector_bpm
        ));
        flags.push("bpm-rekordbox-disagrees".into());
    }

    if !evidence.discogs_mapped.is_empty() {
        let parts: Vec<String> = evidence
            .discogs_mapped
            .iter()
            .map(|mg| {
                let bpm_note = if !bpm_plausible(mg.genre, bpm_context.effective_bpm) {
                    " [bpm-implausible]"
                } else {
                    ""
                };
                format!("{}(x{}){}", mg.genre, mg.style_count, bpm_note)
            })
            .collect();
        ev.push(format!("discogs: {}", parts.join(", ")));
    }

    if let Some(bp) = evidence.beatport_genre {
        let raw = evidence.beatport_raw.as_deref().unwrap_or(bp);
        let bpm_note = if !bpm_plausible(bp, bpm_context.effective_bpm) {
            " [bpm-implausible]"
        } else {
            ""
        };
        if raw != bp {
            ev.push(format!("beatport: {bp} (raw: {raw}){bpm_note}"));
        } else {
            ev.push(format!("beatport: {bp}{bpm_note}"));
        }
    }

    if let Some(lg) = evidence.label_genre {
        let label_name = evidence.label.as_deref().unwrap_or("?");
        ev.push(format!("label: {label_name} → {lg}"));
    }

    if votes.iter().any(|v| v.source == "audio-long-tail-atonal") {
        ev.push("audio rule: long-tail+atonal → Drone Techno".into());
    }

    if let Some(profile) = audio_profile.as_ref() {
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
            .map(|f| match f {
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

    // Log current-genre token evidence
    {
        let tokens = genre::extract_genre_tokens(&evidence.current_genre);
        if !tokens.is_empty() {
            ev.push(format!(
                "current-genre: \"{}\" → {}",
                evidence.current_genre,
                tokens.join(", ")
            ));
        }
    }

    let mut confidence = if ranked.len() == 1 && top_score >= 1.0 {
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
        let (second_genre, _, _) = second.expect("second exists when ranked.len() >= 2");
        let same_family = genre::genre_family(top_genre) == genre::genre_family(second_genre);

        if margin / total_weight > 0.4 {
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
            if same_family {
                top_genre = resolve_same_family_specificity(
                    top_genre,
                    second_genre,
                    audio_profile,
                    &mut ev,
                );
                ClassificationConfidence::Low
            } else {
                if let Some(profile) = audio_profile.as_ref() {
                    let top_favored = audio_clearly_favors_family(profile, top_genre);
                    let second_genre = second.expect("second exists").0;
                    let second_favored = audio_clearly_favors_family(profile, second_genre);
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
            }
        }
    } else {
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

    // Swap to a BPM-plausible alternative if the winner is implausible
    let effective_bpm = bpm_context.effective_bpm;
    if !bpm_plausible(final_genre, effective_bpm)
        && let Some((alt_genre, _, _)) = ranked
            .iter()
            .skip(1)
            .find(|(g, _, _)| bpm_plausible(g, effective_bpm))
    {
        ev.push(format!(
            "bpm-override: {} implausible at {}bpm → {}",
            final_genre, effective_bpm as i32, alt_genre
        ));
        flags.push("bpm-override".into());
        let same_family = genre::genre_family(final_genre) == genre::genre_family(alt_genre)
            && genre::genre_family(final_genre) != GenreFamily::Other;
        final_genre = alt_genre;
        // Downgrade: runner-up was elevated by BPM elimination, not evidence weight.
        // Same-family swaps floor at Medium — the family evidence is intact.
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

    // HighEnergy always demotes deep variants (e.g. Deep Techno → Techno).
    // Dancefloor demotes only when the shallower variant also has votes,
    // and not when the track is Atmospheric, Atonal, LongTail, or Compressed
    // — each signals a deeper read.
    if let Some(profile) = audio_profile.as_ref()
        && let Some(shallower) = shallower_alternative(final_genre)
    {
        let demote = match profile.bucket {
            Some(EnergyBucket::HighEnergy) => true,
            Some(EnergyBucket::Dancefloor) => {
                ranked.iter().any(|(g, _, _)| *g == shallower)
                    && !has_flag(profile, CharFlag::Atmospheric)
                    && !has_flag(profile, CharFlag::Atonal)
                    && !has_flag(profile, CharFlag::LongTail)
                    && !has_flag(profile, CharFlag::Compressed)
            }
            _ => false,
        };
        if demote {
            ev.push(format!(
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
    for mg in &evidence.discogs_mapped {
        if genre::genre_family(mg.genre) != primary_family && mg.style_count >= 2 {
            ev.push(format!(
                "influence: {} (discogs x{})",
                mg.genre, mg.style_count
            ));
        }
    }
    if let Some(bp) = evidence.beatport_genre
        && genre::genre_family(bp) != primary_family
    {
        ev.push(format!("influence: {bp} (beatport)"));
    }

    (Some(final_genre), confidence, ev, flags)
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

fn shallower_alternative(genre: &str) -> Option<&'static str> {
    match genre {
        "Deep Techno" | "Dub Techno" | "Ambient Techno" | "Drone Techno" => Some("Techno"),
        "Deep House" => Some("House"),
        _ => None,
    }
}

fn audio_clearly_favors_family(profile: &AudioProfile, candidate: &str) -> bool {
    let family = genre::genre_family(candidate);
    match family {
        GenreFamily::Downtempo => {
            let very_low_centroid = profile.centroid.is_some_and(|c| c < CENTROID_VERY_LOW);
            (profile.bucket == Some(EnergyBucket::LowEnergy)
                && (has_flag(profile, CharFlag::Atmospheric) || very_low_centroid))
                || (profile.bucket == Some(EnergyBucket::NonDancefloor) && very_low_centroid)
        }
        GenreFamily::Bass => {
            has_flag(profile, CharFlag::Fast)
                || (has_flag(profile, CharFlag::Broken)
                    && profile
                        .bucket
                        .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor))
        }
        GenreFamily::Techno => {
            let dark_timbre = profile.centroid.is_some_and(|c| c < CENTROID_DARK);
            let long_tail = has_flag(profile, CharFlag::LongTail);
            (profile
                .bucket
                .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 125.0)
                || (profile.bucket == Some(EnergyBucket::LowEnergy)
                    && profile.rhythm_regularity.is_some()
                    && !has_flag(profile, CharFlag::Broken)
                    && profile.bpm >= 118.0
                    && profile.bpm <= 132.0
                    && (dark_timbre || long_tail))
        }
        GenreFamily::House => {
            profile.bucket == Some(EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && !has_flag(profile, CharFlag::Atonal)
                && profile.bpm >= 118.0
                && profile.bpm <= 132.0
        }
        GenreFamily::Hardcore => {
            profile
                .bucket
                .is_some_and(|bucket| bucket >= EnergyBucket::Dancefloor)
                && profile.rhythm_regularity.is_some()
                && !has_flag(profile, CharFlag::Broken)
                && profile.bpm >= 138.0
        }
        _ => false,
    }
}

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
        .filter(|value| value.is_finite());
    let rr = evidence
        .audio
        .as_ref()
        .and_then(|a| a.rhythm_regularity)
        .filter(|value| value.is_finite());
    let sc = evidence
        .audio
        .as_ref()
        .and_then(|a| a.spectral_centroid_mean)
        .filter(|value| value.is_finite());

    let mut candidates: Vec<&'static str> = Vec::new();
    let mut ev = Vec::new();

    // D.1: Broad bucket
    match profile.bucket {
        None => {
            ev.push("D.1: energy evidence missing; no audio-only recommendation".into());
        }
        Some(EnergyBucket::NonDancefloor) => {
            if dc.is_some_and(|value| value > 10.0) {
                candidates.push("Ambient");
                ev.push("D.1: non-dancefloor + high dynamic complexity → Ambient".into());
            } else if dc.is_some_and(|value| value > 5.0) {
                candidates.extend_from_slice(&["Experimental", "Ambient"]);
                ev.push("D.1: non-dancefloor + moderate complexity → Experimental/Ambient".into());
            } else if dc.is_some() {
                candidates.extend_from_slice(&["Downtempo", "Experimental"]);
                ev.push("D.1: non-dancefloor + low complexity → Downtempo/Experimental".into());
            } else {
                ev.push(
                    "D.1: dynamic-complexity evidence missing; no non-dancefloor refinement".into(),
                );
            }
        }
        Some(EnergyBucket::LowEnergy) => {
            if profile.bpm > 145.0 {
                candidates.extend_from_slice(&["Jungle", "Breakbeat"]);
                ev.push(format!(
                    "D.1: low-energy but fast ({}bpm) → Jungle/Breakbeat",
                    profile.bpm as i32
                ));
            } else if dc.is_some_and(|value| value > 5.0) {
                candidates.extend_from_slice(&["Downtempo", "Ambient Techno"]);
                ev.push("D.1: low-energy + atmospheric → Downtempo/Ambient Techno".into());
            } else if dc.is_some() {
                candidates.extend_from_slice(&["Electro", "IDM"]);
                ev.push("D.1: low-energy + low complexity → Electro/IDM".into());
            } else {
                ev.push(
                    "D.1: dynamic-complexity evidence missing; no low-energy refinement".into(),
                );
            }
        }
        Some(EnergyBucket::Dancefloor | EnergyBucket::HighEnergy) => {
            // D.2: Subgenre by BPM x rhythm regularity
            let bpm = profile.bpm;
            let Some(rr) = rr else {
                ev.push("D.2: rhythm-regularity evidence missing; no dancefloor refinement".into());
                return finish_audio_only_result(evidence, profile, candidates, ev);
            };
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
            } else if (120.0..=135.0).contains(&bpm) && rr > 0.9 {
                candidates.extend_from_slice(&["Techno", "Tech House", "House"]);
                ev.push(format!(
                    "D.2: {}bpm + regular → Techno/Tech House/House",
                    bpm as i32
                ));
            } else if (118.0..=130.0).contains(&bpm) && rr >= 0.8 {
                candidates.extend_from_slice(&["House", "Deep House"]);
                ev.push(format!(
                    "D.2: {}bpm + moderate rhythm → House/Deep House",
                    bpm as i32
                ));
            } else if (120.0..=140.0).contains(&bpm) && rr < 0.8 {
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

    // D.3: Spectral centroid refinement.
    // Preference lists are provisional — refine once issue #19 has empirical data.
    if let Some(centroid) = sc
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
            candidates.sort_by_key(|g| preferred.iter().position(|p| p == g).unwrap_or(usize::MAX));
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
            candidates.sort_by_key(|g| preferred.iter().position(|p| p == g).unwrap_or(usize::MAX));
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
            ev.push(format!(
                "D.3: {} → {} over {}",
                centroid_hint, candidates[0], first_before
            ));
        } else {
            ev.push(format!("D.3: {centroid_hint} (confirms D.2 order)"));
        }
    }

    finish_audio_only_result(evidence, profile, candidates, ev)
}

fn finish_audio_only_result(
    evidence: &TrackEvidence,
    profile: &AudioProfile,
    candidates: Vec<&'static str>,
    ev: Vec<String>,
) -> (
    Option<&'static str>,
    ClassificationConfidence,
    Vec<String>,
    Vec<String>,
) {
    // D.4: Confidence
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

fn resolve_current_canonical(current_genre: &str) -> Option<&'static str> {
    if current_genre.is_empty() {
        return None;
    }
    genre::resolve_genre(current_genre)
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
        .map(|(g, (score, bpm))| GenreCandidate {
            genre: g,
            score,
            bpm_plausible: bpm,
            chosen: Some(g) == top_genre,
        })
        .collect();
    // Chosen first, then by score desc, then deterministic tiebreak
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

fn build_review_hint(evidence: &TrackEvidence, flags: &[String]) -> String {
    let mut hints = Vec::new();
    if !evidence.has_discogs && !evidence.has_beatport {
        hints.push("No enrichment data available");
    }
    if !evidence.has_audio {
        hints.push("No audio analysis available");
    }
    if flags.iter().any(|flag| flag == "missing-danceability") {
        hints.push("Audio danceability/energy evidence is missing");
    }
    if flags.iter().any(|flag| flag == "missing-rhythm-regularity") {
        hints.push("Audio rhythm-regularity evidence is missing");
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

    fn make_audio(bpm: f64, danceability: f64, dc: f64, rr: f64, centroid: f64) -> AudioFeatures {
        AudioFeatures {
            rekordbox_bpm: bpm,
            stratum_bpm: Some(bpm),
            bpm_agreement: Some(true),
            essentia_bpm: Some(bpm),
            duration_seconds: Some(300.0),
            danceability: Some(danceability),
            dynamic_complexity: Some(dc),
            rhythm_regularity: Some(rr),
            spectral_centroid_mean: Some(centroid),
            decay_mid_tau: None,
            decay_high_tau: None,
            onset_rate: None,
            loudness_integrated: None,
            loudness_range: None,
            spectral_centroid_cv: None,
            spectral_flux_mean: None,
            dissonance_mean: None,
            key_clarity: None,
            key_confidence: None,
            kick_pattern: None,
            kick_pattern_confidence: None,
            kick_kicks_per_bar: None,
            kick_onset_count: None,
            kick_rate_basis: None,
            kick_histogram: None,
            mfcc_mean: None,
            mfcc_std: None,
            spectral_contrast_mean: None,
        }
    }

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
    fn missing_danceability_does_not_create_energy_evidence_or_fast_bass_veto() {
        let mut ev = make_evidence("");
        let mut audio = make_audio(160.0, 2.0, 3.0, 0.92, 1800.0);
        audio.danceability = None;
        ev.audio = Some(audio);
        ev.has_audio = true;

        let result = classify_track(&ev);

        assert_eq!(result.genre, None);
        assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
        assert!(!result.flags.contains(&"audio-vetoed".to_string()));
        assert!(result.flags.contains(&"missing-danceability".to_string()));
        assert!(
            result
                .evidence
                .iter()
                .all(|line| !line.contains("dancefloor")),
            "missing danceability must not be formatted as an energy bucket: {:?}",
            result.evidence
        );
    }

    #[test]
    fn missing_rhythm_regularity_is_unknown_with_or_without_other_essentia_values() {
        let mut otherwise_complete = make_audio(128.0, 2.0, 3.0, 0.92, 1800.0);
        otherwise_complete.rhythm_regularity = None;
        let profile = compute_audio_profile(&otherwise_complete);
        assert!(!has_flag(&profile, CharFlag::Broken));
        assert!(!has_flag(&profile, CharFlag::Irregular));

        let mut no_essentia = make_audio(128.0, 2.0, 3.0, 0.92, 1800.0);
        no_essentia.danceability = None;
        no_essentia.dynamic_complexity = None;
        no_essentia.rhythm_regularity = None;
        no_essentia.spectral_centroid_mean = None;
        let profile = compute_audio_profile(&no_essentia);
        assert!(!has_flag(&profile, CharFlag::Broken));
        assert!(!has_flag(&profile, CharFlag::Irregular));
    }

    #[test]
    fn bpm_only_audio_is_insufficient_and_has_stable_missing_evidence_flags() {
        let mut ev = make_evidence("");
        let mut audio = make_audio(160.0, 2.0, 3.0, 0.92, 1800.0);
        audio.danceability = None;
        audio.dynamic_complexity = None;
        audio.rhythm_regularity = None;
        audio.spectral_centroid_mean = None;
        ev.audio = Some(audio);
        ev.has_audio = true;

        let result = classify_track(&ev);

        assert_eq!(result.genre, None);
        assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
        assert_eq!(
            result.flags,
            vec![
                "audio-only".to_string(),
                "no-enrichment".to_string(),
                "missing-danceability".to_string(),
                "missing-rhythm-regularity".to_string(),
            ]
        );
    }

    #[test]
    fn complete_fast_dancefloor_audio_keeps_representative_bass_veto() {
        let mut ev = make_evidence("");
        ev.audio = Some(make_audio(160.0, 2.0, 3.0, 0.92, 1800.0));
        ev.has_audio = true;

        let result = classify_track(&ev);

        assert_eq!(result.genre, Some("Breakbeat"));
        assert_eq!(result.confidence, ClassificationConfidence::Medium);
        assert!(result.flags.contains(&"audio-vetoed".to_string()));
    }

    #[test]
    fn enrichment_confidence_is_preserved_when_optional_audio_is_missing() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Techno",
            style_count: 3,
        }];
        ev.has_discogs = true;
        let mut audio = make_audio(132.0, 2.0, 3.0, 0.92, 1800.0);
        audio.danceability = None;
        audio.dynamic_complexity = None;
        audio.rhythm_regularity = None;
        audio.spectral_centroid_mean = None;
        ev.audio = Some(audio);
        ev.has_audio = true;

        let result = classify_track(&ev);

        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
        assert!(result.flags.contains(&"missing-danceability".to_string()));
        assert!(
            result
                .flags
                .contains(&"missing-rhythm-regularity".to_string())
        );
    }

    #[test]
    fn beatport_only_returns_medium() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
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
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
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
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
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
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
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
        ev.audio = Some(make_audio(140.0, 2.0, 3.0, 0.92, 1800.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep House"));
        assert!(matches!(
            result.confidence,
            ClassificationConfidence::Medium | ClassificationConfidence::Low
        ));
    }

    #[test]
    fn audio_veto_ambient() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio(100.0, 0.5, 12.0, 0.3, 400.0));
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
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
    }

    #[test]
    fn depth_prefers_shallower_when_high_energy() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio(135.0, 2.8, 2.0, 0.95, 2500.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert!(result.evidence.iter().any(|e| e.contains("depth:")));
    }

    fn make_audio_with_key_conf(
        bpm: f64,
        danceability: f64,
        dc: f64,
        rr: f64,
        centroid: f64,
        key_conf: f64,
    ) -> AudioFeatures {
        let mut a = make_audio(bpm, danceability, dc, rr, centroid);
        a.key_confidence = Some(key_conf);
        a
    }

    fn make_audio_with_decay(
        bpm: f64,
        danceability: f64,
        dc: f64,
        rr: f64,
        centroid: f64,
        decay_mid_tau: f64,
    ) -> AudioFeatures {
        let mut a = make_audio(bpm, danceability, dc, rr, centroid);
        a.decay_mid_tau = Some(decay_mid_tau);
        a
    }

    fn make_audio_with_key_conf_and_decay(
        bpm: f64,
        danceability: f64,
        dc: f64,
        rr: f64,
        centroid: f64,
        key_conf: f64,
        decay_mid_tau: f64,
    ) -> AudioFeatures {
        let mut a = make_audio_with_key_conf(bpm, danceability, dc, rr, centroid, key_conf);
        a.decay_mid_tau = Some(decay_mid_tau);
        a
    }

    fn make_audio_with_loudness_range(
        bpm: f64,
        danceability: f64,
        dc: f64,
        rr: f64,
        centroid: f64,
        loudness_range: f64,
    ) -> AudioFeatures {
        let mut a = make_audio(bpm, danceability, dc, rr, centroid);
        a.loudness_range = Some(loudness_range);
        a
    }

    fn make_audio_with_detector_bpms(
        rekordbox_bpm: f64,
        stratum_bpm: f64,
        essentia_bpm: f64,
    ) -> AudioFeatures {
        let mut a = make_audio(rekordbox_bpm, 2.0, 3.0, 0.92, 1800.0);
        a.stratum_bpm = Some(stratum_bpm);
        a.bpm_agreement = Some((stratum_bpm - rekordbox_bpm).abs() <= 2.0);
        a.essentia_bpm = Some(essentia_bpm);
        a
    }

    #[test]
    fn atonal_techno_prefers_deep_techno() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        // Dancefloor (not high-energy), bright centroid, atonal (key_conf=0.05).
        ev.audio = Some(make_audio_with_key_conf(
            125.0, 2.0, 3.0, 0.92, 1800.0, 0.05,
        ));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep Techno"));
        assert!(
            result.evidence.iter().any(|e| e.contains("atonal")),
            "evidence should mention atonal: {:?}",
            result.evidence
        );
    }

    #[test]
    fn atonal_house_demotes_to_house() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep House",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("House");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_key_conf(
            124.0, 2.0, 3.0, 0.92, 1800.0, 0.05,
        ));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("House"));
        assert!(
            result.evidence.iter().any(|e| e.contains("atonal")),
            "evidence should mention atonal: {:?}",
            result.evidence
        );
    }

    // Energy demotion runs after same-family resolution, so HighEnergy overrides Atonal.
    #[test]
    fn high_energy_atonal_still_demotes_deep_techno() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        // HighEnergy bucket (danceability > 2.5) + atonal.
        ev.audio = Some(make_audio_with_key_conf(
            135.0, 2.8, 2.0, 0.95, 2500.0, 0.05,
        ));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
    }

    // `key_confidence == 0.0` is stratum's sentinel for detection failure,
    // not atonal music — must not flip the resolver toward Deep House.
    #[test]
    fn key_confidence_zero_sentinel_does_not_set_atonal() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep House",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("House");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_key_conf(124.0, 2.0, 3.0, 0.92, 1800.0, 0.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert!(
            !result.evidence.iter().any(|e| e.contains("atonal")),
            "key_confidence=0.0 must not set Atonal flag: {:?}",
            result.evidence
        );
    }

    #[test]
    fn long_tail_techno_prefers_deep_techno() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_decay(125.0, 2.0, 3.0, 0.92, 1800.0, 250.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep Techno"));
        assert!(
            result.evidence.iter().any(|e| e.contains("long-tail")),
            "evidence should mention long-tail: {:?}",
            result.evidence
        );
    }

    #[test]
    fn long_tail_atonal_boosts_drone_techno_candidate() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Drone Techno",
            style_count: 1,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Ambient");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_key_conf_and_decay(
            126.0, 2.0, 3.0, 0.92, 1000.0, 0.05, 275.0,
        ));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Drone Techno"));
        assert!(
            result
                .evidence
                .iter()
                .any(|e| e.contains("long-tail+atonal")),
            "evidence should mention conjunctive boost: {:?}",
            result.evidence
        );
    }

    #[test]
    fn long_tail_low_energy_techno_wins_cross_family_tiebreak() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![
            MappedGenre {
                genre: "Ambient",
                style_count: 1,
            },
            MappedGenre {
                genre: "Minimal",
                style_count: 1,
            },
        ];
        ev.has_discogs = true;
        ev.audio = Some(make_audio_with_decay(125.0, 1.2, 3.0, 0.95, 1500.0, 250.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Minimal"));
        assert!(
            result
                .flags
                .contains(&"audio-assisted-tiebreak".to_string()),
            "expected audio tiebreak flag, got flags={:?} evidence={:?}",
            result.flags,
            result.evidence
        );
    }

    #[test]
    fn high_energy_long_tail_still_demotes_deep_techno() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_decay(135.0, 2.8, 2.0, 0.95, 2500.0, 260.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
    }

    #[test]
    fn compressed_dancefloor_prefers_deep_techno() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio_with_loudness_range(
            128.0, 2.0, 3.0, 0.92, 1800.0, 0.7,
        ));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep Techno"));
        assert!(
            result.evidence.iter().any(|e| e.contains("compressed")),
            "evidence should mention compressed: {:?}",
            result.evidence
        );
    }

    #[test]
    fn compressed_atmospheric_skips_expanded_ambient_veto() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Dub Techno");
        ev.has_beatport = true;
        // NonDancefloor + Atmospheric normally trips the expanded Ambient veto.
        let audio = make_audio_with_loudness_range(120.0, 0.8, 7.0, 0.85, 900.0, 0.6);
        let profile = compute_audio_profile(&audio);
        assert!(has_flag(&profile, CharFlag::Compressed));
        ev.audio = Some(audio);
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_ne!(result.genre, Some("Ambient"));
        assert!(
            !result
                .evidence
                .iter()
                .any(|e| e.contains("non-dancefloor + atmospheric")),
            "compressed should suppress expanded Ambient veto: {:?}",
            result.evidence
        );
    }

    #[test]
    fn compressed_flag_ignores_short_tracks() {
        let mut ev = make_evidence("");
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 2,
        }];
        ev.has_discogs = true;
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        let mut audio = make_audio_with_loudness_range(128.0, 2.0, 3.0, 0.92, 1800.0, 0.7);
        audio.duration_seconds = Some(45.0);
        ev.audio = Some(audio);
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert!(
            !result.evidence.iter().any(|e| e.contains("compressed")),
            "short tracks should not set compressed: {:?}",
            result.evidence
        );
    }

    #[test]
    fn bpm_disagreement_uses_detector_consensus() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Deep House");
        ev.has_beatport = true;
        // Rekordbox 132 is just outside Deep House plausibility after tolerance;
        // Stratum + Essentia agree that the track is really around 125 BPM.
        ev.audio = Some(make_audio_with_detector_bpms(132.0, 125.0, 125.2));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep House"));
        assert_eq!(result.confidence, ClassificationConfidence::High);
        assert!(
            result
                .flags
                .contains(&"bpm-rekordbox-disagrees".to_string()),
            "expected BPM disagreement flag: {:?}",
            result.flags
        );
        assert!(
            result.evidence.iter().any(|e| e.contains("bpm-fallback")),
            "expected fallback evidence: {:?}",
            result.evidence
        );
        assert!(
            !result
                .evidence
                .iter()
                .any(|e| e.contains("bpm-implausible")),
            "fallback should make Deep House BPM-plausible: {:?}",
            result.evidence
        );
    }

    #[test]
    fn bpm_disagreement_no_detector_consensus_uses_rekordbox() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Deep House");
        ev.has_beatport = true;
        // Stratum disagrees with Rekordbox, but Essentia does not agree with
        // Stratum, so keep Rekordbox for plausibility.
        ev.audio = Some(make_audio_with_detector_bpms(132.0, 125.0, 132.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep House"));
        assert!(
            !result
                .flags
                .contains(&"bpm-rekordbox-disagrees".to_string()),
            "detector disagreement should not set fallback flag: {:?}",
            result.flags
        );
        assert!(
            result
                .evidence
                .iter()
                .any(|e| e.contains("bpm-implausible")),
            "Rekordbox BPM should remain in use: {:?}",
            result.evidence
        );
    }

    #[test]
    fn bpm_disagreement_rejects_double_time_consensus() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Ambient");
        ev.has_beatport = true;
        // Both detectors agree near double Rekordbox tempo. That is common for
        // half-time material and should stay reviewable, not override Rekordbox.
        ev.audio = Some(make_audio_with_detector_bpms(74.0, 148.0, 147.8));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Ambient"));
        assert!(
            !result
                .flags
                .contains(&"bpm-rekordbox-disagrees".to_string()),
            "double-time consensus should not set fallback flag: {:?}",
            result.flags
        );
    }

    #[test]
    fn bpm_disagreement_requires_dancefloor_audio() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Deep House");
        ev.has_beatport = true;
        let mut audio = make_audio_with_detector_bpms(90.0, 122.0, 122.2);
        audio.danceability = Some(1.2);
        ev.audio = Some(audio);
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Deep House"));
        assert!(
            !result
                .flags
                .contains(&"bpm-rekordbox-disagrees".to_string()),
            "low-energy audio should not use detector BPM fallback: {:?}",
            result.flags
        );
    }

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

    // 2. Kassian - Actions: BPM-implausible Deep House (132 > range max 131)
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
            audio: Some(make_audio(156.97, 1.01, 3.95, 1.04, 1395.0)),
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

    // 6. Alarico - 0 Kelvin: Deep Techno BPM-implausible at 145, Techno barely plausible
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
            audio: Some(make_audio(134.31, 1.74, 3.64, 1.07, 1244.0)),
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
            "4-way even split should be Low or Insufficient, got {:?}",
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

    // 11. Daniel Stefanik - #four: BPM 119 makes Techno implausible, Tech House plausible
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
            audio: Some(make_audio(130.0, 1.15, 3.35, 1.16, 1585.0)),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Breakbeat"));
        assert_eq!(result.action, ClassificationAction::Confirm);
    }

    // 13. Flying Lotus - ...And The World Laughs With You: enrichment overrides bass veto
    // Enrichment says Downtempo/IDM, not bass — veto should not fire despite 165bpm.
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
            beatport_genre: None, // "Electronica" unmapped
            beatport_raw: Some("Electronica".into()),
            label: None,
            label_genre: None,
            audio: Some(make_audio(164.7, 1.19, 4.18, 0.68, 1919.0)),
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
            audio: Some(make_audio(104.02, 1.36, 7.02, 0.97, 1161.0)),
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
            audio: Some(make_audio(128.0, 1.42, 1.76, 0.84, 1923.0)),
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
            audio: Some(make_audio(154.0, 1.36, 3.47, 1.10, 1341.0)),
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
            audio: Some(make_audio(136.0, 1.69, 6.62, 0.95, 2205.0)),
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
            audio: Some(make_audio(132.0, 3.62, 2.71, 1.00, 1572.0)),
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
            audio: Some(make_audio(122.0, 2.06, 4.30, 0.92, 601.0)),
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

    // H5: Cross-family conflict (Discogs House vs Beatport Techno) resolved by audio tiebreaker
    #[test]
    fn test_conflicting_enrichment_resolved_by_audio() {
        let ev = TrackEvidence {
            track_id: "test-h5".into(),
            artist: "Test Artist".into(),
            title: "Conflict Tiebreak".into(),
            current_genre: "".into(),
            bpm: 130.0,
            discogs_mapped: vec![MappedGenre {
                genre: "House",
                style_count: 1,
            }],
            beatport_genre: Some("Techno"),
            beatport_raw: Some("Techno (Peak Time / Driving)".into()),
            label: None,
            label_genre: None,
            audio: Some(make_audio(130.0, 2.2, 3.0, 0.92, 1800.0)),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("Techno"),
            "Beatport (track-level) should win over Discogs (album-level) for Techno"
        );
        // With confirmatory discount, Discogs weight is reduced when Beatport
        // exists, so the margin exceeds the 15% tight-race threshold → Medium.
        assert_eq!(
            result.confidence,
            ClassificationConfidence::Medium,
            "Track-level source should produce Medium confidence, not a tight race"
        );
    }

    // BPM override within the same genre family should floor confidence at Medium,
    // not downgrade Medium → Low.
    #[test]
    fn bpm_override_same_family_floors_at_medium() {
        // Beatport + Discogs both say "Deep Techno" (strong signal), BPM is 145 —
        // outside Deep Techno range (120-132+5). Label says "Techno" which IS
        // plausible at 145 BPM (128-140+5). Audio is dancefloor + atmospheric so
        // depth resolution prefers deeper (Deep Techno), then BPM override swaps
        // to Techno. Same Techno family → confidence floors at Medium.
        let ev = TrackEvidence {
            track_id: "test-bpm-same-family".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 145.0,
            discogs_mapped: vec![MappedGenre {
                genre: "Deep Techno",
                style_count: 2,
            }],
            beatport_genre: Some("Deep Techno"),
            beatport_raw: Some("Techno (Raw / Deep / Hypnotic)".into()),
            label: Some("Test Label".into()),
            label_genre: Some("Techno"),
            audio: Some(make_audio(145.0, 1.6, 6.0, 0.92, 800.0)),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Techno"));
        assert_eq!(result.confidence, ClassificationConfidence::Medium);
        assert!(
            result.flags.contains(&"bpm-override".to_string()),
            "Expected bpm-override flag, got: {:?}",
            result.flags
        );
        assert!(
            result
                .flags
                .contains(&"bpm-override-same-family".to_string()),
            "Expected bpm-override-same-family flag, got: {:?}",
            result.flags
        );
    }

    // BPM override across genre families should still downgrade Medium → Low.
    #[test]
    fn bpm_override_cross_family_downgrades_to_low() {
        // Beatport + Discogs both say "House" (strong signal), BPM is 170 —
        // outside House range (120-130+5). Label says "Drum & Bass" which IS
        // plausible at 170 BPM (168-180). Different families (House vs Bass) →
        // confidence downgrades Medium → Low.
        let ev = TrackEvidence {
            track_id: "test-bpm-cross-family".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 170.0,
            discogs_mapped: vec![MappedGenre {
                genre: "House",
                style_count: 2,
            }],
            beatport_genre: Some("House"),
            beatport_raw: Some("House".into()),
            label: Some("Test Label".into()),
            label_genre: Some("Drum & Bass"),
            audio: Some(make_audio(170.0, 2.0, 3.0, 0.85, 2000.0)),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Drum & Bass"));
        assert_eq!(result.confidence, ClassificationConfidence::Low);
        assert!(
            result.flags.contains(&"bpm-override".to_string()),
            "Expected bpm-override flag, got: {:?}",
            result.flags
        );
        assert!(
            !result
                .flags
                .contains(&"bpm-override-same-family".to_string()),
            "Should NOT have same-family flag for cross-family override"
        );
    }

    // --- Tests for new behaviors from the classification improvements ---

    #[test]
    fn expanded_ambient_veto_atmospheric_not_ambient() {
        // dc=7.0 → Atmospheric (>5.0) but NOT Ambient (<10.0)
        // danceability=0.5 → NonDancefloor
        // Should trigger the new NonDancefloor + Atmospheric veto
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.audio = Some(make_audio(100.0, 0.5, 7.0, 0.3, 400.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert_eq!(result.genre, Some("Ambient"));
        assert_eq!(result.confidence, ClassificationConfidence::Low);
        assert!(result.flags.contains(&"audio-vetoed".to_string()));
        assert!(
            result.evidence.iter().any(|e| e.contains("atmospheric")),
            "Should mention atmospheric in evidence"
        );
    }

    #[test]
    fn both_candidates_tiebreak_favors_downtempo() {
        // Cross-family close race: Disco vs Downtempo at BPM 100.
        // BPM 100 is plausible for Downtempo (80-115) but implausible for Disco (115-130).
        // So Disco's weight is halved → not quite a tie. Use label to bring Disco closer.
        // Audio: LowEnergy + Atmospheric → Downtempo passes.
        // Disco family (Other) → audio_clearly_favors_family returns false.
        let ev = TrackEvidence {
            track_id: "test-swap".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 100.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Disco",
                    style_count: 2,
                },
                MappedGenre {
                    genre: "Downtempo",
                    style_count: 2,
                },
            ],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            // LowEnergy (1.2) + Atmospheric (dc=6.0) → Downtempo passes
            audio: Some(make_audio(100.0, 1.2, 6.0, 0.85, 1500.0)),
            has_discogs: true,
            has_beatport: false,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.genre,
            Some("Downtempo"),
            "Audio should favor Downtempo. Got {:?}, flags: {:?}, evidence: {:?}",
            result.genre,
            result.flags,
            result.evidence
        );
    }

    #[test]
    fn both_candidates_tiebreak_insufficient_when_both_pass() {
        // Cross-family close race where BOTH families pass audio check → Insufficient
        // LowEnergy + centroid < 600 → Downtempo passes (very_low_centroid)
        // LowEnergy + bpm 118-132 + centroid < 1200 → Techno also passes (dark timbre)
        let ev = TrackEvidence {
            track_id: "test-both".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 125.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Ambient",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Minimal",
                    style_count: 1,
                },
            ],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            // LowEnergy + very low centroid → both Downtempo and Techno pass
            audio: Some(make_audio(125.0, 1.2, 3.0, 0.95, 264.0)),
            has_discogs: true,
            has_beatport: false,
            has_audio: true,
        };
        let result = classify_track(&ev);
        assert_eq!(
            result.confidence,
            ClassificationConfidence::Insufficient,
            "Both families pass audio check → should be Insufficient"
        );
    }

    #[test]
    fn deterministic_tiebreak_across_runs() {
        // Two genres with identical scores → result must be deterministic
        let ev = TrackEvidence {
            track_id: "test-det".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 125.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Ambient",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Minimal",
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
        let first_result = classify_track(&ev);
        for _ in 0..10 {
            let result = classify_track(&ev);
            assert_eq!(
                result.genre, first_result.genre,
                "Genre should be deterministic across runs"
            );
        }
    }

    #[test]
    fn candidates_include_chosen_genre() {
        let mut ev = make_evidence("");
        ev.beatport_genre = Some("Techno");
        ev.has_beatport = true;
        ev.discogs_mapped = vec![MappedGenre {
            genre: "Deep Techno",
            style_count: 1,
        }];
        ev.has_discogs = true;
        ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
        ev.has_audio = true;
        let result = classify_track(&ev);
        assert!(
            result.candidates.iter().any(|c| c.chosen),
            "Should have a chosen candidate"
        );
        let chosen = result.candidates.iter().find(|c| c.chosen).unwrap();
        assert_eq!(chosen.genre, result.genre.unwrap());
    }

    #[test]
    fn profile_votes_influence_consensus() {
        use crate::audio_profile;

        // Build a registry with Dub Techno prototype: low BPM, low centroid, moderate danceability
        let dub_techno_tracks: Vec<AudioFeatures> = (0..8)
            .map(|i| {
                let mut a = make_audio(122.0 + i as f64 * 0.5, 2.3, 4.5, 0.96, 550.0);
                a.onset_rate = Some(4.2);
                a.loudness_integrated = Some(-9.0);
                a
            })
            .collect();
        let ambient_tracks: Vec<AudioFeatures> = (0..8)
            .map(|i| {
                let mut a = make_audio(85.0 + i as f64, 0.8, 7.0, 0.5, 350.0);
                a.onset_rate = Some(1.5);
                a.loudness_integrated = Some(-12.0);
                a
            })
            .collect();

        let samples: Vec<(&str, &AudioFeatures)> = dub_techno_tracks
            .iter()
            .map(|a| ("Dub Techno", a))
            .chain(ambient_tracks.iter().map(|a| ("Ambient", a)))
            .collect();
        let registry = audio_profile::calibrate(&samples);

        // Evidence: noisy Discogs (Ambient vs Minimal tie) but audio is dub-techno-like
        let ev = TrackEvidence {
            track_id: "test-profile".into(),
            artist: "Test".into(),
            title: "Test".into(),
            current_genre: "".into(),
            bpm: 124.0,
            discogs_mapped: vec![
                MappedGenre {
                    genre: "Ambient",
                    style_count: 1,
                },
                MappedGenre {
                    genre: "Minimal",
                    style_count: 1,
                },
            ],
            beatport_genre: None,
            beatport_raw: None,
            label: None,
            label_genre: None,
            audio: Some({
                let mut a = make_audio(124.0, 2.4, 4.0, 0.97, 520.0);
                a.onset_rate = Some(4.3);
                a.loudness_integrated = Some(-9.5);
                a
            }),
            has_discogs: true,
            has_beatport: true,
            has_audio: true,
        };

        // Without profiles: Ambient or Minimal wins
        let result_no_profile = classify_track(&ev);

        // With profiles: Dub Techno should get a vote and influence the result
        let result_with_profile = classify_track_with_profiles(&ev, Some(&registry));

        // The profile should inject a Dub Techno vote
        assert!(
            result_with_profile
                .evidence
                .iter()
                .any(|e| e.contains("audio-profile")),
            "Should have audio-profile evidence string. Evidence: {:?}",
            result_with_profile.evidence
        );

        // With profile, result should differ from no-profile (Dub Techno vote changes things)
        assert_ne!(
            result_no_profile.genre, result_with_profile.genre,
            "Profile votes should influence the consensus. Without: {:?}, With: {:?}",
            result_no_profile.genre, result_with_profile.genre
        );
    }

    #[test]
    fn sparse_audio_with_registry_reports_calibrated_coverage_gap() {
        use crate::audio_profile;

        let house_tracks: Vec<AudioFeatures> = (0..8)
            .map(|_| make_audio(128.0, 2.0, 3.0, 0.9, 1500.0))
            .collect();
        let ambient_tracks: Vec<AudioFeatures> = (0..8)
            .map(|_| make_audio(128.0, 0.5, 3.0, 0.9, 1500.0))
            .collect();
        let samples: Vec<(&str, &AudioFeatures)> = house_tracks
            .iter()
            .map(|audio| ("House", audio))
            .chain(ambient_tracks.iter().map(|audio| ("Ambient", audio)))
            .collect();
        let registry = audio_profile::calibrate(&samples);

        let mut ev = make_evidence("");
        ev.beatport_genre = Some("House");
        ev.has_beatport = true;
        let mut sparse = make_audio(128.0, 2.0, 3.0, 0.9, 1500.0);
        sparse.danceability = None;
        ev.audio = Some(sparse);
        ev.has_audio = true;

        let result = classify_track_with_profiles(&ev, Some(&registry));

        assert_eq!(result.genre, Some("House"));
        assert!(
            result
                .flags
                .contains(&"calibrated-audio-insufficient-coverage".to_string())
        );
        assert!(
            result
                .evidence
                .iter()
                .any(|line| line == "audio-profile: insufficient optional-feature coverage")
        );
    }
}
