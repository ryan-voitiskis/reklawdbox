//! Scoring evaluation harness — synthetic-pool quality gates.
//!
//! Runs deterministic tests against synthetic track pools (no DB required)
//! to catch scoring regressions. Follows the threshold-based eval pattern
//! from `eval_routing.rs`.

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::tools::params::*;
    use crate::tools::scoring::*;

    const MEAN_COMPOSITE_MIN: f64 = 0.65;
    const MIN_COMPOSITE_MIN: f64 = 0.30;
    const COMPOSITE_VARIANCE_MAX: f64 = 0.08;
    const HARMONIC_COHERENCE_MIN: f64 = 0.50;
    const ENERGY_FIDELITY_MIN: f64 = 0.40;
    const MAX_PITCH_ADJUSTMENT: f64 = 8.0;

    #[allow(clippy::too_many_arguments)]
    fn synth_profile(
        id: &str,
        key: &str,
        bpm: f64,
        energy: f64,
        genre: &str,
        brightness: Option<f64>,
        rhythm: Option<f64>,
        loudness_range: Option<f64>,
    ) -> TrackProfile {
        TrackProfile {
            track: crate::types::Track {
                id: id.to_string(),
                title: id.to_string(),
                artist: "Eval".to_string(),
                album: String::new(),
                genre: genre.to_string(),
                key: key.to_string(),
                bpm,
                rating: 0,
                comments: String::new(),
                color: String::new(),
                color_code: 0,
                label: String::new(),
                remixer: String::new(),
                year: 2025,
                length: 360,
                file_path: format!("/eval/{id}.flac"),
                play_count: 0,
                bit_rate: 1411,
                sample_rate: 44100,
                file_kind: crate::types::FileKind::Flac,
                date_added: String::new(),
                position: None,
                played_at: None,
            },
            camelot_key: parse_camelot_key(key),
            key_display: key.to_string(),
            bpm,
            energy,
            brightness,
            rhythm_regularity: rhythm,
            loudness_range,
            canonical_genre: Some(genre.to_string()),
            genre_family: genre_family_for(genre),
            timbral: None,
        }
    }

    fn simple_profile(id: &str, key: &str, bpm: f64, energy: f64, genre: &str) -> TrackProfile {
        synth_profile(id, key, bpm, energy, genre, None, None, None)
    }

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

    #[test]
    fn eval_camelot_walk_greedy() {
        let pool = pool_camelot_walk();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            8,
        )
        .unwrap();
        let plan = build_candidate_plan(
            &pool,
            "cw1",
            8,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            0,
            true,
            Some(HarmonicMixingStyle::Conservative),
            6.0,
            None,
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
    fn eval_camelot_walk_beam() {
        let pool = pool_camelot_walk();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            8,
        )
        .unwrap();
        let plans = build_candidate_plan_beam(
            &pool,
            "cw1",
            8,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            3,
            true,
            Some(HarmonicMixingStyle::Conservative),
            6.0,
            None,
        );
        assert!(
            !plans.is_empty(),
            "beam search should produce at least one plan"
        );

        let best = &plans[0];
        let metrics = compute_metrics(best, &phases);
        assert_quality_gates(&metrics, "camelot_walk_beam");
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

    #[test]
    fn eval_adversarial_degrades_gracefully() {
        let pool = pool_adversarial();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(EnergyCurvePreset::FlatEnergy)),
            6,
        )
        .unwrap();
        let plan = build_candidate_plan(
            &pool,
            "adv1",
            6,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            12.0,
            None,
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

    fn pool_iso_key_bpm() -> HashMap<String, TrackProfile> {
        build_pool(vec![
            synth_profile(
                "iso1",
                "8A",
                126.0,
                0.30,
                "Deep House",
                Some(1800.0),
                Some(0.7),
                Some(6.0),
            ),
            synth_profile(
                "iso2",
                "8A",
                126.0,
                0.40,
                "Deep House",
                Some(1900.0),
                Some(0.72),
                Some(7.0),
            ),
            synth_profile(
                "iso3",
                "8A",
                126.0,
                0.55,
                "House",
                Some(2200.0),
                Some(0.65),
                Some(8.5),
            ),
            synth_profile(
                "iso4",
                "8A",
                126.0,
                0.65,
                "House",
                Some(2500.0),
                Some(0.60),
                Some(9.0),
            ),
            synth_profile(
                "iso5",
                "8A",
                126.0,
                0.75,
                "Tech House",
                Some(2800.0),
                Some(0.55),
                Some(10.0),
            ),
            synth_profile(
                "iso6",
                "8A",
                126.0,
                0.70,
                "Tech House",
                Some(2600.0),
                Some(0.58),
                Some(8.0),
            ),
            synth_profile(
                "iso7",
                "8A",
                126.0,
                0.50,
                "House",
                Some(2100.0),
                Some(0.68),
                Some(7.5),
            ),
            synth_profile(
                "iso8",
                "8A",
                126.0,
                0.35,
                "Deep House",
                Some(1850.0),
                Some(0.71),
                Some(6.5),
            ),
        ])
    }

    #[test]
    fn eval_iso_key_bpm_differentiates_on_secondary_axes() {
        let pool = pool_iso_key_bpm();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            8,
        )
        .unwrap();
        let plan = build_candidate_plan(
            &pool,
            "iso1",
            8,
            &phases,
            &priority_weights(SequencingPriority::Energy),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        assert_eq!(plan.ordered_ids.len(), 8);

        let metrics = compute_metrics(&plan, &phases);
        assert_quality_gates(&metrics, "iso_key_bpm");

        assert!(
            metrics.harmonic_coherence >= 1.0 - f64::EPSILON,
            "iso pool should have 100% harmonic coherence",
        );
    }

    fn pool_realistic_club() -> HashMap<String, TrackProfile> {
        build_pool(vec![
            // Deep House cluster (warmup)
            synth_profile(
                "rc01",
                "6A",
                122.0,
                0.30,
                "Deep House",
                Some(1600.0),
                Some(0.75),
                Some(5.0),
            ),
            synth_profile(
                "rc02",
                "7A",
                122.5,
                0.35,
                "Deep House",
                Some(1700.0),
                Some(0.73),
                Some(5.5),
            ),
            synth_profile(
                "rc03",
                "7A",
                123.0,
                0.38,
                "Deep House",
                Some(1750.0),
                Some(0.70),
                Some(6.0),
            ),
            synth_profile(
                "rc04",
                "8A",
                123.5,
                0.42,
                "Deep House",
                Some(1800.0),
                Some(0.68),
                Some(6.5),
            ),
            // House transition
            synth_profile(
                "rc05",
                "8A",
                124.0,
                0.48,
                "House",
                Some(2000.0),
                Some(0.65),
                Some(7.0),
            ),
            synth_profile(
                "rc06",
                "9A",
                124.5,
                0.52,
                "House",
                Some(2100.0),
                Some(0.63),
                Some(7.5),
            ),
            synth_profile(
                "rc07",
                "9A",
                125.0,
                0.55,
                "House",
                Some(2200.0),
                Some(0.60),
                Some(8.0),
            ),
            synth_profile(
                "rc08",
                "10A",
                125.5,
                0.60,
                "House",
                Some(2300.0),
                Some(0.58),
                Some(8.5),
            ),
            // Tech House build
            synth_profile(
                "rc09",
                "10A",
                126.0,
                0.63,
                "Tech House",
                Some(2400.0),
                Some(0.55),
                Some(9.0),
            ),
            synth_profile(
                "rc10",
                "11A",
                126.5,
                0.67,
                "Tech House",
                Some(2500.0),
                Some(0.53),
                Some(9.5),
            ),
            synth_profile(
                "rc11",
                "11A",
                127.0,
                0.70,
                "Tech House",
                Some(2600.0),
                Some(0.50),
                Some(10.0),
            ),
            synth_profile(
                "rc12",
                "12A",
                127.5,
                0.75,
                "Tech House",
                Some(2700.0),
                Some(0.48),
                Some(10.5),
            ),
            // Peak (Techno)
            synth_profile(
                "rc13",
                "12A",
                128.0,
                0.80,
                "Techno",
                Some(2800.0),
                Some(0.45),
                Some(11.0),
            ),
            synth_profile(
                "rc14",
                "1A",
                128.5,
                0.82,
                "Techno",
                Some(2900.0),
                Some(0.43),
                Some(3.5),
            ),
            synth_profile(
                "rc15",
                "1A",
                129.0,
                0.85,
                "Techno",
                Some(3000.0),
                Some(0.40),
                Some(3.0),
            ),
            synth_profile(
                "rc16",
                "2A",
                128.5,
                0.80,
                "Techno",
                Some(2850.0),
                Some(0.42),
                Some(4.0),
            ),
            // Release
            synth_profile(
                "rc17",
                "2A",
                127.0,
                0.65,
                "Tech House",
                Some(2500.0),
                Some(0.55),
                Some(8.0),
            ),
            synth_profile(
                "rc18",
                "1A",
                126.0,
                0.55,
                "House",
                Some(2200.0),
                Some(0.60),
                Some(7.0),
            ),
            synth_profile(
                "rc19",
                "12A",
                125.0,
                0.45,
                "House",
                Some(2000.0),
                Some(0.65),
                Some(6.0),
            ),
            synth_profile(
                "rc20",
                "11A",
                124.0,
                0.35,
                "Deep House",
                Some(1800.0),
                Some(0.70),
                Some(5.5),
            ),
        ])
    }

    #[test]
    fn eval_realistic_club_greedy() {
        let pool = pool_realistic_club();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            16,
        )
        .unwrap();
        let plan = build_candidate_plan(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );

        let metrics = compute_metrics(&plan, &phases);
        assert_quality_gates(&metrics, "realistic_club_greedy");
    }

    #[test]
    fn eval_realistic_club_beam() {
        let pool = pool_realistic_club();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            16,
        )
        .unwrap();
        let plans = build_candidate_plan_beam(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            5,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        assert!(!plans.is_empty());

        let best = &plans[0];
        let metrics = compute_metrics(best, &phases);
        assert_quality_gates(&metrics, "realistic_club_beam");
    }

    #[test]
    fn eval_beam_at_least_as_good_as_greedy() {
        let pool = pool_realistic_club();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            16,
        )
        .unwrap();

        let greedy = build_candidate_plan(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        let greedy_mean = compute_metrics(&greedy, &phases).mean_composite;

        let beam_plans = build_candidate_plan_beam(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            5,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
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
    fn eval_harmonic_priority_improves_key_scores() {
        let pool = pool_realistic_club();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            16,
        )
        .unwrap();

        let balanced = build_candidate_plan(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        let harmonic = build_candidate_plan(
            &pool,
            "rc01",
            16,
            &phases,
            &priority_weights(SequencingPriority::Harmonic),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
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
    fn eval_deterministic_output() {
        let pool = pool_camelot_walk();
        let phases = resolve_energy_curve(
            Some(&EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            8,
        )
        .unwrap();

        let plan_a = build_candidate_plan(
            &pool,
            "cw1",
            8,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        let plan_b = build_candidate_plan(
            &pool,
            "cw1",
            8,
            &phases,
            &priority_weights(SequencingPriority::Balanced),
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
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

    #[test]
    fn eval_bpm_curve_monotonic() {
        let base_bpm = 128.0;
        let mut prev_value = 1.0;
        for pct_x10 in 1..=200 {
            let pct = pct_x10 as f64 / 10.0; // 0.1% to 20.0%
            let target = base_bpm * (1.0 + pct / 100.0);
            let score = score_bpm_axis(base_bpm, target);
            assert!(
                score.value <= prev_value + f64::EPSILON,
                "BPM curve should be monotonically decreasing: at {pct}% got {} > prev {}",
                score.value,
                prev_value,
            );
            prev_value = score.value;
        }
    }

    #[test]
    fn eval_conservative_penalty_stronger_than_balanced() {
        let from = simple_profile("pen-from", "8A", 128.0, 0.7, "House");
        let to = simple_profile("pen-to", "2A", 128.0, 0.7, "House"); // Clash: key=0.1

        let conservative = score_transition_profiles(
            &from,
            &to,
            Some(EnergyPhase::Peak),
            Some(EnergyPhase::Peak),
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
            &ScoringContext::default(),
            None,
        );
        let balanced = score_transition_profiles(
            &from,
            &to,
            Some(EnergyPhase::Peak),
            Some(EnergyPhase::Peak),
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Balanced),
            &ScoringContext::default(),
            None,
        );

        assert!(
            conservative.composite < balanced.composite,
            "conservative ({:.3}) should penalize harder than balanced ({:.3})",
            conservative.composite,
            balanced.composite,
        );
    }

    #[test]
    fn eval_clean_transition_has_no_adjustments() {
        let from = simple_profile("adj-from", "8A", 128.0, 0.5, "House");
        let to = simple_profile("adj-to", "9A", 128.5, 0.55, "House");

        let scores = score_transition_profiles(
            &from,
            &to,
            Some(EnergyPhase::Build),
            Some(EnergyPhase::Build),
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Balanced),
            &ScoringContext::default(),
            None,
        );
        assert!(
            scores.adjustments.is_empty(),
            "clean transition should have no adjustments, got {:?}",
            scores
                .adjustments
                .iter()
                .map(|a| a.kind)
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn eval_harmonic_gate_produces_adjustment() {
        let from = simple_profile("hg-from", "8A", 128.0, 0.7, "House");
        let to = simple_profile("hg-to", "2A", 128.0, 0.7, "House"); // Clash

        let scores = score_transition_profiles(
            &from,
            &to,
            Some(EnergyPhase::Peak),
            Some(EnergyPhase::Peak),
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
            &ScoringContext::default(),
            None,
        );
        assert!(
            scores.adjustments.iter().any(|a| a.kind == "harmonic_gate"),
            "clash with conservative should produce harmonic_gate adjustment",
        );

        let adj = scores
            .adjustments
            .iter()
            .find(|a| a.kind == "harmonic_gate")
            .unwrap();
        assert!(adj.delta < 0.0, "harmonic_gate delta should be negative");
        assert!(
            adj.composite_without > scores.composite,
            "composite_without should exceed final composite"
        );
    }

    #[test]
    fn eval_genre_streak_produces_adjustment() {
        let from = simple_profile("gs-from", "8A", 128.0, 0.5, "House");
        let to = simple_profile("gs-to", "9A", 128.0, 0.55, "House");

        let scores = score_transition_profiles(
            &from,
            &to,
            None,
            None,
            &priority_weights(SequencingPriority::Balanced),
            true,
            None,
            &ScoringContext {
                genre_run_length: 2,
            },
            None,
        );
        assert!(
            scores.adjustments.iter().any(|a| a.kind == "genre_streak"),
            "same-family transition with run_length=2 should produce genre_streak adjustment",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn synth_profile_timbral(
        id: &str,
        key: &str,
        bpm: f64,
        energy: f64,
        genre: &str,
        brightness: Option<f64>,
        rhythm: Option<f64>,
        loudness_range: Option<f64>,
        mfcc_mean: Option<Vec<f64>>,
        mfcc_std: Option<Vec<f64>>,
        spectral_contrast_mean: Option<Vec<f64>>,
        spectral_centroid_cv: Option<f64>,
        dissonance_mean: Option<f64>,
    ) -> TrackProfile {
        let mut p = synth_profile(
            id,
            key,
            bpm,
            energy,
            genre,
            brightness,
            rhythm,
            loudness_range,
        );
        p.timbral = match (
            mfcc_mean,
            mfcc_std,
            spectral_contrast_mean,
            spectral_centroid_cv,
            dissonance_mean,
        ) {
            (Some(mm), Some(ms), Some(sc), Some(cv), Some(dm)) => Some(TimbralFeatures {
                mfcc_mean: mm,
                mfcc_std: ms,
                spectral_contrast_mean: sc,
                spectral_centroid_cv: cv,
                dissonance_mean: dm,
            }),
            _ => None,
        };
        p
    }

    #[test]
    fn eval_pool_scoring_is_symmetric() {
        let a = simple_profile("sym-a", "8A", 126.0, 0.6, "House");
        let b = simple_profile("sym-b", "10A", 128.0, 0.7, "Tech House");

        let ab = score_pool_compatibility_pair(
            &a,
            &b,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );
        let ba = score_pool_compatibility_pair(
            &b,
            &a,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            (ab.composite - ba.composite).abs() < 1e-10,
            "pool score should be symmetric: A→B={:.6}, B→A={:.6}",
            ab.composite,
            ba.composite,
        );

        assert!(
            (ab.bpm.value - ba.bpm.value).abs() < 1e-10,
            "BPM axis should be symmetric",
        );
        assert!(
            (ab.energy.value - ba.energy.value).abs() < 1e-10,
            "energy axis should be symmetric",
        );
        assert!(
            (ab.key.value - ba.key.value).abs() < 1e-10,
            "key axis should be symmetric",
        );
    }

    #[test]
    fn eval_pool_planted_cluster_separation() {
        // 6 tight tracks (same key neighborhood, close BPM, same genre family)
        let tight = vec![
            simple_profile("tight1", "8A", 126.0, 0.55, "Deep House"),
            simple_profile("tight2", "9A", 126.5, 0.58, "Deep House"),
            simple_profile("tight3", "8A", 127.0, 0.60, "House"),
            simple_profile("tight4", "9A", 126.0, 0.57, "House"),
            simple_profile("tight5", "8B", 126.5, 0.56, "Deep House"),
            simple_profile("tight6", "7A", 127.0, 0.59, "House"),
        ];

        // 6 distractor tracks (different key, different BPM range, different genre)
        let distractors = vec![
            simple_profile("dist1", "2A", 140.0, 0.85, "Techno"),
            simple_profile("dist2", "3B", 138.0, 0.80, "Techno"),
            simple_profile("dist3", "5A", 110.0, 0.30, "Ambient"),
            simple_profile("dist4", "11B", 145.0, 0.90, "Drum & Bass"),
            simple_profile("dist5", "6B", 135.0, 0.75, "Trance"),
            simple_profile("dist6", "1A", 142.0, 0.88, "Techno"),
        ];

        let tight_refs: Vec<&TrackProfile> = tight.iter().collect();
        let tight_cohesion = compute_pool_cohesion(
            &tight_refs,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            tight_cohesion.mean_pairwise >= 0.65,
            "tight cluster mean pairwise {:.3} should be >= 0.65",
            tight_cohesion.mean_pairwise,
        );

        let mut cross_scores = Vec::new();
        for t in &tight {
            for d in &distractors {
                let score = score_pool_compatibility_pair(
                    t,
                    d,
                    true,
                    127.0,
                    &pool_weights(PoolPreset::Balanced),
                    None,
                );
                cross_scores.push(score.composite);
            }
        }
        let cross_mean = cross_scores.iter().sum::<f64>() / cross_scores.len() as f64;

        assert!(
            cross_mean < tight_cohesion.mean_pairwise - 0.15,
            "cross-cluster mean {cross_mean:.3} should be substantially lower than tight cluster {:.3}",
            tight_cohesion.mean_pairwise,
        );
    }

    #[test]
    fn eval_expand_pool_greedy_selects_compatible_tracks() {
        // 4 seeds in a tight cluster
        let seeds: Vec<TrackProfile> = vec![
            simple_profile("seed1", "8A", 126.0, 0.55, "House"),
            simple_profile("seed2", "9A", 126.5, 0.58, "House"),
            simple_profile("seed3", "8A", 127.0, 0.60, "Deep House"),
            simple_profile("seed4", "7A", 126.0, 0.53, "Deep House"),
        ];

        // 4 good candidates (compatible)
        let good = vec![
            simple_profile("good1", "8B", 126.5, 0.57, "House"),
            simple_profile("good2", "9A", 127.0, 0.62, "House"),
            simple_profile("good3", "10A", 126.0, 0.56, "Deep House"),
            simple_profile("good4", "7A", 127.0, 0.59, "House"),
        ];

        // 6 bad candidates (incompatible)
        let bad = vec![
            simple_profile("bad1", "2A", 140.0, 0.85, "Techno"),
            simple_profile("bad2", "5B", 110.0, 0.25, "Ambient"),
            simple_profile("bad3", "11A", 145.0, 0.90, "Drum & Bass"),
            simple_profile("bad4", "3A", 135.0, 0.80, "Trance"),
            simple_profile("bad5", "6B", 142.0, 0.88, "Techno"),
            simple_profile("bad6", "1B", 105.0, 0.20, "Ambient"),
        ];

        let mut all_candidates: Vec<TrackProfile> = good;
        all_candidates.extend(bad);

        let ref_bpm = 126.5;

        let mut pool = seeds;
        let mut remaining = all_candidates;
        let mut selected_ids = Vec::new();

        for _ in 0..4 {
            if remaining.is_empty() {
                break;
            }

            let pool_refs: Vec<&TrackProfile> = pool.iter().collect();
            let mut best_idx = 0;
            let mut best_min = f64::NEG_INFINITY;

            for (i, candidate) in remaining.iter().enumerate() {
                let result = score_candidate_vs_pool(
                    candidate,
                    &pool_refs,
                    true,
                    ref_bpm,
                    &pool_weights(PoolPreset::Balanced),
                    None,
                );
                if result.min_score > best_min {
                    best_min = result.min_score;
                    best_idx = i;
                }
            }

            let chosen = remaining.swap_remove(best_idx);
            selected_ids.push(chosen.track.id.clone());
            pool.push(chosen);
        }

        let good_ids: HashSet<&str> = ["good1", "good2", "good3", "good4"]
            .iter()
            .copied()
            .collect();
        let selected_good = selected_ids
            .iter()
            .filter(|id| good_ids.contains(id.as_str()))
            .count();

        assert!(
            selected_good >= 3,
            "greedy expansion should select at least 3/4 good candidates, got {selected_good}/4: {selected_ids:?}",
        );
    }

    #[test]
    fn eval_describe_pool_optimal_bpm_sweep() {
        // 5 tracks at 124-132 BPM, all 8A key
        let profiles = vec![
            simple_profile("bpm1", "8A", 124.0, 0.5, "House"),
            simple_profile("bpm2", "8A", 126.0, 0.55, "House"),
            simple_profile("bpm3", "8A", 128.0, 0.6, "House"),
            simple_profile("bpm4", "8A", 130.0, 0.65, "House"),
            simple_profile("bpm5", "8A", 132.0, 0.7, "House"),
        ];

        let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

        let result = super::super::pool_handlers::sweep_optimal_reference_bpm(&profiles, &bpms);

        assert!(
            result.is_some(),
            "should find an optimal reference BPM for 124-132 range",
        );

        let (optimal_bpm, optimal_stability) = result.unwrap();
        assert!(
            (optimal_bpm - 128.0).abs() <= 2.0,
            "optimal BPM {optimal_bpm:.1} should be near median 128.0 (±2.0)",
        );
        assert!(
            optimal_stability > 0.5,
            "optimal stability {optimal_stability:.3} should be reasonable",
        );
    }

    #[test]
    fn eval_pool_energy_axis_gaussian() {
        let same = score_pool_energy_axis(0.5, 0.5);
        assert!(
            (same.value - 1.0).abs() < 1e-10,
            "same energy should score 1.0, got {:.3}",
            same.value,
        );

        let close = score_pool_energy_axis(0.5, 0.55);
        assert!(
            close.value > 0.9,
            "5% energy delta should score > 0.9, got {:.3}",
            close.value,
        );

        let far = score_pool_energy_axis(0.3, 0.8);
        assert!(
            far.value < 0.1,
            "50% energy delta should score < 0.1, got {:.3}",
            far.value,
        );

        let ab = score_pool_energy_axis(0.3, 0.6);
        let ba = score_pool_energy_axis(0.6, 0.3);
        assert!(
            (ab.value - ba.value).abs() < 1e-10,
            "pool energy should be symmetric",
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn make_timbral_profile(
        id: &str,
        key: &str,
        bpm: f64,
        energy: f64,
        genre: &str,
        mfcc_mean: Vec<f64>,
        mfcc_std: Vec<f64>,
        spectral_contrast: Vec<f64>,
        centroid_cv: f64,
        dissonance: f64,
    ) -> TrackProfile {
        synth_profile_timbral(
            id,
            key,
            bpm,
            energy,
            genre,
            Some(2000.0),
            Some(0.6),
            Some(7.0),
            Some(mfcc_mean),
            Some(mfcc_std),
            Some(spectral_contrast),
            Some(centroid_cv),
            Some(dissonance),
        )
    }

    fn dummy_norm_stats(dims: usize) -> crate::store::TimbralNormStats {
        crate::store::TimbralNormStats {
            dims: vec![(0.0, 1.0); dims],
            sample_count: 100,
        }
    }

    #[test]
    fn eval_build_timbral_vector_requires_all_fields() {
        let full = make_timbral_profile(
            "tv1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![0.0; 13],
            vec![0.0; 13],
            vec![0.0; 6],
            0.5,
            0.3,
        );
        let vec = build_timbral_vector(&full);
        assert!(
            vec.is_some(),
            "should produce vector when all fields present"
        );
        assert_eq!(vec.unwrap().len(), 13 + 13 + 6 + 1 + 1, "expected 34 dims");

        let mut missing = full.clone();
        missing.timbral = None;
        assert!(
            build_timbral_vector(&missing).is_none(),
            "missing timbral should return None",
        );
    }

    #[test]
    fn eval_normalize_timbral_vector_zscore() {
        let stats = crate::store::TimbralNormStats {
            dims: vec![(10.0, 2.0), (20.0, 5.0), (30.0, 10.0)],
            sample_count: 50,
        };
        let raw = vec![12.0, 25.0, 30.0];
        let norm = normalize_timbral_vector(&raw, &stats);
        assert!(norm.is_some());
        let norm = norm.unwrap();
        assert!((norm[0] - 1.0).abs() < 1e-10, "(12-10)/2 = 1.0");
        assert!((norm[1] - 1.0).abs() < 1e-10, "(25-20)/5 = 1.0");
        assert!((norm[2] - 0.0).abs() < 1e-10, "(30-30)/10 = 0.0");
    }

    #[test]
    fn eval_normalize_timbral_vector_dimension_mismatch() {
        let stats = crate::store::TimbralNormStats {
            dims: vec![(0.0, 1.0); 3],
            sample_count: 50,
        };
        assert!(
            normalize_timbral_vector(&[1.0; 5], &stats).is_none(),
            "longer raw should return None",
        );
        assert!(
            normalize_timbral_vector(&[1.0; 2], &stats).is_none(),
            "shorter raw should return None",
        );
        assert!(
            normalize_timbral_vector(&[1.0; 3], &stats).is_some(),
            "matching dims should succeed",
        );
    }

    #[test]
    fn eval_pool_timbral_axis_identical_vectors() {
        let dims = 34;
        let stats = dummy_norm_stats(dims);
        let a = make_timbral_profile(
            "ta1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );
        let b = a.clone();
        let score = score_pool_timbral_axis(&a, &b, &stats);
        assert!(score.is_some(), "identical profiles should produce a score");
        assert!(
            (score.unwrap().value - 1.0).abs() < 1e-10,
            "identical vectors should score 1.0",
        );
    }

    #[test]
    fn eval_pool_timbral_axis_distant_vectors() {
        let dims = 34;
        let stats = dummy_norm_stats(dims);
        let a = make_timbral_profile(
            "td1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![0.0; 13],
            vec![0.0; 13],
            vec![0.0; 6],
            0.0,
            0.0,
        );
        let b = make_timbral_profile(
            "td2",
            "8A",
            126.0,
            0.5,
            "House",
            vec![5.0; 13],
            vec![5.0; 13],
            vec![5.0; 6],
            5.0,
            5.0,
        );
        let score = score_pool_timbral_axis(&a, &b, &stats);
        assert!(score.is_some());
        assert!(
            score.unwrap().value < 0.3,
            "distant vectors should score low",
        );
    }

    #[test]
    fn eval_pool_timbral_axis_missing_data_returns_none() {
        let dims = 34;
        let stats = dummy_norm_stats(dims);
        let with_data = make_timbral_profile(
            "tm1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );
        let without_data = simple_profile("tm2", "8A", 126.0, 0.5, "House");

        assert!(
            score_pool_timbral_axis(&with_data, &without_data, &stats).is_none(),
            "missing timbral data should return None",
        );
    }

    #[test]
    fn eval_pool_timbral_axis_dimension_mismatch_returns_none() {
        let stats = dummy_norm_stats(34);
        let a = make_timbral_profile(
            "tdm1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 2],
            0.4,
            0.2,
        );
        let b = a.clone();
        assert!(
            score_pool_timbral_axis(&a, &b, &stats).is_none(),
            "dimension mismatch with norm stats should return None",
        );
    }

    #[test]
    fn eval_pool_composite_with_vs_without_timbral() {
        let dims = 34;
        let stats = dummy_norm_stats(dims);

        let a_timbral = make_timbral_profile(
            "ct1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );
        let b_timbral = make_timbral_profile(
            "ct2",
            "9A",
            126.5,
            0.55,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );

        let with_timbral = score_pool_compatibility_pair(
            &a_timbral,
            &b_timbral,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            Some(&stats),
        );
        let without_timbral = score_pool_compatibility_pair(
            &a_timbral,
            &b_timbral,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            (with_timbral.composite - without_timbral.composite).abs() > 0.001,
            "timbral axis should affect composite: with={:.4} without={:.4}",
            with_timbral.composite,
            without_timbral.composite,
        );
        assert!(
            with_timbral.timbral.is_some(),
            "should have timbral score when stats provided",
        );
        assert!(
            without_timbral.timbral.is_none(),
            "should not have timbral score without stats",
        );
    }

    #[test]
    fn eval_pool_preset_timbral_vs_balanced() {
        let dims = 34;
        let stats = dummy_norm_stats(dims);

        let a = make_timbral_profile(
            "pt1",
            "8A",
            126.0,
            0.5,
            "House",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );
        let b = make_timbral_profile(
            "pt2",
            "2A",
            126.0,
            0.5,
            "Techno",
            vec![1.0; 13],
            vec![0.5; 13],
            vec![0.3; 6],
            0.4,
            0.2,
        );

        let balanced = score_pool_compatibility_pair(
            &a,
            &b,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            Some(&stats),
        );
        let timbral = score_pool_compatibility_pair(
            &a,
            &b,
            true,
            126.0,
            &pool_weights(PoolPreset::Timbral),
            Some(&stats),
        );

        assert!(
            timbral.composite > balanced.composite,
            "timbral preset should score higher for timbral-matched key-clashing pair: \
             timbral={:.3} balanced={:.3}",
            timbral.composite,
            balanced.composite,
        );
    }

    #[test]
    fn eval_expand_pool_stops_below_quality_threshold() {
        let seeds = vec![
            simple_profile("qs1", "8A", 126.0, 0.55, "Deep House"),
            simple_profile("qs2", "9A", 126.5, 0.58, "Deep House"),
        ];

        let candidates = vec![
            simple_profile("qc1", "2A", 140.0, 0.85, "Techno"),
            simple_profile("qc2", "5B", 145.0, 0.90, "Drum & Bass"),
            simple_profile("qc3", "11A", 110.0, 0.20, "Ambient"),
        ];

        let ref_bpm = 126.25;
        let quality_threshold = 0.4;
        let mut pool = seeds;
        let mut remaining = candidates;
        let mut added = 0;

        for _ in 0..3 {
            if remaining.is_empty() {
                break;
            }
            let pool_refs: Vec<&TrackProfile> = pool.iter().collect();
            let mut best_min = f64::NEG_INFINITY;
            let mut best_idx = 0;
            for (i, c) in remaining.iter().enumerate() {
                let result = score_candidate_vs_pool(
                    c,
                    &pool_refs,
                    true,
                    ref_bpm,
                    &pool_weights(PoolPreset::Balanced),
                    None,
                );
                if result.min_score > best_min {
                    best_min = result.min_score;
                    best_idx = i;
                }
            }
            if best_min < quality_threshold {
                break;
            }
            pool.push(remaining.swap_remove(best_idx));
            added += 1;
        }

        assert_eq!(
            added, 0,
            "should add zero tracks when all candidates score below quality threshold",
        );
    }

    #[test]
    fn eval_bpm_sweep_too_wide_returns_none() {
        let profiles = vec![
            simple_profile("wide1", "8A", 100.0, 0.5, "House"),
            simple_profile("wide2", "8A", 140.0, 0.5, "House"),
        ];
        let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

        let result = super::super::pool_handlers::sweep_optimal_reference_bpm(&profiles, &bpms);
        assert!(
            result.is_none(),
            "100-140 BPM spread should be too wide for any single reference",
        );
    }

    #[test]
    fn eval_bpm_sweep_tight_range_succeeds() {
        let profiles = vec![
            simple_profile("tight1", "8A", 126.0, 0.5, "House"),
            simple_profile("tight2", "8A", 127.0, 0.5, "House"),
            simple_profile("tight3", "8A", 128.0, 0.5, "House"),
        ];
        let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

        let result = super::super::pool_handlers::sweep_optimal_reference_bpm(&profiles, &bpms);
        assert!(result.is_some(), "126-128 range should find optimal BPM");

        let (optimal, stability) = result.unwrap();
        assert!(
            (optimal - 127.0).abs() <= 1.0,
            "optimal should be near center 127.0, got {optimal:.1}",
        );
        assert!(stability > 0.8, "tight range should have high stability");
    }

    #[test]
    fn eval_bpm_sweep_narrow_interval_found() {
        let profiles = vec![
            simple_profile("narrow1", "8A", 100.0, 0.5, "House"),
            simple_profile("narrow2", "8A", 112.0, 0.5, "House"),
        ];
        let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

        let result = super::super::pool_handlers::sweep_optimal_reference_bpm(&profiles, &bpms);
        assert!(
            result.is_some(),
            "100-112 BPM should find a valid reference (narrow interval ~105.7-105.9)",
        );

        let (optimal, _) = result.unwrap();
        assert!(
            (105.5..=106.1).contains(&optimal),
            "optimal should be near 105.8, got {optimal:.2}",
        );
    }

    #[test]
    fn eval_pool_cohesion_single_track() {
        let profiles = [simple_profile("single", "8A", 126.0, 0.5, "House")];
        let refs: Vec<&TrackProfile> = profiles.iter().collect();
        let result = compute_pool_cohesion(
            &refs,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            (result.mean_pairwise - 1.0).abs() < 1e-10,
            "single track should have mean_pairwise 1.0",
        );
        assert!(result.per_pair.is_empty(), "single track has no pairs");
        assert_eq!(
            result.medoid_id.as_deref(),
            Some("single"),
            "single track is its own medoid",
        );
    }

    #[test]
    fn eval_candidate_vs_empty_pool() {
        let candidate = simple_profile("c", "8A", 126.0, 0.5, "House");
        let result = score_candidate_vs_pool(
            &candidate,
            &[],
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );
        assert!(
            (result.mean_score - 0.0).abs() < 1e-10,
            "empty pool should give mean 0.0",
        );
        assert!(result.per_member.is_empty());
    }

    #[test]
    fn eval_pool_scoring_master_tempo_off_changes_key() {
        let a = simple_profile("mto-a", "8A", 126.0, 0.5, "House");
        let b = simple_profile("mto-b", "8A", 132.0, 0.5, "House");

        let mt_on = score_pool_compatibility_pair(
            &a,
            &b,
            true,
            129.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );
        assert!(
            mt_on.key.value > 0.9,
            "master_tempo ON, same key should score high: {:.3}",
            mt_on.key.value,
        );

        let mt_off = score_pool_compatibility_pair(
            &a,
            &b,
            false,
            129.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            (mt_on.key.value - mt_off.key.value).abs() > 0.01,
            "master_tempo OFF should change key scoring: on={:.3} off={:.3}",
            mt_on.key.value,
            mt_off.key.value,
        );
    }

    #[test]
    fn eval_pool_scoring_master_tempo_off_symmetric() {
        let a = simple_profile("mts-a", "8A", 124.0, 0.5, "House");
        let b = simple_profile("mts-b", "10A", 128.0, 0.55, "Deep House");

        let ab = score_pool_compatibility_pair(
            &a,
            &b,
            false,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );
        let ba = score_pool_compatibility_pair(
            &b,
            &a,
            false,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
        );

        assert!(
            (ab.composite - ba.composite).abs() < 0.01,
            "pool score should be symmetric with master_tempo off: A→B={:.6}, B→A={:.6}",
            ab.composite,
            ba.composite,
        );
    }

    #[test]
    fn eval_pool_genre_axis_unknown() {
        let score =
            score_pool_genre_axis(None, Some("House"), GenreFamily::Other, GenreFamily::House);
        assert!(
            (score.value - 0.5).abs() < 1e-10,
            "unknown genre should score 0.5, got {:.3}",
            score.value,
        );
    }

    #[test]
    fn eval_pool_genre_axis_other_family_not_matched() {
        let score = score_pool_genre_axis(
            Some("Noise"),
            Some("Field Recording"),
            GenreFamily::Other,
            GenreFamily::Other,
        );
        assert!(
            (score.value - 0.3).abs() < 1e-10,
            "two Other-family genres should score 0.3, got {:.3}",
            score.value,
        );
    }

    #[test]
    fn eval_discover_pools_finds_planted_clusters() {
        let cluster_a = vec![
            simple_profile("da1", "8A", 126.0, 0.55, "Deep House"),
            simple_profile("da2", "9A", 126.5, 0.58, "Deep House"),
            simple_profile("da3", "8A", 127.0, 0.60, "House"),
            simple_profile("da4", "8B", 126.0, 0.56, "Deep House"),
        ];
        let cluster_b = vec![
            simple_profile("db1", "2A", 140.0, 0.85, "Techno"),
            simple_profile("db2", "3A", 139.0, 0.82, "Techno"),
            simple_profile("db3", "2A", 141.0, 0.88, "Techno"),
        ];

        let mut all: Vec<TrackProfile> = cluster_a;
        all.extend(cluster_b);
        let refs: Vec<&TrackProfile> = all.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            130.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.65,
            3,
            12,
            10,
        );

        assert!(
            pools.len() >= 2,
            "should find at least 2 pools from 2 planted clusters, got {}",
            pools.len(),
        );

        let a_ids: HashSet<&str> = ["da1", "da2", "da3", "da4"].into();
        let b_ids: HashSet<&str> = ["db1", "db2", "db3"].into();

        let mut found_a = false;
        let mut found_b = false;
        for pool in &pools {
            let pool_ids: HashSet<&str> = pool.track_ids.iter().map(std::string::String::as_str).collect();
            let a_overlap = pool_ids.intersection(&a_ids).count();
            let b_overlap = pool_ids.intersection(&b_ids).count();
            if a_overlap >= 3 && b_overlap == 0 {
                found_a = true;
            }
            if b_overlap >= 3 && a_overlap == 0 {
                found_b = true;
            }
        }
        assert!(found_a, "should find a pool from cluster A");
        assert!(found_b, "should find a pool from cluster B");
    }

    #[test]
    fn eval_discover_pools_respects_min_max_size() {
        let profiles: Vec<TrackProfile> = (0..8)
            .map(|i| simple_profile(&format!("sz{i}"), "8A", 126.0, 0.5, "House"))
            .collect();
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.5,
            4,
            6,
            10,
        );

        assert!(!pools.is_empty(), "should find at least one pool");
        for pool in &pools {
            assert!(
                pool.track_ids.len() >= 4 && pool.track_ids.len() <= 6,
                "pool size {} should be in [4, 6]",
                pool.track_ids.len(),
            );
        }
    }

    #[test]
    fn eval_discover_pools_empty_below_min_size() {
        let profiles = [simple_profile("small1", "8A", 126.0, 0.5, "House")];
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.7,
            3,
            12,
            10,
        );
        assert!(
            pools.is_empty(),
            "should return no pools for 1 track with min_size=3"
        );
    }

    #[test]
    fn eval_discover_pools_high_threshold_yields_fewer_pools() {
        let profiles: Vec<TrackProfile> = vec![
            simple_profile("ht1", "8A", 126.0, 0.55, "House"),
            simple_profile("ht2", "9A", 126.5, 0.58, "House"),
            simple_profile("ht3", "8A", 127.0, 0.60, "Deep House"),
            simple_profile("ht4", "10A", 128.0, 0.65, "Tech House"),
            simple_profile("ht5", "11A", 130.0, 0.70, "Techno"),
        ];
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools_low = discover_pools(
            &refs,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.5,
            2,
            12,
            10,
        );
        let pools_high = discover_pools(
            &refs,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.85,
            2,
            12,
            10,
        );

        if let (Some(best_low), Some(best_high)) = (pools_low.first(), pools_high.first()) {
            assert!(
                best_high.min_compatibility >= best_low.min_compatibility - 0.05,
                "higher threshold pools should be at least as tight: low_min={:.3} high_min={:.3}",
                best_low.min_compatibility,
                best_high.min_compatibility,
            );
        }
    }

    #[test]
    fn eval_find_bridge_tracks() {
        let pools = vec![
            DiscoveredPool {
                track_ids: vec!["a".into(), "b".into(), "c".into()],
                mean_compatibility: 0.8,
                min_compatibility: 0.7,
                core_members: vec!["a".into(), "b".into()],
                edge_members: vec!["c".into()],
                score: 0.8,
            },
            DiscoveredPool {
                track_ids: vec!["c".into(), "d".into(), "e".into()],
                mean_compatibility: 0.75,
                min_compatibility: 0.65,
                core_members: vec!["d".into()],
                edge_members: vec!["c".into(), "e".into()],
                score: 0.75,
            },
        ];

        let bridges = find_bridge_tracks(&pools);
        assert_eq!(bridges.len(), 1, "track 'c' should be the only bridge");
        assert_eq!(bridges[0].0, "c");
        assert_eq!(bridges[0].1, vec![0, 1]);
    }

    #[test]
    fn eval_discover_pools_max_pools_cap() {
        let profiles: Vec<TrackProfile> = (0..8)
            .map(|i| simple_profile(&format!("mc{i}"), "8A", 126.0, 0.5, "House"))
            .collect();
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.5,
            2,
            12,
            2,
        );
        assert!(
            pools.len() <= 2,
            "max_pools=2 should cap output, got {}",
            pools.len(),
        );
        assert!(!pools.is_empty(), "should find at least one pool");
    }

    #[test]
    fn eval_discover_pools_no_subset_duplicates() {
        let profiles: Vec<TrackProfile> = (0..8)
            .map(|i| simple_profile(&format!("sd{i}"), "8A", 126.0, 0.5, "House"))
            .collect();
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.5,
            2,
            8,
            20,
        );

        for (i, a) in pools.iter().enumerate() {
            let a_set: HashSet<&str> = a.track_ids.iter().map(std::string::String::as_str).collect();
            for (j, b) in pools.iter().enumerate() {
                if i == j {
                    continue;
                }
                let b_set: HashSet<&str> = b.track_ids.iter().map(std::string::String::as_str).collect();
                assert!(
                    !a_set.is_subset(&b_set),
                    "pool {i} should not be a subset of pool {j}",
                );
            }
        }
    }

    #[test]
    fn eval_discover_pools_all_incompatible() {
        let profiles = [
            simple_profile("inc1", "1A", 100.0, 0.2, "Ambient"),
            simple_profile("inc2", "5B", 140.0, 0.9, "Drum & Bass"),
            simple_profile("inc3", "9A", 80.0, 0.5, "Dub"),
            simple_profile("inc4", "3B", 160.0, 0.95, "Hardcore"),
            simple_profile("inc5", "7A", 110.0, 0.3, "Downtempo"),
        ];
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            120.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.8,
            2,
            12,
            10,
        );
        assert!(
            pools.is_empty(),
            "all-incompatible tracks should produce no pools at threshold 0.8, got {}",
            pools.len(),
        );
    }

    #[test]
    fn eval_discover_pools_core_edge_classification() {
        let profiles = [
            simple_profile("ce1", "8A", 126.0, 0.55, "House"),
            simple_profile("ce2", "9A", 126.5, 0.57, "House"),
            simple_profile("ce3", "8A", 127.0, 0.56, "House"),
            simple_profile("ce4", "8B", 126.0, 0.58, "House"),
            simple_profile("ce5", "3A", 130.0, 0.70, "Techno"),
        ];
        let refs: Vec<&TrackProfile> = profiles.iter().collect();

        let pools = discover_pools(
            &refs,
            true,
            127.0,
            &pool_weights(PoolPreset::Balanced),
            None,
            0.5,
            3,
            12,
            10,
        );

        for pool in &pools {
            if pool.track_ids.contains(&"ce5".to_string()) {
                assert!(
                    pool.edge_members.contains(&"ce5".to_string()),
                    "ce5 (marginal track) should be an edge member, not core",
                );
            }
        }
    }
}
