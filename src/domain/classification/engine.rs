//! Server-side genre decision tree.
//!
//! Applies evidence-based genre classification using cached Discogs and label
//! metadata plus audio analysis features (Essentia).

mod audio;
mod audio_only;
mod consensus;
mod votes;

use super::profiles::{self as audio_profile, ProfileRegistry};
use super::taxonomy::{self as genre, GenreFamily};
#[cfg(test)]
use super::{AudioBackendStatus, AudioFeatures, LabelProvenance, MappedGenre};
use super::{
    ClassificationAction, ClassificationConfidence, ClassificationDegradedReason,
    ClassificationMode, ClassificationResult, DiscogsMatchQuality, DiscogsReadiness, TrackEvidence,
    classification_readiness,
};
#[cfg(test)]
use audio::profile as compute_audio_profile;
use audio::{AudioProfile, CharFlag, EnergyBucket, has_flag};

struct ClassificationDecision {
    genre: Option<&'static str>,
    confidence: ClassificationConfidence,
    evidence: Vec<String>,
    flags: Vec<String>,
}

impl ClassificationDecision {
    fn empty() -> Self {
        Self {
            genre: None,
            confidence: ClassificationConfidence::Insufficient,
            evidence: Vec::new(),
            flags: Vec::new(),
        }
    }
}

#[cfg(test)]
pub(crate) fn classify_track(evidence: &TrackEvidence) -> ClassificationResult {
    classify_track_with_profiles(evidence, None)
}

pub(crate) fn classify_track_with_profiles(
    evidence: &TrackEvidence,
    profile_registry: Option<&ProfileRegistry>,
) -> ClassificationResult {
    let audio_profile = evidence.audio.as_ref().map(audio::profile);
    let bpm_context = audio::bpm_context(
        evidence.audio.as_ref(),
        audio_profile.as_ref(),
        evidence.bpm,
    );

    if let Some(profile) = audio_profile.as_ref()
        && let Some(mut result) = check_audio_vetoes(evidence, profile)
    {
        add_missing_audio_flags(evidence, &mut result.flags);
        return finalize_classification_result(evidence, result);
    }

    let vote_collection = votes::gather(evidence, profile_registry, bpm_context);

    let mut decision = if vote_collection.votes.is_empty() {
        audio_only::resolve(evidence, audio_profile.as_ref())
    } else {
        consensus::resolve(
            evidence,
            &vote_collection.votes,
            audio_profile.as_ref(),
            bpm_context,
        )
    };

    if decision.genre.is_none() {
        let current_tokens = current_genre_tokens(&evidence.current_genre);
        if current_tokens.len() == 1 {
            decision.genre = Some(current_tokens[0]);
            decision.confidence = ClassificationConfidence::Low;
            decision.evidence.push(format!(
                "current-genre hint: \"{}\" → {} (no independent recommendation)",
                evidence.current_genre, current_tokens[0]
            ));
            push_unique_flag(&mut decision.flags, "current-genre-only");
        } else if current_tokens.len() > 1 {
            push_unique_flag(&mut decision.flags, "current-genre-ambiguous");
        }
    }

    // Add audio-profile evidence strings for affinities that contributed
    for affinity in &vote_collection.affinities {
        if affinity.vote_weight >= 0.1 {
            decision
                .evidence
                .push(audio_profile::format_evidence(affinity));
        }
    }
    if vote_collection.calibrated_coverage_missing {
        decision
            .evidence
            .push("audio-profile: insufficient optional-feature coverage".into());
        push_unique_flag(
            &mut decision.flags,
            "calibrated-audio-insufficient-coverage",
        );
    }

    let current_canonical = resolve_current_canonical(&evidence.current_genre);
    let action = compare_to_current(current_canonical, decision.genre);

    let candidates = votes::candidates(&vote_collection.votes, decision.genre);

    match evidence.discogs_readiness() {
        DiscogsReadiness::UsableGenre => {}
        DiscogsReadiness::NotSearched => {
            push_unique_flag(&mut decision.flags, "no-enrichment");
        }
        DiscogsReadiness::NoMatch => {
            push_unique_flag(&mut decision.flags, "no-enrichment");
        }
        DiscogsReadiness::MatchedUnmapped => {
            push_unique_flag(&mut decision.flags, "no-enrichment");
            push_unique_flag(&mut decision.flags, "discogs-matched-unmapped");
        }
    }
    if evidence.discogs_match_quality == Some(DiscogsMatchQuality::Invalid) {
        push_unique_flag(&mut decision.flags, "discogs-match-invalid");
    }
    if !evidence.has_audio {
        push_unique_flag(&mut decision.flags, "no-audio");
    }
    add_missing_audio_flags(evidence, &mut decision.flags);

    let review_hint = match decision.confidence {
        ClassificationConfidence::Low | ClassificationConfidence::Insufficient => {
            Some(build_review_hint(evidence, &decision.flags))
        }
        _ => None,
    };

    finalize_classification_result(
        evidence,
        ClassificationResult {
            track_id: evidence.track_id.clone(),
            artist: evidence.artist.clone(),
            title: evidence.title.clone(),
            current_genre: evidence.current_genre.clone(),
            genre: decision.genre,
            confidence: decision.confidence,
            action,
            mode: ClassificationMode::Full,
            degraded_reasons: Vec::new(),
            evidence: decision.evidence,
            candidates,
            flags: decision.flags,
            review_hint,
        },
    )
}

fn finalize_classification_result(
    evidence: &TrackEvidence,
    mut result: ClassificationResult,
) -> ClassificationResult {
    let (mode, reasons) =
        classification_readiness(evidence.stratum_status, evidence.essentia_status);
    result.mode = mode;
    result.degraded_reasons = reasons;

    if mode == ClassificationMode::Degraded {
        if matches!(
            result.confidence,
            ClassificationConfidence::High | ClassificationConfidence::Medium
        ) {
            result.confidence = ClassificationConfidence::Low;
        }
        push_unique_flag(&mut result.flags, "degraded-classification");
        let degraded_hint = degraded_review_hint(&result.degraded_reasons);
        result.review_hint = Some(match result.review_hint.take() {
            Some(existing) if !existing.is_empty() => format!("{degraded_hint} {existing}"),
            _ => degraded_hint,
        });
    }

    result
}

fn degraded_review_hint(reasons: &[ClassificationDegradedReason]) -> String {
    let labels: Vec<&str> = reasons
        .iter()
        .map(|reason| match reason {
            ClassificationDegradedReason::MissingStratum => "Stratum missing",
            ClassificationDegradedReason::InvalidStratum => "Stratum invalid",
            ClassificationDegradedReason::MissingEssentia => "Essentia missing",
            ClassificationDegradedReason::InvalidEssentia => "Essentia invalid",
        })
        .collect();
    format!(
        "Degraded classification ({}) requires review and cannot be auto-staged.",
        labels.join(", ")
    )
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
        mode: ClassificationMode::Full,
        degraded_reasons: Vec::new(),
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
        let has_enrichment = !evidence.discogs_mapped.is_empty();
        let enrichment_supports_bass = evidence
            .discogs_mapped
            .iter()
            .any(|mg| genre::genre_family(mg.genre) == GenreFamily::Bass);

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
    let has_any = !evidence.discogs_mapped.is_empty();
    if !has_any {
        return false;
    }
    evidence.discogs_mapped.iter().all(|mg| {
        matches!(
            genre::genre_family(mg.genre),
            GenreFamily::House | GenreFamily::Techno | GenreFamily::Bass | GenreFamily::Hardcore
        )
    })
}

fn find_family_genre(evidence: &TrackEvidence, family: GenreFamily) -> Option<&'static str> {
    evidence
        .discogs_mapped
        .iter()
        .filter(|mg| genre::genre_family(mg.genre) == family)
        .max_by_key(|mg| mg.style_count)
        .map(|mg| mg.genre)
}

fn resolve_current_canonical(current_genre: &str) -> Option<&'static str> {
    if current_genre.is_empty() {
        return None;
    }
    genre::resolve_genre(current_genre)
}

fn current_genre_tokens(current_genre: &str) -> Vec<&'static str> {
    if let Some(canonical) = resolve_current_canonical(current_genre) {
        vec![canonical]
    } else {
        genre::extract_genre_tokens(current_genre)
    }
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

fn build_review_hint(evidence: &TrackEvidence, flags: &[String]) -> String {
    let mut hints = Vec::new();
    if evidence.discogs_readiness() != DiscogsReadiness::UsableGenre {
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
mod tests;
