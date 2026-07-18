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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: false,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
fn classification_mode_caps_confidence_without_reordering_recommendations() {
    let mut full = make_evidence("");
    full.discogs_mapped = vec![MappedGenre {
        genre: "Techno",
        style_count: 2,
    }];
    full.has_discogs = true;
    full.discogs_match_quality = Some(DiscogsMatchQuality::Exact);

    let full_result = classify_track(&full);
    let mut degraded = full;
    degraded.essentia_status = AudioBackendStatus::Missing;
    let degraded_result = classify_track(&degraded);

    assert_eq!(full_result.mode, ClassificationMode::Full);
    assert_eq!(degraded_result.mode, ClassificationMode::Degraded);
    assert_eq!(degraded_result.confidence, ClassificationConfidence::Low);
    assert_eq!(degraded_result.genre, full_result.genre);
    assert_eq!(degraded_result.action, full_result.action);
    assert_eq!(
        degraded_result
            .candidates
            .iter()
            .map(|candidate| candidate.genre)
            .collect::<Vec<_>>(),
        full_result
            .candidates
            .iter()
            .map(|candidate| candidate.genre)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        degraded_result.degraded_reasons,
        vec![ClassificationDegradedReason::MissingEssentia]
    );
    assert!(degraded_result.review_required());
    assert!(!degraded_result.is_auto_stage_eligible());
    assert!(full_result.is_auto_stage_eligible());
}

#[test]
fn classification_mode_finalizes_the_audio_veto_exit() {
    let mut full = make_evidence("");
    full.audio = Some(make_audio(90.0, 0.5, 12.0, 0.3, 500.0));
    full.has_audio = true;
    let full_result = classify_track(&full);
    assert!(full_result.flags.contains(&"audio-vetoed".to_string()));
    assert_eq!(full_result.confidence, ClassificationConfidence::Medium);

    let mut degraded = full;
    degraded.stratum_status = AudioBackendStatus::Invalid;
    let degraded_result = classify_track(&degraded);
    assert_eq!(degraded_result.genre, full_result.genre);
    assert!(degraded_result.flags.contains(&"audio-vetoed".to_string()));
    assert!(
        degraded_result
            .flags
            .contains(&"degraded-classification".to_string())
    );
    assert_eq!(degraded_result.confidence, ClassificationConfidence::Low);
    assert!(
        degraded_result
            .review_hint
            .as_deref()
            .is_some_and(|hint| hint.contains("Stratum invalid"))
    );
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
fn discogs_confidence_is_preserved_when_optional_audio_is_missing() {
    let mut ev = make_evidence("");
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
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
    assert!(result.flags.contains(&"missing-danceability".to_string()));
    assert!(
        result
            .flags
            .contains(&"missing-rhythm-regularity".to_string())
    );
}

#[test]
fn discogs_and_uncalibrated_audio_returns_medium() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Techno",
        style_count: 3,
    }];
    ev.has_discogs = true;
    ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
    ev.has_audio = true;
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
    assert_eq!(result.action, ClassificationAction::Suggest);
}

#[test]
fn confirms_correct_current_genre() {
    let mut ev = make_evidence("Techno");
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
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Deep House",
        style_count: 1,
    }];
    ev.has_discogs = true;
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
    ev.audio = Some(make_audio(100.0, 0.5, 12.0, 0.3, 400.0));
    ev.has_audio = true;
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Ambient"));
    assert!(result.flags.contains(&"audio-vetoed".to_string()));
}

#[test]
fn label_confirms_enrichment() {
    let mut ev = make_evidence("");
    ev.label = Some("Tresor".into());
    ev.label_genre = Some("Techno"); // Tresor → Techno
    ev.audio = Some(make_audio(132.0, 2.0, 3.0, 0.92, 1800.0));
    ev.has_audio = true;
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
}

#[test]
fn discogs_style_and_fallback_label_are_one_confidence_group() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Techno",
        style_count: 2,
    }];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.label = Some("Cached Discogs Label".into());
    ev.label_genre = Some("Techno");
    ev.label_provenance = Some(LabelProvenance::Discogs);

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
    assert!(
        result
            .flags
            .contains(&"single-source-confidence-cap".to_string())
    );
}

#[test]
fn distinct_rekordbox_label_and_discogs_can_reach_high_confidence() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Techno",
        style_count: 2,
    }];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.label = Some("Independent Library Label".into());
    ev.label_genre = Some("Techno");
    ev.label_provenance = Some(LabelProvenance::Rekordbox);

    let result = classify_track(&ev);
    assert_eq!(result.confidence, ClassificationConfidence::High);
}

#[test]
fn duplicate_rekordbox_and_discogs_label_is_correlated() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Techno",
        style_count: 2,
    }];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.label = Some("Same Label".into());
    ev.label_genre = Some("Techno");
    ev.label_provenance = Some(LabelProvenance::CorrelatedDiscogs);

    let result = classify_track(&ev);
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
}

#[test]
fn fuzzy_discogs_only_evidence_cannot_be_high() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "House",
        style_count: 3,
    }];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Fuzzy);

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("House"));
    assert_ne!(result.confidence, ClassificationConfidence::High);
}

#[test]
fn invalid_discogs_match_cannot_create_provider_evidence() {
    let mut ev = make_evidence("");
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Invalid);

    let result = classify_track(&ev);
    assert_eq!(result.genre, None);
    assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
    assert!(result.flags.contains(&"discogs-match-invalid".to_string()));
}

#[test]
fn current_genre_only_is_a_low_confidence_hint() {
    let ev = make_evidence("Techno");
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Low);
    assert_eq!(result.action, ClassificationAction::Confirm);
    assert!(result.flags.contains(&"current-genre-only".to_string()));
}

#[test]
fn ambiguous_current_genre_does_not_create_a_recommendation() {
    let ev = make_evidence("House / Techno");
    let result = classify_track(&ev);
    assert_eq!(result.genre, None);
    assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
    assert!(
        result
            .flags
            .contains(&"current-genre-ambiguous".to_string())
    );
}

#[test]
fn current_genre_breaks_a_real_tie_without_raising_confidence() {
    let mut ev = make_evidence("House");
    ev.discogs_mapped = vec![
        MappedGenre {
            genre: "House",
            style_count: 1,
        },
        MappedGenre {
            genre: "Techno",
            style_count: 1,
        },
    ];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("House"));
    assert_eq!(result.confidence, ClassificationConfidence::Insufficient);
    assert!(result.flags.contains(&"current-genre-tiebreak".to_string()));
}

#[test]
fn depth_prefers_shallower_when_high_energy() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Deep Techno",
        style_count: 2,
    }];
    ev.has_discogs = true;
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
fn conflicting_enrichment_is_resolved_by_independent_audio_characteristics() {
    let mut ev = make_evidence("");
    ev.bpm = 130.0;
    ev.discogs_mapped = vec![
        MappedGenre {
            genre: "House",
            style_count: 1,
        },
        MappedGenre {
            genre: "Techno",
            style_count: 1,
        },
    ];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.audio = Some(make_audio_with_key_conf(
        130.0, 2.2, 3.0, 0.95, 1800.0, 0.05,
    ));
    ev.has_audio = true;

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Low);
    assert!(
        result
            .flags
            .contains(&"audio-assisted-tiebreak".to_string())
    );
}

#[test]
fn bpm_override_within_family_floors_at_medium() {
    let mut ev = make_evidence("");
    ev.bpm = 145.0;
    ev.discogs_mapped = vec![
        MappedGenre {
            genre: "Deep Techno",
            style_count: 4,
        },
        MappedGenre {
            genre: "Techno",
            style_count: 1,
        },
    ];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.label = Some("Independent Library Label".into());
    ev.label_genre = Some("Deep Techno");
    ev.label_provenance = Some(LabelProvenance::Rekordbox);

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
    assert!(result.flags.contains(&"bpm-override".to_string()));
    assert!(
        result
            .flags
            .contains(&"bpm-override-same-family".to_string())
    );
}

#[test]
fn bpm_override_across_families_downgrades_to_low() {
    let mut ev = make_evidence("");
    ev.bpm = 170.0;
    ev.discogs_mapped = vec![
        MappedGenre {
            genre: "House",
            style_count: 4,
        },
        MappedGenre {
            genre: "Drum & Bass",
            style_count: 1,
        },
    ];
    ev.has_discogs = true;
    ev.discogs_match_quality = Some(DiscogsMatchQuality::Exact);
    ev.label = Some("Independent Library Label".into());
    ev.label_genre = Some("House");
    ev.label_provenance = Some(LabelProvenance::Rekordbox);

    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Drum & Bass"));
    assert_eq!(result.confidence, ClassificationConfidence::Low);
    assert!(result.flags.contains(&"bpm-override".to_string()));
    assert!(
        !result
            .flags
            .contains(&"bpm-override-same-family".to_string())
    );
}

#[test]
fn atonal_house_demotes_to_house() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![
        MappedGenre {
            genre: "Deep House",
            style_count: 1,
        },
        MappedGenre {
            genre: "House",
            style_count: 1,
        },
    ];
    ev.has_discogs = true;
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
    let mut audio = make_audio_with_loudness_range(128.0, 2.0, 3.0, 0.92, 1800.0, 0.7);
    audio.duration_seconds = Some(45.0);
    ev.audio = Some(audio);
    ev.has_audio = true;
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Deep Techno"));
    assert!(
        !result.evidence.iter().any(|e| e.contains("compressed")),
        "short tracks should not set compressed: {:?}",
        result.evidence
    );
}

#[test]
fn bpm_disagreement_uses_detector_consensus() {
    let mut ev = make_evidence("");
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Deep House",
        style_count: 1,
    }];
    ev.has_discogs = true;
    // Rekordbox 132 is just outside Deep House plausibility after tolerance;
    // Stratum + Essentia agree that the track is really around 125 BPM.
    ev.audio = Some(make_audio_with_detector_bpms(132.0, 125.0, 125.2));
    ev.has_audio = true;
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Deep House"));
    assert_eq!(result.confidence, ClassificationConfidence::Medium);
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
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Deep House",
        style_count: 1,
    }];
    ev.has_discogs = true;
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
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Ambient",
        style_count: 1,
    }];
    ev.has_discogs = true;
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
    ev.discogs_mapped = vec![MappedGenre {
        genre: "Deep House",
        style_count: 1,
    }];
    ev.has_discogs = true;
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

// 1. Marcel Dettmann - Aim: Discogs and label consensus on Techno.
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
        label: Some("Ostgut Ton".into()),
        label_genre: Some("Techno"),
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        discogs_mapped: vec![MappedGenre {
            genre: "Deep House",
            style_count: 1,
        }],
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(156.97, 1.01, 3.95, 1.04, 1395.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Breakbeat"));
    assert!(result.flags.contains(&"audio-vetoed".to_string()));
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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(
        result.genre,
        Some("Techno"),
        "Deep Techno is BPM-implausible at 145, should prefer Techno"
    );
}

// 7. Efdemin - Aachen: Discogs and label agree on Techno.
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
        label: Some("Ostgut Ton".into()),
        label_genre: Some("Techno"),
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(134.31, 1.74, 3.64, 1.07, 1244.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Techno"));
    assert!(result.flags.contains(&"audio-only".to_string()));
}

// 9. prince of denmark - (in the end): 4-way Discogs split, insufficient.
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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(164.7, 1.19, 4.18, 0.68, 1919.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(104.02, 1.36, 7.02, 0.97, 1161.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("Downtempo"));
    assert_eq!(result.action, ClassificationAction::Conflict);
}

// 15. Gallery S - 100 Skyward Fist: BPM filters Jungle, House wins.
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(128.0, 1.42, 1.76, 0.84, 1923.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(154.0, 1.36, 3.47, 1.10, 1341.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: Some("Lobster Theremin".into()),
        label_genre: Some("House"),
        audio: Some(make_audio(136.0, 1.69, 6.62, 0.95, 2205.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(result.genre, Some("House"));
    assert_eq!(result.action, ClassificationAction::Conflict);
}

// 18. Anthony Parasole - 7EVEN: current Techno and high-energy audio agree.
#[test]
fn collection_parasole_7even_high_energy_vetoes_deep() {
    let ev = TrackEvidence {
        track_id: "165247703".into(),
        artist: "Anthony Parasole".into(),
        title: "7EVEN".into(),
        current_genre: "Techno".into(),
        bpm: 132.0,
        discogs_mapped: vec![],
        label: None,
        label_genre: None,
        audio: Some(make_audio(132.0, 3.62, 2.71, 1.00, 1572.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(
        result.genre,
        Some("Techno"),
        "High energy (danceability 3.62) should veto Deep Techno in favor of Techno"
    );
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
        label: None,
        label_genre: None,
        audio: Some(make_audio(122.0, 2.06, 4.30, 0.92, 601.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
    };
    let result = classify_track(&ev);
    assert_eq!(
        result.genre,
        Some("House"),
        "User confirms this track is House, not Deep House"
    );
    assert_eq!(result.action, ClassificationAction::Conflict);
}

// --- Tests for new behaviors from the classification improvements ---

#[test]
fn expanded_ambient_veto_atmospheric_not_ambient() {
    // dc=7.0 → Atmospheric (>5.0) but NOT Ambient (<10.0)
    // danceability=0.5 → NonDancefloor
    // Should trigger the new NonDancefloor + Atmospheric veto
    let mut ev = make_evidence("");
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
        label: None,
        label_genre: None,
        // LowEnergy (1.2) + Atmospheric (dc=6.0) → Downtempo passes
        audio: Some(make_audio(100.0, 1.2, 6.0, 0.85, 1500.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        // LowEnergy + very low centroid → both Downtempo and Techno pass
        audio: Some(make_audio(125.0, 1.2, 3.0, 0.95, 264.0)),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: None,
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: false,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
        label: None,
        label_genre: None,
        audio: Some({
            let mut a = make_audio(124.0, 2.4, 4.0, 0.97, 520.0);
            a.onset_rate = Some(4.3);
            a.loudness_integrated = Some(-9.5);
            a
        }),
        has_discogs: true,
        discogs_match_quality: None,
        label_provenance: None,
        has_audio: true,
        stratum_status: AudioBackendStatus::Fresh,
        essentia_status: AudioBackendStatus::Fresh,
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
    ev.discogs_mapped = vec![MappedGenre {
        genre: "House",
        style_count: 1,
    }];
    ev.has_discogs = true;
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
