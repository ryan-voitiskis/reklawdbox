use rmcp::handler::server::wrapper::Parameters;
use rusqlite::params;

use crate::adapters::state as store;
use crate::domain::classification::taxonomy::GenreFamily;
use crate::domain::planning::{
    EnergyPhase, HarmonicMixingStyle, SequencingPriority, TrackProfile, format_camelot,
    key_to_camelot, musical_key_to_camelot, parse_camelot_key, priority_weights, score_bpm_axis,
    score_genre_axis, score_key_axis, score_transition_profiles, transpose_camelot_key,
};
use crate::mcp::planning::{
    EnergyPhase as McpEnergyPhase, HarmonicMixingStyle as McpHarmonicMixingStyle,
    QueryTransitionCandidatesParams, ScoreTransitionParams, TransitionWeightSpec,
};

use super::super::common::{
    create_server_with_connections, create_single_track_test_db, default_http_client_for_tests,
    extract_json, set_test_audio_analysis,
};
use super::support::{
    create_build_set_test_db, make_test_profile, mixing_policy, seed_build_set_cache,
    transition_moment,
};

#[test]
fn mcp_planning_transition_musical_key_to_camelot_converts_major_minor_and_flats() {
    assert_eq!(
        musical_key_to_camelot("Am").map(format_camelot).as_deref(),
        Some("8A")
    );
    assert_eq!(
        musical_key_to_camelot("C").map(format_camelot).as_deref(),
        Some("8B")
    );
    assert_eq!(
        musical_key_to_camelot("F#m").map(format_camelot).as_deref(),
        Some("11A")
    );
    assert_eq!(
        musical_key_to_camelot("Bb").map(format_camelot).as_deref(),
        Some("6B")
    );
    assert_eq!(
        musical_key_to_camelot("Dbm").map(format_camelot).as_deref(),
        Some("12A")
    );
    assert_eq!(
        key_to_camelot("8a").map(format_camelot).as_deref(),
        Some("8A")
    );
    assert_eq!(musical_key_to_camelot("not-a-key"), None);
}

#[test]
fn mcp_planning_transition_camelot_distance_scoring_handles_wrap_and_mode_shift() {
    let wrap_up = score_key_axis(parse_camelot_key("12A"), parse_camelot_key("1A"));
    assert_eq!(wrap_up.value, 0.9);
    assert!(
        wrap_up.label.contains("Camelot adjacent"),
        "wrap-around up should be treated as +1"
    );

    let wrap_down = score_key_axis(parse_camelot_key("1A"), parse_camelot_key("12A"));
    assert_eq!(wrap_down.value, 0.9);
    assert!(
        wrap_down.label.contains("Camelot adjacent"),
        "wrap-around down should be treated as -1"
    );

    let mood_shift = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("6B"));
    assert_eq!(mood_shift.value, 0.8);

    let diagonal = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("7B"));
    assert_eq!(diagonal.value, 0.55);
    assert!(
        diagonal.label.contains("Energy diagonal"),
        "cross-letter ±1 should be Energy diagonal"
    );
}

#[test]
fn mcp_planning_transition_key_axis_covers_all_relation_types() {
    let perfect = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("8A"));
    assert_eq!(perfect.value, 1.0);
    assert_eq!(perfect.label, "Perfect");

    let adjacent = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("9A"));
    assert_eq!(adjacent.value, 0.9);
    assert!(adjacent.label.contains("Camelot adjacent"));

    let mood = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("8B"));
    assert_eq!(mood.value, 0.8);
    assert!(mood.label.contains("Mood shift"));

    let diagonal = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("9B"));
    assert_eq!(diagonal.value, 0.55);
    assert!(diagonal.label.contains("Energy diagonal"));

    let extended = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("10A"));
    assert_eq!(extended.value, 0.45);
    assert!(extended.label.contains("Extended"));

    let clash = score_key_axis(parse_camelot_key("1A"), parse_camelot_key("6A"));
    assert_eq!(clash.value, 0.1);
    assert_eq!(clash.label, "Clash");
}

#[test]
fn mcp_planning_transition_bpm_exponential_scoring_curve() {
    // exp(-0.019 * pct²): 0% → 1.0, monotonically decreasing
    let seamless = score_bpm_axis(128.0, 129.5); // 1.17%
    assert!(
        seamless.value > 0.97,
        "1.17% should score near 1.0, got {}",
        seamless.value
    );
    assert!(seamless.label.contains("Seamless"));

    let comfortable = score_bpm_axis(130.0, 126.5); // 2.69%
    assert!(
        comfortable.value > 0.85 && comfortable.value < 0.95,
        "2.69% should be ~0.87, got {}",
        comfortable.value
    );
    assert!(comfortable.label.contains("Comfortable"));

    let noticeable = score_bpm_axis(120.0, 125.5); // 4.58%
    assert!(
        noticeable.value > 0.55 && noticeable.value < 0.75,
        "4.58% should be ~0.65, got {}",
        noticeable.value
    );
    assert!(noticeable.label.contains("Noticeable"));

    let creative = score_bpm_axis(128.0, 138.0); // 7.81%
    assert!(
        creative.value > 0.25 && creative.value < 0.45,
        "7.81% should be ~0.33, got {}",
        creative.value
    );
    assert!(creative.label.contains("Creative transition needed"));

    let jarring = score_bpm_axis(120.0, 132.0); // 10.0%
    assert!(
        jarring.value < 0.20,
        "10% should be near 0, got {}",
        jarring.value
    );
    assert!(jarring.label.contains("Jarring"));

    let unknown = score_bpm_axis(0.0, 128.0);
    assert_eq!(unknown.value, 0.5);
    assert_eq!(unknown.label, "Unknown BPM");

    let at_0 = score_bpm_axis(128.0, 128.0);
    let at_1 = score_bpm_axis(128.0, 129.28); // ~1%
    let at_3 = score_bpm_axis(128.0, 131.84); // ~3%
    let at_5 = score_bpm_axis(128.0, 134.4); // ~5%
    let at_8 = score_bpm_axis(128.0, 138.24); // ~8%
    assert!(at_0.value > at_1.value);
    assert!(at_1.value > at_3.value);
    assert!(at_3.value > at_5.value);
    assert!(at_5.value > at_8.value);
}

#[test]
fn mcp_planning_transition_transpose_camelot_key_circle_of_fifths() {
    // +1 semitone = +7 Camelot positions: 8A → 3A
    let k8a = parse_camelot_key("8A").unwrap();
    let up1 = transpose_camelot_key(k8a, 1);
    assert_eq!(format_camelot(up1), "3A");

    let full = transpose_camelot_key(k8a, 12);
    assert_eq!(format_camelot(full), "8A");

    let down1 = transpose_camelot_key(k8a, -1);
    assert_eq!(format_camelot(down1), "1A");

    let round_trip = transpose_camelot_key(up1, -1);
    assert_eq!(format_camelot(round_trip), "8A");

    let k5b = parse_camelot_key("5B").unwrap();
    let up2 = transpose_camelot_key(k5b, 2);
    assert!(
        format_camelot(up2).ends_with('B'),
        "letter should be preserved through transposition"
    );
}

#[test]
fn mcp_planning_transition_master_tempo_off_changes_key_scoring() {
    let from = TrackProfile {
        track: crate::domain::library::Track {
            id: "mt-from".to_string(),
            title: "MT From".to_string(),
            artist: "Test".to_string(),
            album: String::new(),
            genre: "House".to_string(),
            key: "Am".to_string(),
            bpm: 128.0,
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 300,
            file_path: "/tmp/mt-from.flac".to_string(),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        },
        camelot_key: parse_camelot_key("8A"),
        key_display: "8A".to_string(),
        bpm: 128.0,
        energy: 0.6,
        brightness: None,
        rhythm_regularity: None,
        loudness_range: None,
        canonical_genre: Some("House".to_string()),
        genre_family: GenreFamily::House,
        timbral: None,
    };

    // 128/135 BPM → -1 semitone pitch shift
    let mut to = from.clone();
    to.track.id = "mt-to".to_string();
    to.bpm = 135.0;
    to.camelot_key = parse_camelot_key("8A"); // same key naturally

    let scores_mt_on = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(None, None, 0, None),
    );
    assert_eq!(
        scores_mt_on.key.value, 1.0,
        "master_tempo on: same key should be perfect"
    );
    assert_eq!(scores_mt_on.pitch_shift_semitones, 0);
    assert!(scores_mt_on.effective_to_key.is_none());

    let scores_mt_off = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, None),
    );
    assert_eq!(
        scores_mt_off.pitch_shift_semitones, -1,
        "128→135 BPM should yield -1 semitone shift"
    );
    assert_eq!(
        scores_mt_off.effective_to_key,
        Some("1A".to_string()),
        "8A shifted -1 semitone = 1A"
    );
    // Continuous detuning blends Perfect (1.0) and Clash (0.1) weighted by
    // fractional semitones (-0.909), so score is slightly above 0.1.
    assert!(
        scores_mt_off.key.value > 0.1 && scores_mt_off.key.value < 0.25,
        "128→135: key score should be slightly above 0.1 due to detuning blend, got {}",
        scores_mt_off.key.value,
    );
    assert_eq!(
        scores_mt_on.key.value, 1.0,
        "master_tempo on: same key is perfect (1.0)"
    );
}

#[test]
fn mcp_planning_transition_detuning_eliminates_cliff_at_rounding_boundary() {
    // Regression: old rounding caused a 10x score cliff at 0.5 semitones.
    // Continuous detuning should produce similar scores on either side.
    let from = make_test_profile("cliff-from", "8A", 128.0, 0.5, "House");
    let to_under = make_test_profile("cliff-under", "8A", 131.5, 0.5, "House");
    let to_over = make_test_profile("cliff-over", "8A", 132.0, 0.5, "House");

    let scores_under = score_transition_profiles(
        &from,
        &to_under,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, None),
    );
    let scores_over = score_transition_profiles(
        &from,
        &to_over,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, None),
    );

    let diff = (scores_under.key.value - scores_over.key.value).abs();
    assert!(
        diff < 0.15,
        "Key scores across the rounding boundary should be similar: \
         under={:.3} (shift {:.3}), over={:.3} (shift {:.3}), diff={:.3}",
        scores_under.key.value,
        12.0 * (128.0_f64 / 131.5).log2(),
        scores_over.key.value,
        12.0 * (128.0_f64 / 132.0).log2(),
        diff,
    );

    assert!(
        scores_under.key.value < 0.85,
        "~0.46 semitone detuning should noticeably reduce key score, got {}",
        scores_under.key.value,
    );
}

#[test]
fn mcp_planning_transition_detuning_smooth_degradation_with_increasing_shift() {
    let from = make_test_profile("smooth-from", "8A", 128.0, 0.5, "House");
    let bpms = [128.0, 129.0, 130.0, 131.0, 132.0, 133.0, 134.0, 135.0];
    let mut prev_score = 1.1_f64;

    for &bpm in &bpms {
        let to = make_test_profile("smooth-to", "8A", bpm, 0.5, "House");
        let scores = score_transition_profiles(
            &from,
            &to,
            mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
            transition_moment(None, None, 0, None),
        );

        assert!(
            scores.key.value <= prev_score + 0.01,
            "Key score should not increase: at {bpm} BPM got {:.3}, prev was {:.3}",
            scores.key.value,
            prev_score,
        );
        prev_score = scores.key.value;
    }

    let same = make_test_profile("smooth-same", "8A", 128.0, 0.5, "House");
    let scores_same = score_transition_profiles(
        &from,
        &same,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, None),
    );
    assert_eq!(scores_same.key.value, 1.0, "Same BPM should be perfect");
}

#[test]
fn mcp_planning_transition_detuning_master_tempo_on_unchanged() {
    let from = make_test_profile("mt-on-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("mt-on-to", "8A", 135.0, 0.5, "House");
    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(None, None, 0, None),
    );

    assert_eq!(
        scores.key.value, 1.0,
        "Master tempo ON: same key should always be perfect regardless of BPM"
    );
}

#[test]
fn mcp_planning_transition_detuning_label_shows_cents_when_audible() {
    let from = make_test_profile("label-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("label-to", "8A", 130.5, 0.5, "House");
    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, None),
    );

    assert!(
        scores.key.label.contains("detuned"),
        "Label should mention detuning for ~34 cents shift, got: {}",
        scores.key.label,
    );
}

#[test]
fn mcp_planning_transition_detuning_play_bpms_bilinear_interpolation() {
    // Both tracks have fractional pitch shifts, exercising all 4 bilinear blend combinations.
    let from = make_test_profile("pb-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("pb-to", "8A", 132.0, 0.5, "House");
    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, Some((130.0, 130.0))), // both pitched to 130
    );

    assert!(
        scores.key.value > 0.5 && scores.key.value < 0.85,
        "Bilinear blend with ~0.53 total shift should score moderately, got {}",
        scores.key.value,
    );
}

#[test]
fn mcp_planning_transition_detuning_play_bpms_master_tempo_on_ignores_shifts() {
    let from = make_test_profile("pb-mt-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("pb-mt-to", "8A", 135.0, 0.5, "House");
    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true, // master tempo ON
            None,
        ),
        transition_moment(None, None, 0, Some((130.0, 130.0))),
    );

    assert_eq!(
        scores.key.value, 1.0,
        "Master tempo ON with play_bpms: same key should be perfect, got {}",
        scores.key.value,
    );
}

#[test]
fn mcp_planning_transition_detuning_play_bpms_asymmetric_one_zero_shift() {
    let from = make_test_profile("pb-asym-from", "8A", 130.0, 0.5, "House");
    let to = make_test_profile("pb-asym-to", "8A", 132.0, 0.5, "House");
    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, Some((130.0, 130.0))), // from at native, to pitched down 2 BPM
    );

    assert!(
        scores.key.value > 0.7 && scores.key.value < 0.9,
        "One-sided ~0.26 semitone shift should score ~0.76, got {}",
        scores.key.value,
    );
}

#[test]
fn mcp_planning_transition_harmonic_style_conservative_penalizes_poor_transitions() {
    let from = make_test_profile("hs-from", "8A", 128.0, 0.7, "House");
    let to = make_test_profile("hs-to", "9B", 128.0, 0.7, "House");

    let conservative = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );

    let baseline = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );

    assert!(
        conservative.composite < baseline.composite,
        "conservative should penalize key=0.55 at peak phase"
    );
    let expected = baseline.composite * 0.1;
    assert!(
        (conservative.composite - expected).abs() < 1e-9,
        "conservative penalty should be 0.1x; got {} vs expected {}",
        conservative.composite,
        expected
    );

    let adventurous = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Adventurous),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );
    assert_eq!(
        adventurous.composite, baseline.composite,
        "adventurous should not penalize key=0.55 at peak phase"
    );

    // key=0.45 at exactly the Balanced build threshold (0.45) should not trigger penalty
    let from2 = make_test_profile("hs-from2", "8A", 128.0, 0.5, "House");
    let to2 = make_test_profile("hs-to2", "10A", 128.0, 0.6, "House");
    let balanced_build = score_transition_profiles(
        &from2,
        &to2,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Balanced),
        ),
        transition_moment(Some(EnergyPhase::Build), Some(EnergyPhase::Build), 0, None),
    );
    let baseline_build = score_transition_profiles(
        &from2,
        &to2,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(Some(EnergyPhase::Build), Some(EnergyPhase::Build), 0, None),
    );
    assert_eq!(
        balanced_build.composite, baseline_build.composite,
        "balanced should not penalize key=0.45 at build phase (exactly at threshold)"
    );
}

#[test]
fn mcp_planning_transition_harmonic_style_adventurous_is_phase_dependent() {
    let from = make_test_profile("adv-from", "8A", 128.0, 0.7, "House");
    let to = make_test_profile("adv-to", "2A", 128.0, 0.7, "House");

    let adv_peak = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Adventurous),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );
    let baseline_peak = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );
    assert_eq!(
        adv_peak.composite, baseline_peak.composite,
        "adventurous at peak should not penalize key=0.1 (threshold is 0.1)"
    );

    let adv_warmup = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Adventurous),
        ),
        transition_moment(
            Some(EnergyPhase::Warmup),
            Some(EnergyPhase::Warmup),
            0,
            None,
        ),
    );
    let baseline_warmup = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(
            Some(EnergyPhase::Warmup),
            Some(EnergyPhase::Warmup),
            0,
            None,
        ),
    );
    assert!(
        adv_warmup.composite < baseline_warmup.composite,
        "adventurous at warmup should penalize key=0.1 (threshold is 0.45)"
    );
    let expected = baseline_warmup.composite * 0.5;
    assert!(
        (adv_warmup.composite - expected).abs() < 1e-9,
        "adventurous penalty should be 0.5x; got {} vs expected {}",
        adv_warmup.composite,
        expected
    );

    let cons_peak = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );
    let cons_warmup = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
        ),
        transition_moment(
            Some(EnergyPhase::Warmup),
            Some(EnergyPhase::Warmup),
            0,
            None,
        ),
    );
    assert!(cons_peak.composite < baseline_peak.composite);
    assert!(cons_warmup.composite < baseline_warmup.composite);
}

#[test]
fn mcp_planning_transition_score_genre_axis_treats_missing_genre_as_neutral() {
    let unknown_source = score_genre_axis(
        None,
        Some("House"),
        GenreFamily::Other,
        GenreFamily::House,
        0,
    );
    assert_eq!(unknown_source.value, 0.5);
    assert_eq!(unknown_source.label, "Unknown genre");

    let unknown_destination = score_genre_axis(
        Some("House"),
        None,
        GenreFamily::House,
        GenreFamily::Other,
        0,
    );
    assert_eq!(unknown_destination.value, 0.5);
    assert_eq!(unknown_destination.label, "Unknown genre");
}

#[test]
fn mcp_planning_transition_genre_stickiness_bonus_and_penalty() {
    let approx = |a: f64, b: f64| (a - b).abs() < 1e-9;

    let bonus = score_genre_axis(
        Some("Deep House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        3,
    );
    assert!(
        approx(bonus.value, 0.8),
        "0.7 + 0.1 streak bonus; got {}",
        bonus.value
    );
    assert!(bonus.label.contains("streak bonus"));

    let no_bonus = score_genre_axis(
        Some("Deep House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        5,
    );
    assert_eq!(no_bonus.value, 0.7);
    assert!(!no_bonus.label.contains("streak bonus"));

    let penalty = score_genre_axis(
        Some("House"),
        Some("Drum & Bass"),
        GenreFamily::House,
        GenreFamily::Bass,
        1,
    );
    assert!(
        approx(penalty.value, 0.2),
        "0.3 - 0.1 early switch penalty; got {}",
        penalty.value
    );
    assert!(penalty.label.contains("early switch penalty"));

    let no_penalty = score_genre_axis(
        Some("House"),
        Some("Drum & Bass"),
        GenreFamily::House,
        GenreFamily::Bass,
        2,
    );
    assert_eq!(no_penalty.value, 0.3);
    assert!(!no_penalty.label.contains("early switch penalty"));

    let first = score_genre_axis(
        Some("House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        0,
    );
    assert_eq!(first.value, 0.7);
    assert!(!first.label.contains("streak bonus"));
}

#[tokio::test]
async fn mcp_planning_contract_score_transition_returns_expected_axis_scores() {
    let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let from_path = temp_audio_dir.path().join("from-track.flac");
    let to_path = temp_audio_dir.path().join("to-track.flac");
    std::fs::write(&from_path, b"from-track-audio").expect("source track fixture should write");
    std::fs::write(&to_path, b"to-track-audio").expect("target track fixture should write");
    let from_path_str = from_path.to_string_lossy().to_string();
    let to_path_str = to_path.to_string_lossy().to_string();
    let from_metadata = std::fs::metadata(&from_path).expect("source track metadata should load");
    let to_metadata = std::fs::metadata(&to_path).expect("target track metadata should load");
    let from_file_size = from_metadata.len() as i64;
    let to_file_size = to_metadata.len() as i64;
    let from_file_mtime = from_metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let to_file_mtime = to_metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let db_conn = create_single_track_test_db("from-track", &from_path_str);
    db_conn
        .execute(
            "INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em')",
            [],
        )
        .expect("second key should insert");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, 'Second Track', 'a1', 'al1', 'g1', 'k2', 'c1', 'l1', '',
                    12350, 153, 'score transition test', 2025, 260, ?2, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params!["to-track", &to_path_str],
        )
        .expect("second track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &from_path_str,
        "stratum-dsp",
        from_file_size,
        from_file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":122.0,"key":"Am","key_camelot":"8A"}"#,
    )
    .expect("source stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &to_path_str,
        "stratum-dsp",
        to_file_size,
        to_file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":123.5,"key":"Em","key_camelot":"9A"}"#,
    )
    .expect("destination stratum cache should seed");

    set_test_audio_analysis(
        &store_conn,
        &from_path_str,
        "essentia",
        from_file_size,
        from_file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":0.90,"loudness_integrated":-12.0,"onset_rate":3.0}"#,
    )
    .expect("source essentia cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &to_path_str,
        "essentia",
        to_file_size,
        to_file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":1.80,"loudness_integrated":-8.0,"onset_rate":5.0}"#,
    )
    .expect("destination essentia cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "from-track".to_string(),
            target_track_id: "to-track".to_string(),
            energy_phase: Some(McpEnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
        }))
        .await
        .expect("score_transition should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["from"]["track_id"], "from-track");
    assert_eq!(payload["from"]["key"], "8A");
    assert_eq!(payload["to"]["track_id"], "to-track");
    assert_eq!(payload["to"]["key"], "9A");

    assert_eq!(payload["scores"]["key"]["value"], 0.9);
    assert_eq!(payload["scores"]["bpm"]["value"], 0.791);
    assert_eq!(payload["scores"]["energy"]["value"], 1.0);
    assert_eq!(payload["scores"]["genre"]["value"], 1.0);
    assert_eq!(payload["scores"]["brightness"]["value"], 0.5);
    assert_eq!(payload["scores"]["rhythm"]["value"], 0.5);
    assert_eq!(payload["scores"]["composite"], 0.915);

    assert!(
        payload["key_relation"].is_string(),
        "key_relation should be present"
    );
    assert!(
        payload["key_relation"]
            .as_str()
            .unwrap()
            .contains("Camelot adjacent")
    );
    assert!(
        payload["bpm_adjustment_pct"].is_number(),
        "bpm_adjustment_pct should be present"
    );
    let bpm_pct = payload["bpm_adjustment_pct"].as_f64().unwrap();
    assert!(
        bpm_pct > 3.0 && bpm_pct < 4.0,
        "128→123.5 is ~3.52%; got {bpm_pct}"
    );
}

#[tokio::test]
async fn mcp_planning_transition_score_transition_balanced_default_penalizes_clash() {
    // harmonic_style: None defaults to Balanced, which applies 0.5x penalty on Clash
    let db_conn = create_single_track_test_db("clash-from", "/tmp/clash-from.flac");
    db_conn
        .execute(
            "INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Bbm')",
            [],
        )
        .expect("second key should insert");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, 'Clash Track', 'a1', 'al1', 'g1', 'k2', 'c1', 'l1', '',
                    12200, 153, 'clash test', 2025, 260, ?2, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params!["clash-to", "/tmp/clash-to.flac"],
        )
        .expect("second track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        "/tmp/clash-from.flac",
        "stratum-dsp",
        1,
        1,
        "stratum-dsp-1.0.0",
        r#"{"bpm":122.0,"key":"Am","key_camelot":"8A"}"#,
    )
    .expect("from stratum should seed");
    set_test_audio_analysis(
        &store_conn,
        "/tmp/clash-to.flac",
        "stratum-dsp",
        1,
        1,
        "stratum-dsp-1.0.0",
        r#"{"bpm":122.0,"key":"Bbm","key_camelot":"2A"}"#,
    )
    .expect("to stratum should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let penalized = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "clash-from".to_string(),
            target_track_id: "clash-to".to_string(),
            energy_phase: Some(McpEnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
        }))
        .await
        .expect("score_transition should succeed");
    let penalized_payload = extract_json(&penalized);

    let unpenalized = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "clash-from".to_string(),
            target_track_id: "clash-to".to_string(),
            energy_phase: Some(McpEnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: Some(McpHarmonicMixingStyle::Adventurous),
        }))
        .await
        .expect("score_transition should succeed");
    let unpenalized_payload = extract_json(&unpenalized);

    assert_eq!(penalized_payload["scores"]["key"]["value"], 0.1);
    assert_eq!(unpenalized_payload["scores"]["key"]["value"], 0.1);

    let penalized_composite = penalized_payload["scores"]["composite"].as_f64().unwrap();
    let unpenalized_composite = unpenalized_payload["scores"]["composite"].as_f64().unwrap();
    let expected = unpenalized_composite * 0.5;
    assert!(
        (penalized_composite - expected).abs() < 0.01,
        "Balanced default should halve composite for Clash; got {penalized_composite} vs expected {expected}"
    );
}

#[test]
fn mcp_planning_transition_play_bpms_none_preserves_existing_behavior() {
    let from = make_test_profile("pb-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pb-to", "9A", 130.0, 0.7, "House");

    let without = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(None, None, 0, None),
    );
    assert!(without.composite > 0.0);
    assert!(without.effective_to_key.is_none());
    assert_eq!(without.pitch_shift_semitones, 0);
}

#[test]
fn mcp_planning_transition_play_bpms_affects_bpm_adjustment_pct() {
    let from = make_test_profile("pbadj-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pbadj-to", "9A", 126.0, 0.7, "House");

    let with_play = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(None, None, 0, Some((128.0, 130.0))),
    );
    assert!(
        (with_play.bpm_adjustment_pct - 3.174).abs() < 0.1,
        "bpm_adjustment_pct should reflect target vs native; got {}",
        with_play.bpm_adjustment_pct
    );
}

#[test]
fn mcp_planning_transition_play_bpms_affects_key_transposition() {
    let from = make_test_profile("pbkey-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pbkey-to", "8A", 128.0, 0.7, "House");

    let no_shift = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, Some((128.0, 128.0))),
    );
    assert_eq!(
        no_shift.key.value, 1.0,
        "same play BPM, same native key = perfect"
    );

    let big_shift = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), false, None),
        transition_moment(None, None, 0, Some((128.0, 136.0))),
    );
    assert_ne!(
        big_shift.pitch_shift_semitones, 0,
        "large BPM shift should transpose key"
    );
}

#[tokio::test]
async fn mcp_planning_transition_query_transition_candidates_ranks_pool() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let from_id = track_ids[0].clone();
    let pool_ids: Vec<String> = track_ids[1..].to_vec();

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: from_id.clone(),
            candidate_track_ids: Some(pool_ids),
            playlist_id: None,
            target_bpm: None,
            energy_phase: Some(McpEnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect("query_transition_candidates should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["from"]["track_id"], from_id);
    assert!(payload["master_tempo"].as_bool().unwrap());

    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert!(
        !candidates.is_empty(),
        "should return at least one candidate"
    );

    let composites: Vec<f64> = candidates
        .iter()
        .map(|c| c["scores"]["composite"].as_f64().unwrap())
        .collect();
    for window in composites.windows(2) {
        assert!(
            window[0] >= window[1],
            "candidates should be sorted by composite descending"
        );
    }

    for c in candidates {
        assert!(c["track_id"].is_string());
        assert!(c["native_bpm"].is_number());
        assert!(c["native_key"].is_string());
        assert!(c["bpm_difference_pct"].is_number());
        assert!(c["key_relation"].is_string());
        assert!(c["scores"]["composite"].is_number());
        assert!(
            c.get("play_at_bpm").is_none() || c["play_at_bpm"].is_null(),
            "play_at_bpm should not be present without target_bpm"
        );
        assert!(
            c.get("pitch_adjustment_pct").is_none() || c["pitch_adjustment_pct"].is_null(),
            "pitch_adjustment_pct should not be present without target_bpm"
        );
    }
}

#[tokio::test]
async fn mcp_planning_transition_query_transition_candidates_with_target_bpm() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: track_ids[0].clone(),
            candidate_track_ids: Some(track_ids[1..].to_vec()),
            playlist_id: None,
            target_bpm: Some(130.0),
            energy_phase: None,
            priority: None,
            use_master_tempo: None,
            harmonic_style: None,
            limit: Some(3),
        }))
        .await
        .expect("query_transition_candidates with target_bpm should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["reference_bpm"], 130.0);

    let candidates = payload["candidates"].as_array().unwrap();
    assert!(candidates.len() <= 3, "limit should be respected");

    for c in candidates {
        assert_eq!(
            c["play_at_bpm"].as_f64().unwrap(),
            130.0,
            "play_at_bpm should equal target_bpm for all candidates"
        );
        assert!(
            c["pitch_adjustment_pct"].as_f64().unwrap() >= 0.0,
            "pitch_adjustment_pct should be non-negative"
        );
    }
}

#[tokio::test]
async fn mcp_planning_transition_query_transition_candidates_master_tempo_off() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: track_ids[0].clone(),
            candidate_track_ids: Some(track_ids[1..].to_vec()),
            playlist_id: None,
            target_bpm: Some(135.0), // significant BPM shift to trigger key transposition
            energy_phase: None,
            priority: None,
            use_master_tempo: Some(false),
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect("query_transition_candidates with master_tempo off should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["master_tempo"], false);
    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());
    let has_shift = candidates
        .iter()
        .any(|c| c.get("pitch_shift_semitones").is_some());
    assert!(
        has_shift,
        "with master_tempo off and large BPM shift, some candidates should have pitch_shift_semitones"
    );
}

#[tokio::test]
async fn mcp_planning_transition_query_transition_candidates_rejects_missing_pool() {
    let db_conn = create_single_track_test_db("orphan-track", "/tmp/orphan.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let err = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: "orphan-track".to_string(),
            candidate_track_ids: None,
            playlist_id: None,
            target_bpm: None,
            energy_phase: None,
            priority: None,
            use_master_tempo: None,
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect_err("should reject when neither pool_track_ids nor playlist_id is set");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("pool_track_ids") || msg.contains("playlist_id"),
        "error should mention required pool source; got: {msg}"
    );
}
