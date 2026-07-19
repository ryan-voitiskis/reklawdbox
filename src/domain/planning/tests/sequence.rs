use std::collections::HashMap;

use crate::domain::planning::*;

use super::support::{
    ProfileAnalysis, ProfileSpec, mixing_policy, sequence_policy, simple_profile, synth_profile,
};

const MEAN_COMPOSITE_MIN: f64 = 0.65;

const MIN_COMPOSITE_MIN: f64 = 0.30;

const COMPOSITE_VARIANCE_MAX: f64 = 0.08;

const HARMONIC_COHERENCE_MIN: f64 = 0.50;

const ENERGY_FIDELITY_MIN: f64 = 0.40;

const MAX_PITCH_ADJUSTMENT: f64 = 8.0;

struct EvalMetrics {
    mean_composite: f64,
    min_composite: f64,
    composite_variance: f64,
    harmonic_coherence: f64,
    energy_fidelity: f64,
    max_pitch_pct: f64,
}

fn compute_metrics(plan: &CandidatePlan, _phases: &[EnergyPhase]) -> EvalMetrics {
    let composites: Vec<f64> = plan
        .transitions
        .iter()
        .map(|t| t.scores.composite)
        .collect();
    let n = composites.len() as f64;
    if n < 1.0 {
        return EvalMetrics {
            mean_composite: 0.0,
            min_composite: 0.0,
            composite_variance: 0.0,
            harmonic_coherence: 0.0,
            energy_fidelity: 0.0,
            max_pitch_pct: 0.0,
        };
    }

    let mean = composites.iter().sum::<f64>() / n;
    let min = composites.iter().copied().fold(f64::INFINITY, f64::min);
    let variance = composites.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / n;

    let harmonic_ok = plan
        .transitions
        .iter()
        .filter(|t| t.scores.key.value >= 0.8)
        .count();
    let energy_ok = plan
        .transitions
        .iter()
        .filter(|t| t.scores.energy.value >= 1.0 - f64::EPSILON)
        .count();
    let max_pitch = plan
        .transitions
        .iter()
        .map(|t| t.scores.bpm_adjustment_pct)
        .fold(0.0_f64, f64::max);

    EvalMetrics {
        mean_composite: mean,
        min_composite: min,
        composite_variance: variance,
        harmonic_coherence: harmonic_ok as f64 / n,
        energy_fidelity: energy_ok as f64 / n,
        max_pitch_pct: max_pitch,
    }
}

fn assert_quality_gates(metrics: &EvalMetrics, pool_name: &str) {
    assert!(
        metrics.mean_composite >= MEAN_COMPOSITE_MIN,
        "[{pool_name}] mean composite {:.3} < threshold {MEAN_COMPOSITE_MIN}",
        metrics.mean_composite,
    );
    assert!(
        metrics.min_composite >= MIN_COMPOSITE_MIN,
        "[{pool_name}] min composite {:.3} < threshold {MIN_COMPOSITE_MIN}",
        metrics.min_composite,
    );
    assert!(
        metrics.composite_variance <= COMPOSITE_VARIANCE_MAX,
        "[{pool_name}] composite variance {:.4} > threshold {COMPOSITE_VARIANCE_MAX}",
        metrics.composite_variance,
    );
    assert!(
        metrics.harmonic_coherence >= HARMONIC_COHERENCE_MIN,
        "[{pool_name}] harmonic coherence {:.2} < threshold {HARMONIC_COHERENCE_MIN}",
        metrics.harmonic_coherence,
    );
    assert!(
        metrics.energy_fidelity >= ENERGY_FIDELITY_MIN,
        "[{pool_name}] energy fidelity {:.2} < threshold {ENERGY_FIDELITY_MIN}",
        metrics.energy_fidelity,
    );
    assert!(
        metrics.max_pitch_pct <= MAX_PITCH_ADJUSTMENT,
        "[{pool_name}] max pitch adjustment {:.1}% > threshold {MAX_PITCH_ADJUSTMENT}%",
        metrics.max_pitch_pct,
    );
}

fn build_pool(profiles: Vec<TrackProfile>) -> HashMap<String, TrackProfile> {
    profiles
        .into_iter()
        .map(|p| (p.track.id.clone(), p))
        .collect()
}

const SEQUENCE_GOLDEN_TOLERANCE: f64 = 1e-12;

type TransitionVector = [f64; 8];
type AdjustmentGolden<'a> = &'a [(&'a str, f64, f64)];

fn assert_sequence_float(actual: f64, expected: f64, label: &str) {
    assert!(
        (actual - expected).abs() <= SEQUENCE_GOLDEN_TOLERANCE,
        "{label}: expected {expected:.17}, got {actual:.17}",
    );
}

fn assert_transition_goldens(
    plan: &CandidatePlan,
    expected_vectors: &[TransitionVector],
    expected_adjustments: &[AdjustmentGolden<'_>],
) {
    assert_eq!(plan.transitions.len(), expected_vectors.len());
    assert_eq!(plan.transitions.len(), expected_adjustments.len());
    for (index, ((transition, expected), adjustments)) in plan
        .transitions
        .iter()
        .zip(expected_vectors)
        .zip(expected_adjustments)
        .enumerate()
    {
        assert_eq!(transition.from_index, index);
        assert_eq!(transition.to_index, index + 1);
        let scores = &transition.scores;
        let actual = [
            scores.key.value,
            scores.bpm.value,
            scores.energy.value,
            scores.genre.value,
            scores.brightness.value,
            scores.rhythm.value,
            scores.composite,
            scores.bpm_adjustment_pct,
        ];
        for (axis, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
            assert_sequence_float(
                actual,
                *expected,
                &format!("transition {index} axis {axis}"),
            );
        }
        assert_eq!(scores.effective_to_key, None);
        assert_eq!(scores.pitch_shift_semitones, 0);
        assert_eq!(scores.adjustments.len(), adjustments.len());
        for (actual, (kind, delta, composite_without)) in
            scores.adjustments.iter().zip(*adjustments)
        {
            assert_eq!(actual.kind, *kind);
            assert_sequence_float(actual.delta, *delta, "adjustment delta");
            assert_sequence_float(
                actual.composite_without,
                *composite_without,
                "pre-adjustment composite",
            );
        }
    }
}

fn golden_sequence_profiles() -> HashMap<String, TrackProfile> {
    build_pool(vec![
        simple_profile("b1", "8A", 126.0, 0.4, "Deep House"),
        simple_profile("b2", "9A", 127.0, 0.5, "Deep House"),
        simple_profile("b3", "10A", 128.0, 0.6, "House"),
        simple_profile("b4", "11A", 129.0, 0.7, "House"),
        simple_profile("b5", "12A", 130.0, 0.8, "Tech House"),
    ])
}

#[test]
fn planning_sequence_base_golden_preserves_greedy_beam_and_bpm_trajectory() {
    let profiles = golden_sequence_profiles();
    let phases = [
        EnergyPhase::Warmup,
        EnergyPhase::Build,
        EnergyPhase::Peak,
        EnergyPhase::Peak,
    ];
    let target_bpms = compute_bpm_trajectory(&phases, 126.0, 130.0);
    assert_eq!(target_bpms, [126.0, 128.0, 130.0, 130.0]);
    let weights = priority_weights(SequencingPriority::Balanced);
    let policy = SequencePolicy {
        target_track_count: 4,
        energy_phases: &phases,
        mixing: mixing_policy(&weights, false, Some(HarmonicMixingStyle::Balanced)),
        bpm_drift_pct: 6.0,
        target_bpms: Some(&target_bpms),
    };

    let greedy_zero = build_candidate_plan(&profiles, "b1", policy, 0);
    assert_eq!(greedy_zero.ordered_ids, ["b1", "b2", "b3", "b4"]);
    assert_transition_goldens(
        &greedy_zero,
        &[
            [
                0.791_372_993_012_792_4,
                0.988_470_302_628_205_9,
                1.0,
                1.0,
                0.5,
                0.5,
                0.923_654_068_740_563_4,
                0.787_401_574_803_149_5,
            ],
            [
                0.634_955_953_910_760_7,
                0.956_025_766_200_672_3,
                0.5,
                0.8,
                0.5,
                0.5,
                0.714_931_693_427_485_5,
                1.562_5,
            ],
            [
                0.635_732_997_076_276_7,
                0.988_820_358_344_232_1,
                1.0,
                1.0,
                0.5,
                0.5,
                0.868_804_671_519_681_9,
                0.775_193_798_449_612_4,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.694_931_693_427_485_5)],
            &[("genre_streak", 0.02, 0.848_804_671_519_681_9)],
        ],
    );

    let greedy_one = build_candidate_plan(&profiles, "b1", policy, 1);
    assert_eq!(greedy_one.ordered_ids, ["b1", "b3", "b4", "b5"]);
    assert_transition_goldens(
        &greedy_one,
        &[
            [0.45, 1.0, 1.0, 0.7, 0.5, 0.5, 0.745_882_352_941_176_6, 0.0],
            [
                0.793_050_646_990_076_1,
                0.988_820_358_344_232_1,
                1.0,
                1.0,
                0.5,
                0.5,
                0.924_328_547_959_846_2,
                0.775_193_798_449_612_4,
            ],
            [
                0.793_050_646_990_076_1,
                1.0,
                0.5,
                0.8,
                0.5,
                0.5,
                0.781_076_698_937_674,
                0.0,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.904_328_547_959_846_2)],
            &[("genre_streak", 0.02, 0.761_076_698_937_673_9)],
        ],
    );

    let beam = build_candidate_plan_beam(&profiles, "b1", policy, 4);
    assert_eq!(
        beam.iter()
            .map(|plan| plan.ordered_ids.as_slice())
            .collect::<Vec<_>>(),
        [
            ["b1", "b2", "b3", "b4"].as_slice(),
            ["b1", "b3", "b4", "b5"].as_slice(),
            ["b1", "b3", "b5", "b4"].as_slice(),
            ["b1", "b3", "b4", "b2"].as_slice(),
        ],
    );
    assert_transition_goldens(
        &beam[0],
        &[
            [
                0.791_372_993_012_792_4,
                0.988_470_302_628_205_9,
                1.0,
                1.0,
                0.5,
                0.5,
                0.923_654_068_740_563_4,
                0.787_401_574_803_149_5,
            ],
            [
                0.634_955_953_910_760_7,
                0.956_025_766_200_672_3,
                0.5,
                0.8,
                0.5,
                0.5,
                0.714_931_693_427_485_5,
                1.562_5,
            ],
            [
                0.635_732_997_076_276_7,
                0.988_820_358_344_232_1,
                1.0,
                1.0,
                0.5,
                0.5,
                0.868_804_671_519_681_9,
                0.775_193_798_449_612_4,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.694_931_693_427_485_5)],
            &[("genre_streak", 0.02, 0.848_804_671_519_681_9)],
        ],
    );
    assert_transition_goldens(
        &beam[1],
        &[
            [0.45, 1.0, 1.0, 0.7, 0.5, 0.5, 0.745_882_352_941_176_6, 0.0],
            [
                0.793_050_646_990_076_1,
                0.988_820_358_344_232_1,
                1.0,
                1.0,
                0.5,
                0.5,
                0.924_328_547_959_846_2,
                0.775_193_798_449_612_4,
            ],
            [
                0.793_050_646_990_076_1,
                1.0,
                0.5,
                0.8,
                0.5,
                0.5,
                0.781_076_698_937_674,
                0.0,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.904_328_547_959_846_2)],
            &[("genre_streak", 0.02, 0.761_076_698_937_673_9)],
        ],
    );
    assert_transition_goldens(
        &beam[2],
        &[
            [0.45, 1.0, 1.0, 0.7, 0.5, 0.5, 0.745_882_352_941_176_6, 0.0],
            [0.45, 1.0, 0.5, 0.8, 0.5, 0.5, 0.66, 0.0],
            [
                0.793_050_646_990_076_1,
                0.988_820_358_344_232_1,
                0.5,
                0.8,
                0.5,
                0.5,
                0.778_446_195_018_669_7,
                0.775_193_798_449_612_4,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.64)],
            &[("genre_streak", 0.02, 0.758_446_195_018_669_7)],
        ],
    );
    assert_transition_goldens(
        &beam[3],
        &[
            [0.45, 1.0, 1.0, 0.7, 0.5, 0.5, 0.745_882_352_941_176_6, 0.0],
            [
                0.793_050_646_990_076_1,
                0.988_820_358_344_232_1,
                1.0,
                1.0,
                0.5,
                0.5,
                0.924_328_547_959_846_2,
                0.775_193_798_449_612_4,
            ],
            [
                0.299_565_607_666_593_6,
                0.903_767_237_891_080_2,
                0.5,
                0.8,
                0.5,
                0.5,
                0.292_131_252_869_526,
                2.362_204_724_409_448_6,
            ],
        ],
        &[
            &[],
            &[("genre_streak", 0.02, 0.904_328_547_959_846_2)],
            &[
                ("genre_streak", 0.02, 0.564_262_505_739_051_9),
                (
                    "harmonic_gate",
                    -0.292_131_252_869_526,
                    0.584_262_505_739_051_9,
                ),
            ],
        ],
    );
}

#[test]
fn planning_sequence_base_golden_preserves_id_tie_break_and_variation() {
    let profiles = build_pool(vec![
        simple_profile("tie-start", "8A", 126.0, 0.5, "House"),
        simple_profile("tie-a", "8A", 126.0, 0.5, "House"),
        simple_profile("tie-b", "8A", 126.0, 0.5, "House"),
        simple_profile("tie-c", "8A", 126.0, 0.5, "House"),
    ]);
    let phases = [EnergyPhase::Build; 4];
    let weights = priority_weights(SequencingPriority::Balanced);
    let policy = sequence_policy(4, &phases, &weights, HarmonicMixingStyle::Balanced);
    let expected_vectors = [
        [1.0, 1.0, 0.3, 1.0, 0.5, 0.5, 0.851_764_705_882_353_1, 0.0],
        [1.0, 1.0, 0.3, 1.0, 0.5, 0.5, 0.851_764_705_882_353_1, 0.0],
        [1.0, 1.0, 0.3, 1.0, 0.5, 0.5, 0.851_764_705_882_353_1, 0.0],
    ];
    let expected_adjustments: &[AdjustmentGolden<'_>] = &[
        &[],
        &[("genre_streak", 0.02, 0.831_764_705_882_353_1)],
        &[("genre_streak", 0.02, 0.831_764_705_882_353_1)],
    ];

    let variation_zero = build_candidate_plan(&profiles, "tie-start", policy, 0);
    assert_eq!(
        variation_zero.ordered_ids,
        ["tie-start", "tie-a", "tie-b", "tie-c"],
    );
    assert_transition_goldens(&variation_zero, &expected_vectors, expected_adjustments);

    let variation_one = build_candidate_plan(&profiles, "tie-start", policy, 1);
    assert_eq!(
        variation_one.ordered_ids,
        ["tie-start", "tie-b", "tie-a", "tie-c"],
    );
    assert_transition_goldens(&variation_one, &expected_vectors, expected_adjustments);
}

fn pool_camelot_walk() -> HashMap<String, TrackProfile> {
    build_pool(vec![
        simple_profile("cw1", "8A", 124.0, 0.35, "Deep House"),
        simple_profile("cw2", "9A", 124.5, 0.40, "Deep House"),
        simple_profile("cw3", "10A", 125.0, 0.50, "House"),
        simple_profile("cw4", "11A", 125.5, 0.55, "House"),
        simple_profile("cw5", "12A", 126.0, 0.65, "Tech House"),
        simple_profile("cw6", "1A", 126.5, 0.70, "Tech House"),
        simple_profile("cw7", "2A", 127.0, 0.60, "House"),
        simple_profile("cw8", "3A", 127.5, 0.45, "Deep House"),
    ])
}

fn pool_adversarial() -> HashMap<String, TrackProfile> {
    build_pool(vec![
        simple_profile("adv1", "1A", 120.0, 0.30, "Techno"),
        simple_profile("adv2", "5B", 132.0, 0.80, "Drum & Bass"),
        simple_profile("adv3", "9A", 140.0, 0.90, "Ambient"),
        simple_profile("adv4", "3B", 110.0, 0.20, "House"),
        simple_profile("adv5", "7A", 128.0, 0.60, "Dubstep"),
        simple_profile("adv6", "11B", 145.0, 0.95, "Trance"),
    ])
}

fn pool_iso_key_bpm() -> HashMap<String, TrackProfile> {
    build_pool(vec![
        synth_profile(
            ProfileSpec::new("iso1", "8A", 126.0, 0.30, "Deep House"),
            ProfileAnalysis::measured(1800.0, 0.7, 6.0),
        ),
        synth_profile(
            ProfileSpec::new("iso2", "8A", 126.0, 0.40, "Deep House"),
            ProfileAnalysis::measured(1900.0, 0.72, 7.0),
        ),
        synth_profile(
            ProfileSpec::new("iso3", "8A", 126.0, 0.55, "House"),
            ProfileAnalysis::measured(2200.0, 0.65, 8.5),
        ),
        synth_profile(
            ProfileSpec::new("iso4", "8A", 126.0, 0.65, "House"),
            ProfileAnalysis::measured(2500.0, 0.60, 9.0),
        ),
        synth_profile(
            ProfileSpec::new("iso5", "8A", 126.0, 0.75, "Tech House"),
            ProfileAnalysis::measured(2800.0, 0.55, 10.0),
        ),
        synth_profile(
            ProfileSpec::new("iso6", "8A", 126.0, 0.70, "Tech House"),
            ProfileAnalysis::measured(2600.0, 0.58, 8.0),
        ),
        synth_profile(
            ProfileSpec::new("iso7", "8A", 126.0, 0.50, "House"),
            ProfileAnalysis::measured(2100.0, 0.68, 7.5),
        ),
        synth_profile(
            ProfileSpec::new("iso8", "8A", 126.0, 0.35, "Deep House"),
            ProfileAnalysis::measured(1850.0, 0.71, 6.5),
        ),
    ])
}

fn pool_realistic_club() -> HashMap<String, TrackProfile> {
    build_pool(vec![
        // Deep House cluster (warmup)
        synth_profile(
            ProfileSpec::new("rc01", "6A", 122.0, 0.30, "Deep House"),
            ProfileAnalysis::measured(1600.0, 0.75, 5.0),
        ),
        synth_profile(
            ProfileSpec::new("rc02", "7A", 122.5, 0.35, "Deep House"),
            ProfileAnalysis::measured(1700.0, 0.73, 5.5),
        ),
        synth_profile(
            ProfileSpec::new("rc03", "7A", 123.0, 0.38, "Deep House"),
            ProfileAnalysis::measured(1750.0, 0.70, 6.0),
        ),
        synth_profile(
            ProfileSpec::new("rc04", "8A", 123.5, 0.42, "Deep House"),
            ProfileAnalysis::measured(1800.0, 0.68, 6.5),
        ),
        // House transition
        synth_profile(
            ProfileSpec::new("rc05", "8A", 124.0, 0.48, "House"),
            ProfileAnalysis::measured(2000.0, 0.65, 7.0),
        ),
        synth_profile(
            ProfileSpec::new("rc06", "9A", 124.5, 0.52, "House"),
            ProfileAnalysis::measured(2100.0, 0.63, 7.5),
        ),
        synth_profile(
            ProfileSpec::new("rc07", "9A", 125.0, 0.55, "House"),
            ProfileAnalysis::measured(2200.0, 0.60, 8.0),
        ),
        synth_profile(
            ProfileSpec::new("rc08", "10A", 125.5, 0.60, "House"),
            ProfileAnalysis::measured(2300.0, 0.58, 8.5),
        ),
        // Tech House build
        synth_profile(
            ProfileSpec::new("rc09", "10A", 126.0, 0.63, "Tech House"),
            ProfileAnalysis::measured(2400.0, 0.55, 9.0),
        ),
        synth_profile(
            ProfileSpec::new("rc10", "11A", 126.5, 0.67, "Tech House"),
            ProfileAnalysis::measured(2500.0, 0.53, 9.5),
        ),
        synth_profile(
            ProfileSpec::new("rc11", "11A", 127.0, 0.70, "Tech House"),
            ProfileAnalysis::measured(2600.0, 0.50, 10.0),
        ),
        synth_profile(
            ProfileSpec::new("rc12", "12A", 127.5, 0.75, "Tech House"),
            ProfileAnalysis::measured(2700.0, 0.48, 10.5),
        ),
        // Peak (Techno)
        synth_profile(
            ProfileSpec::new("rc13", "12A", 128.0, 0.80, "Techno"),
            ProfileAnalysis::measured(2800.0, 0.45, 11.0),
        ),
        synth_profile(
            ProfileSpec::new("rc14", "1A", 128.5, 0.82, "Techno"),
            ProfileAnalysis::measured(2900.0, 0.43, 3.5),
        ),
        synth_profile(
            ProfileSpec::new("rc15", "1A", 129.0, 0.85, "Techno"),
            ProfileAnalysis::measured(3000.0, 0.40, 3.0),
        ),
        synth_profile(
            ProfileSpec::new("rc16", "2A", 128.5, 0.80, "Techno"),
            ProfileAnalysis::measured(2850.0, 0.42, 4.0),
        ),
        // Release
        synth_profile(
            ProfileSpec::new("rc17", "2A", 127.0, 0.65, "Tech House"),
            ProfileAnalysis::measured(2500.0, 0.55, 8.0),
        ),
        synth_profile(
            ProfileSpec::new("rc18", "1A", 126.0, 0.55, "House"),
            ProfileAnalysis::measured(2200.0, 0.60, 7.0),
        ),
        synth_profile(
            ProfileSpec::new("rc19", "12A", 125.0, 0.45, "House"),
            ProfileAnalysis::measured(2000.0, 0.65, 6.0),
        ),
        synth_profile(
            ProfileSpec::new("rc20", "11A", 124.0, 0.35, "Deep House"),
            ProfileAnalysis::measured(1800.0, 0.70, 5.5),
        ),
    ])
}
#[test]
fn planning_sequence_eval_camelot_walk_greedy() {
    let pool = pool_camelot_walk();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        8,
    )
    .unwrap();
    let plan = build_candidate_plan(
        &pool,
        "cw1",
        sequence_policy(
            8,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            HarmonicMixingStyle::Conservative,
        ),
        0,
    );
    assert_eq!(plan.ordered_ids.len(), 8, "should use all 8 tracks");

    let metrics = compute_metrics(&plan, &phases);
    assert_quality_gates(&metrics, "camelot_walk_greedy");

    assert!(
        metrics.harmonic_coherence >= 0.85,
        "camelot walk should have ≥85% harmonic coherence, got {:.2}",
        metrics.harmonic_coherence,
    );
}

#[test]
fn planning_sequence_eval_camelot_walk_beam() {
    let pool = pool_camelot_walk();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        8,
    )
    .unwrap();
    let plans = build_candidate_plan_beam(
        &pool,
        "cw1",
        sequence_policy(
            8,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            HarmonicMixingStyle::Conservative,
        ),
        3,
    );
    assert!(
        !plans.is_empty(),
        "beam search should produce at least one plan"
    );

    let best = &plans[0];
    let metrics = compute_metrics(best, &phases);
    assert_quality_gates(&metrics, "camelot_walk_beam");
}

#[test]
fn planning_sequence_eval_adversarial_degrades_gracefully() {
    let pool = pool_adversarial();
    let phases =
        resolve_energy_curve(Some(&EnergyCurve::Preset(EnergyCurvePreset::FlatEnergy)), 6).unwrap();
    let plan = build_candidate_plan(
        &pool,
        "adv1",
        SequencePolicy {
            bpm_drift_pct: 12.0,
            ..sequence_policy(
                6,
                &phases,
                &priority_weights(SequencingPriority::Balanced),
                HarmonicMixingStyle::Balanced,
            )
        },
        0,
    );
    assert_eq!(plan.ordered_ids.len(), 6, "should use all 6 tracks");

    let metrics = compute_metrics(&plan, &phases);
    assert!(
        metrics.mean_composite >= 0.15,
        "[adversarial] mean composite {:.3} should be ≥0.15 even in worst case",
        metrics.mean_composite,
    );
    assert!(
        metrics.min_composite >= 0.05,
        "[adversarial] min composite {:.3} should be ≥0.05",
        metrics.min_composite,
    );
}

#[test]
fn planning_sequence_eval_iso_key_bpm_differentiates_on_secondary_axes() {
    let pool = pool_iso_key_bpm();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        8,
    )
    .unwrap();
    let plan = build_candidate_plan(
        &pool,
        "iso1",
        sequence_policy(
            8,
            &phases,
            &priority_weights(SequencingPriority::Energy),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );
    assert_eq!(plan.ordered_ids.len(), 8);

    let metrics = compute_metrics(&plan, &phases);
    assert_quality_gates(&metrics, "iso_key_bpm");

    assert!(
        metrics.harmonic_coherence >= 1.0 - f64::EPSILON,
        "iso pool should have 100% harmonic coherence",
    );
}

#[test]
fn planning_sequence_eval_realistic_club_greedy() {
    let pool = pool_realistic_club();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        16,
    )
    .unwrap();
    let plan = build_candidate_plan(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );

    let metrics = compute_metrics(&plan, &phases);
    assert_quality_gates(&metrics, "realistic_club_greedy");
}

#[test]
fn planning_sequence_eval_realistic_club_beam() {
    let pool = pool_realistic_club();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        16,
    )
    .unwrap();
    let plans = build_candidate_plan_beam(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        5,
    );
    assert!(!plans.is_empty());

    let best = &plans[0];
    let metrics = compute_metrics(best, &phases);
    assert_quality_gates(&metrics, "realistic_club_beam");
}

#[test]
fn planning_sequence_beam_is_at_least_as_good_as_greedy() {
    let pool = pool_realistic_club();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        16,
    )
    .unwrap();

    let greedy = build_candidate_plan(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );
    let greedy_mean = compute_metrics(&greedy, &phases).mean_composite;

    let beam_plans = build_candidate_plan_beam(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        5,
    );
    let beam_best_mean = beam_plans
        .iter()
        .map(|p| compute_metrics(p, &phases).mean_composite)
        .fold(0.0_f64, f64::max);

    assert!(
        beam_best_mean >= greedy_mean - 0.01,
        "beam best mean {beam_best_mean:.3} should be ≥ greedy mean {greedy_mean:.3} - 0.01",
    );
}

#[test]
fn planning_sequence_eval_harmonic_priority_improves_key_scores() {
    let pool = pool_realistic_club();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        16,
    )
    .unwrap();

    let balanced = build_candidate_plan(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );
    let harmonic = build_candidate_plan(
        &pool,
        "rc01",
        sequence_policy(
            16,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );

    let balanced_key_mean = balanced
        .transitions
        .iter()
        .map(|t| t.scores.key.value)
        .sum::<f64>()
        / balanced.transitions.len() as f64;
    let harmonic_key_mean = harmonic
        .transitions
        .iter()
        .map(|t| t.scores.key.value)
        .sum::<f64>()
        / harmonic.transitions.len() as f64;

    assert!(
        harmonic_key_mean >= balanced_key_mean - 0.05,
        "harmonic priority key mean {harmonic_key_mean:.3} should be ≥ balanced {balanced_key_mean:.3} - 0.05",
    );
}

#[test]
fn planning_sequence_is_deterministic_for_ids_and_scores() {
    let pool = pool_camelot_walk();
    let phases = resolve_energy_curve(
        Some(&EnergyCurve::Preset(
            EnergyCurvePreset::WarmupBuildPeakRelease,
        )),
        8,
    )
    .unwrap();

    let plan_a = build_candidate_plan(
        &pool,
        "cw1",
        sequence_policy(
            8,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );
    let plan_b = build_candidate_plan(
        &pool,
        "cw1",
        sequence_policy(
            8,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            HarmonicMixingStyle::Balanced,
        ),
        0,
    );

    assert_eq!(
        plan_a.ordered_ids, plan_b.ordered_ids,
        "same inputs must produce identical track ordering",
    );
    let composites_a: Vec<f64> = plan_a
        .transitions
        .iter()
        .map(|t| t.scores.composite)
        .collect();
    let composites_b: Vec<f64> = plan_b
        .transitions
        .iter()
        .map(|t| t.scores.composite)
        .collect();
    assert_eq!(
        composites_a, composites_b,
        "same inputs must produce identical composites"
    );
}
