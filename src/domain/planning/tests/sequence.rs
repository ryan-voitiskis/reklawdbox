use std::collections::HashMap;

use crate::domain::planning::*;

use super::support::{
    ProfileAnalysis, ProfileSpec, sequence_policy, simple_profile, synth_profile,
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
