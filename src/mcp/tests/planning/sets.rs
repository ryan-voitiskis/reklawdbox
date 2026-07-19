use std::collections::{HashMap, HashSet};

use rmcp::handler::server::wrapper::Parameters;
use rusqlite::params;

use crate::adapters::state as store;
use crate::domain::planning::{
    AxisScore, EnergyPhase, HarmonicMixingStyle, PriorityWeights, SequencePolicy,
    SequencingPriority, TrackProfile, TransitionScores, build_candidate_plan,
    build_candidate_plan_beam, compute_bpm_trajectory, compute_track_energy, priority_weights,
    resolve_energy_curve, round_to_3_decimals, score_energy_axis,
};
use crate::mcp::planning::{
    BuildSetParams, EnergyCurveInput as McpEnergyCurveInput,
    EnergyCurvePreset as McpEnergyCurvePreset, EnergyPhase as McpEnergyPhase, TransitionWeightSpec,
};

use super::super::common::{
    create_server_with_connections, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track,
};
use super::support::{
    create_build_set_test_db, make_test_profile, mixing_policy, seed_build_set_cache,
};

fn sequence_policy<'a>(
    target_track_count: usize,
    energy_phases: &'a [EnergyPhase],
    weights: &'a PriorityWeights,
    target_bpms: Option<&'a [f64]>,
) -> SequencePolicy<'a> {
    SequencePolicy {
        target_track_count,
        energy_phases,
        mixing: mixing_policy(weights, true, Some(HarmonicMixingStyle::Balanced)),
        bpm_drift_pct: 6.0,
        target_bpms,
    }
}

fn make_beam_test_profiles() -> HashMap<String, TrackProfile> {
    let tracks = vec![
        make_test_profile("b1", "8A", 126.0, 0.4, "Deep House"),
        make_test_profile("b2", "9A", 127.0, 0.5, "Deep House"),
        make_test_profile("b3", "10A", 128.0, 0.6, "House"),
        make_test_profile("b4", "11A", 129.0, 0.7, "House"),
        make_test_profile("b5", "12A", 130.0, 0.8, "Tech House"),
    ];
    tracks
        .into_iter()
        .map(|p| (p.track.id.clone(), p))
        .collect()
}

fn assert_float_close(actual: f64, expected: f64, field: &str) {
    assert!(
        (actual - expected).abs() <= 1e-12,
        "{field} differs: actual={actual:.15}, expected={expected:.15}",
    );
}

fn assert_axis_score_parity(actual: &AxisScore, expected: &AxisScore, field: &str) {
    assert_float_close(actual.value, expected.value, field);
    assert_eq!(actual.label, expected.label, "{field} label differs");
}

fn assert_transition_score_parity(actual: &TransitionScores, expected: &TransitionScores) {
    assert_axis_score_parity(&actual.key, &expected.key, "key score");
    assert_axis_score_parity(&actual.bpm, &expected.bpm, "BPM score");
    assert_axis_score_parity(&actual.energy, &expected.energy, "energy score");
    assert_axis_score_parity(&actual.genre, &expected.genre, "genre score");
    assert_axis_score_parity(&actual.brightness, &expected.brightness, "brightness score");
    assert_axis_score_parity(&actual.rhythm, &expected.rhythm, "rhythm score");
    assert_float_close(actual.composite, expected.composite, "composite score");
    assert_eq!(
        actual.effective_to_key, expected.effective_to_key,
        "effective target key differs",
    );
    assert_eq!(
        actual.pitch_shift_semitones, expected.pitch_shift_semitones,
        "pitch shift differs",
    );
    assert_eq!(
        actual.key_relation, expected.key_relation,
        "key relation differs",
    );
    assert_float_close(
        actual.bpm_adjustment_pct,
        expected.bpm_adjustment_pct,
        "BPM adjustment",
    );
    assert_eq!(
        actual.adjustments.len(),
        expected.adjustments.len(),
        "adjustment count differs",
    );
    for (index, (actual, expected)) in actual
        .adjustments
        .iter()
        .zip(&expected.adjustments)
        .enumerate()
    {
        assert_eq!(
            actual.kind, expected.kind,
            "adjustment {index} kind differs"
        );
        assert_float_close(
            actual.delta,
            expected.delta,
            &format!("adjustment {index} delta"),
        );
        assert_float_close(
            actual.composite_without,
            expected.composite_without,
            &format!("adjustment {index} pre-adjustment composite"),
        );
        assert_eq!(
            actual.reason, expected.reason,
            "adjustment {index} reason differs",
        );
    }
}

fn assert_greedy_beam_parity(
    profiles: &HashMap<String, TrackProfile>,
    start_track_id: &str,
    policy: SequencePolicy<'_>,
) {
    let greedy = build_candidate_plan(profiles, start_track_id, policy, 0);
    let beam_plans = build_candidate_plan_beam(profiles, start_track_id, policy, 1);

    assert!(
        greedy
            .transitions
            .iter()
            .any(|transition| !transition.scores.adjustments.is_empty()),
        "parity fixture should exercise ordered score adjustments",
    );
    assert_eq!(
        beam_plans.len(),
        1,
        "beam width 1 should produce exactly 1 plan",
    );
    let beam = &beam_plans[0];
    assert_eq!(
        greedy.ordered_ids, beam.ordered_ids,
        "beam width 1 should match greedy ordering",
    );
    assert_eq!(
        greedy.transitions.len(),
        beam.transitions.len(),
        "beam width 1 should produce the same transition count",
    );
    for (index, (greedy, beam)) in greedy.transitions.iter().zip(&beam.transitions).enumerate() {
        assert_eq!(
            greedy.from_index, beam.from_index,
            "transition {index} source index differs",
        );
        assert_eq!(
            greedy.to_index, beam.to_index,
            "transition {index} target index differs",
        );
        assert_transition_score_parity(&greedy.scores, &beam.scores);
    }
}

#[tokio::test]
async fn mcp_planning_set_build_set_generates_candidates_and_transition_scores() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: Some(McpEnergyCurveInput::Preset(
                McpEnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            opening_track_id: None,
            candidates: Some(3),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed for fixture pool");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 6);
    assert_eq!(payload["tracks_used"], 4);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert_eq!(candidates.len(), 3);

    for candidate in candidates {
        let tracks = candidate["tracks"]
            .as_array()
            .expect("candidate tracks should be an array");
        let transitions = candidate["transitions"]
            .as_array()
            .expect("candidate transitions should be an array");
        assert_eq!(tracks.len(), 4);
        assert_eq!(transitions.len(), 3);
        assert!(
            candidate["set_score"].as_f64().is_some(),
            "set_score should be numeric"
        );
        let set_score = candidate["set_score"]
            .as_f64()
            .expect("set_score should be numeric");
        assert!(
            (set_score - round_to_3_decimals(set_score)).abs() < 1e-9,
            "set_score should be rounded to 3 decimal places"
        );
        assert!(
            candidate["estimated_duration_minutes"].as_i64().is_some(),
            "estimated_duration_minutes should be numeric"
        );
        for transition in transitions {
            assert!(
                transition["scores"]["composite"].as_f64().is_some(),
                "each transition should include numeric composite score"
            );
            assert!(
                transition["key_relation"].is_string(),
                "each transition should include key_relation"
            );
            assert!(
                transition["bpm_adjustment_pct"].is_number(),
                "each transition should include bpm_adjustment_pct"
            );
        }
    }

    let candidate_a_ids: Vec<String> = candidates[0]["tracks"]
        .as_array()
        .expect("candidate A tracks array")
        .iter()
        .map(|track| {
            track["track_id"]
                .as_str()
                .expect("candidate track should include track_id")
                .to_string()
        })
        .collect();
    let candidate_b_ids: Vec<String> = candidates[1]["tracks"]
        .as_array()
        .expect("candidate B tracks array")
        .iter()
        .map(|track| {
            track["track_id"]
                .as_str()
                .expect("candidate track should include track_id")
                .to_string()
        })
        .collect();
    assert_ne!(
        candidate_a_ids, candidate_b_ids,
        "candidate generation should include variation"
    );
}

#[tokio::test]
async fn mcp_planning_set_build_set_adapts_energy_curve_to_single_track_pool() {
    let db_conn = create_single_track_test_db("single-set-1", "/tmp/single-set-1.flac");
    db_conn
        .execute(
            "UPDATE djmdContent SET Length = 0 WHERE ID = ?1",
            params!["single-set-1"],
        )
        .expect("single-track fixture should update");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: vec!["single-set-1".to_string()],
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("energy".into())),
            energy_curve: Some(McpEnergyCurveInput::Custom(vec![
                McpEnergyPhase::Warmup,
                McpEnergyPhase::Build,
                McpEnergyPhase::Peak,
                McpEnergyPhase::Release,
            ])),
            opening_track_id: None,
            candidates: Some(2),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed for single-track pool");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 1);
    assert_eq!(payload["tracks_used"], 1);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert_eq!(candidates.len(), 1);
    let first = &candidates[0];
    assert_eq!(
        first["tracks"]
            .as_array()
            .expect("tracks should be array")
            .len(),
        1
    );
    assert_eq!(
        first["transitions"]
            .as_array()
            .expect("transitions should be array")
            .len(),
        0
    );
    assert_eq!(
        first["estimated_duration_minutes"]
            .as_i64()
            .expect("duration should be integer"),
        6
    );
}

#[tokio::test]
async fn mcp_planning_set_build_set_produces_candidates_from_homogeneous_key_pool() {
    let db_conn = create_single_track_test_db("same-key-1", "/tmp/same-key-1.flac");
    insert_test_track(
        &db_conn,
        "same-key-2",
        "Same Key Two",
        "g1",
        "/tmp/same-key-2.flac",
    );
    insert_test_track(
        &db_conn,
        "same-key-3",
        "Same Key Three",
        "g1",
        "/tmp/same-key-3.flac",
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: vec![
                "same-key-1".to_string(),
                "same-key-2".to_string(),
                "same-key-3".to_string(),
            ],
            target_tracks: 3,
            priority: Some(TransitionWeightSpec::Named("harmonic".into())),
            energy_curve: Some(McpEnergyCurveInput::Preset(
                McpEnergyCurvePreset::FlatEnergy,
            )),
            opening_track_id: None,
            candidates: Some(2),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed when all tracks share the same key");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 3);
    assert_eq!(payload["tracks_used"], 3);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    // With beam search (beam_width=2 from candidates), beam explores different
    // orderings of the same 3-track pool, yielding 1 or 2 candidates.
    assert!(
        !candidates.is_empty() && candidates.len() <= 2,
        "same-key pool with beam_width=2 should produce 1-2 candidates; got {}",
        candidates.len()
    );
    assert_eq!(
        candidates[0]["transitions"]
            .as_array()
            .expect("transitions should be an array")
            .len(),
        2
    );
}

#[tokio::test]
async fn mcp_planning_set_build_set_recomputes_preset_curve_when_pool_is_smaller_than_target() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let selected: Vec<String> = track_ids.into_iter().take(3).collect();

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: selected,
            target_tracks: 6,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: Some(McpEnergyCurveInput::Preset(
                McpEnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            opening_track_id: None,
            candidates: Some(1),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed when pool is smaller than target");
    let payload = extract_json(&result);

    assert_eq!(payload["tracks_used"], 3);
    let transitions = payload["candidates"][0]["transitions"]
        .as_array()
        .expect("candidate transitions should be an array");
    assert_eq!(transitions.len(), 2);
    let second_energy_label = transitions[1]["scores"]["energy"]["label"]
        .as_str()
        .expect("second transition should include energy label");
    assert!(
        second_energy_label.contains("peak phase"),
        "phase scaling should include a peak phase for the final transition when tracks_used=3; got: {second_energy_label}"
    );
}

#[test]
fn mcp_planning_set_bpm_trajectory_drift_penalty() {
    use std::collections::HashMap;

    let start = make_test_profile("bpm-start", "8A", 128.0, 0.7, "House");
    let close = make_test_profile("bpm-close", "8A", 130.0, 0.7, "House");
    let far = make_test_profile("bpm-far", "8A", 145.0, 0.7, "House");

    let mut profiles: HashMap<String, TrackProfile> = HashMap::new();
    profiles.insert("bpm-start".to_string(), start);
    profiles.insert("bpm-close".to_string(), close);
    profiles.insert("bpm-far".to_string(), far);

    let phases = [EnergyPhase::Build, EnergyPhase::Build, EnergyPhase::Build];
    let weights = priority_weights(SequencingPriority::Harmonic);
    let policy = |bpm_drift_pct| SequencePolicy {
        target_track_count: 3,
        energy_phases: &phases,
        mixing: mixing_policy(&weights, true, None),
        bpm_drift_pct,
        target_bpms: None,
    };

    let tight = build_candidate_plan(&profiles, "bpm-start", policy(3.0), 0);
    assert_eq!(tight.ordered_ids[1], "bpm-close");

    let moderate = build_candidate_plan(&profiles, "bpm-start", policy(6.0), 0);
    assert_eq!(moderate.ordered_ids[1], "bpm-close");
    assert!(moderate.ordered_ids.contains(&"bpm-far".to_string()));

    let generous = build_candidate_plan(&profiles, "bpm-start", policy(50.0), 0);
    assert_eq!(generous.ordered_ids[1], "bpm-close");
    assert!(generous.ordered_ids.contains(&"bpm-far".to_string()));
}

#[test]
fn mcp_planning_set_bpm_proxy_energy_keeps_peak_phase_reachable_without_essentia() {
    let from_energy = compute_track_energy(None, 126.0);
    let to_energy = compute_track_energy(None, 130.0);
    let peak = score_energy_axis(
        from_energy,
        to_energy,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        None,
    );

    assert!(
        to_energy >= 0.65,
        "fallback energy should allow peak thresholds"
    );
    assert_eq!(peak.value, 1.0);
    assert_eq!(peak.label, "High and stable (peak phase)");
}

#[test]
fn mcp_planning_set_bpm_trajectory_warmup_build_peak_release() {
    let phases = vec![
        EnergyPhase::Warmup,
        EnergyPhase::Build,
        EnergyPhase::Build,
        EnergyPhase::Build,
        EnergyPhase::Peak,
        EnergyPhase::Peak,
        EnergyPhase::Release,
        EnergyPhase::Release,
    ];
    let trajectory = compute_bpm_trajectory(&phases, 124.0, 132.0);
    assert_eq!(trajectory.len(), 8);
    assert_eq!(trajectory[0], 124.0);
    assert_eq!(trajectory[1], 124.0);
    assert_eq!(trajectory[2], 128.0);
    assert_eq!(trajectory[3], 132.0);
    assert_eq!(trajectory[4], 132.0);
    assert_eq!(trajectory[5], 132.0);
    assert_eq!(trajectory[6], 132.0);
    assert_eq!(trajectory[7], 124.0);
}

#[test]
fn mcp_planning_set_bpm_trajectory_flat_curve() {
    let phases = vec![EnergyPhase::Peak; 5];
    let trajectory = compute_bpm_trajectory(&phases, 126.0, 133.0);
    assert_eq!(trajectory.len(), 5);
    for bpm in &trajectory {
        assert_eq!(*bpm, 133.0);
    }
}

#[test]
fn mcp_planning_set_bpm_trajectory_single_position() {
    let trajectory = compute_bpm_trajectory(&[EnergyPhase::Peak], 128.0, 132.0);
    assert_eq!(trajectory.len(), 1);
    assert_eq!(trajectory[0], 132.0);
}

#[test]
fn mcp_planning_set_bpm_trajectory_empty() {
    let trajectory = compute_bpm_trajectory(&[], 128.0, 132.0);
    assert!(trajectory.is_empty());
}

#[test]
fn mcp_planning_set_bpm_trajectory_single_build_single_release() {
    let phases = vec![EnergyPhase::Build, EnergyPhase::Peak, EnergyPhase::Release];
    let trajectory = compute_bpm_trajectory(&phases, 120.0, 130.0);
    assert_eq!(trajectory[0], 125.0); // midpoint for single build
    assert_eq!(trajectory[1], 130.0); // peak
    assert_eq!(trajectory[2], 125.0); // midpoint for single release
}

#[test]
fn mcp_planning_set_beam_search_width_1_matches_greedy() {
    let profiles = make_beam_test_profiles();
    let phases = resolve_energy_curve(None, 4).unwrap();

    assert_greedy_beam_parity(
        &profiles,
        "b1",
        sequence_policy(
            4,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            None,
        ),
    );
}

#[test]
fn mcp_planning_set_beam_search_wider_produces_multiple_plans() {
    let profiles = make_beam_test_profiles();
    let phases = resolve_energy_curve(None, 4).unwrap();

    let plans = build_candidate_plan_beam(
        &profiles,
        "b1",
        sequence_policy(
            4,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            None,
        ),
        4,
    );

    assert!(
        plans.len() > 1,
        "beam width 4 with 5-track pool should produce multiple plans; got {}",
        plans.len()
    );

    for plan in &plans {
        assert_eq!(plan.ordered_ids.len(), 4);
        assert_eq!(plan.transitions.len(), 3);
        assert_eq!(plan.ordered_ids[0], "b1", "all plans should start with b1");
    }

    let unique: HashSet<&Vec<String>> = plans.iter().map(|p| &p.ordered_ids).collect();
    assert_eq!(unique.len(), plans.len(), "all plans should be distinct");
}

#[test]
fn mcp_planning_set_beam_search_empty_pool() {
    let profiles: HashMap<String, TrackProfile> = HashMap::new();
    let plans = build_candidate_plan_beam(
        &profiles,
        "missing",
        SequencePolicy {
            target_track_count: 4,
            energy_phases: &[EnergyPhase::Peak; 4],
            mixing: mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
            bpm_drift_pct: 6.0,
            target_bpms: None,
        },
        3,
    );
    assert_eq!(plans.len(), 1, "empty pool should still produce one plan");
    assert_eq!(plans[0].ordered_ids, vec!["missing"]);
    assert!(plans[0].transitions.is_empty());
}

#[test]
fn mcp_planning_set_beam_search_width_exceeding_pool_size() {
    let mut profiles = HashMap::new();
    profiles.insert(
        "only1".to_string(),
        make_test_profile("only1", "8A", 128.0, 0.5, "House"),
    );
    profiles.insert(
        "only2".to_string(),
        make_test_profile("only2", "9A", 128.5, 0.6, "House"),
    );

    let plans = build_candidate_plan_beam(
        &profiles,
        "only1",
        SequencePolicy {
            target_track_count: 2,
            energy_phases: &[EnergyPhase::Peak; 2],
            mixing: mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
            bpm_drift_pct: 6.0,
            target_bpms: None,
        },
        10,
    );

    assert_eq!(plans.len(), 1, "only one possible plan with 2-track pool");
    assert_eq!(plans[0].ordered_ids.len(), 2);
}

#[test]
fn mcp_planning_set_beam_search_with_bpm_trajectory() {
    let profiles = make_beam_test_profiles();
    let phases = vec![
        EnergyPhase::Warmup,
        EnergyPhase::Build,
        EnergyPhase::Peak,
        EnergyPhase::Peak,
    ];
    let target_bpms = compute_bpm_trajectory(&phases, 126.0, 130.0);

    let plans = build_candidate_plan_beam(
        &profiles,
        "b1",
        sequence_policy(
            4,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            Some(&target_bpms),
        ),
        3,
    );

    assert!(
        !plans.is_empty(),
        "beam search with trajectory should produce plans"
    );
    for plan in &plans {
        assert_eq!(plan.ordered_ids.len(), 4);
        assert_eq!(plan.ordered_ids[0], "b1");
    }
}

#[tokio::test]
async fn mcp_planning_set_build_set_beam_search_produces_multiple_candidates() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: None,
            beam_width: Some(5),
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set with beam_width=5 should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["beam_width"], 5);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert!(
        candidates.len() > 1,
        "beam_width=5 should produce multiple candidates; got {}",
        candidates.len()
    );

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 4);
        assert!(candidate["set_score"].is_number());
    }
}

#[tokio::test]
async fn mcp_planning_set_build_set_with_bpm_range_includes_trajectory_fields() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: None,
            beam_width: Some(3),
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: Some((124.0, 131.0)),
        }))
        .await
        .expect("build_set with bpm_range should succeed");

    let payload = extract_json(&result);

    let trajectory = payload["bpm_trajectory"]
        .as_array()
        .expect("bpm_trajectory should be present at set level");
    assert_eq!(trajectory.len(), 4, "trajectory should match target_tracks");

    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        for track in tracks {
            assert!(
                track["play_at_bpm"].is_number(),
                "tracks should include play_at_bpm when bpm_range is set"
            );
            assert!(
                track["pitch_adjustment_pct"].is_number(),
                "tracks should include pitch_adjustment_pct when bpm_range is set"
            );
        }

        let candidate_trajectory = candidate["bpm_trajectory"]
            .as_array()
            .expect("candidate should include bpm_trajectory");
        assert_eq!(candidate_trajectory.len(), 4);
    }
}

#[tokio::test]
async fn mcp_planning_set_build_set_beam_width_1_backward_compatible() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: track_ids.clone(),
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: Some(1),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set with candidates=1 should succeed");

    let payload = extract_json(&result);
    assert_eq!(
        payload["beam_width"], 1,
        "candidates=1 should route to greedy"
    );
    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 4);
    }
}
