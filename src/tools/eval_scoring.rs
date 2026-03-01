//! Scoring evaluation harness — synthetic-pool quality gates.
//!
//! Runs deterministic tests against synthetic track pools (no DB required)
//! to catch scoring regressions. Follows the threshold-based eval pattern
//! from `eval_routing.rs`.

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::tools::params::*;
    use crate::tools::scoring::*;

    // -----------------------------------------------------------------------
    // Quality gate thresholds
    // -----------------------------------------------------------------------

    const MEAN_COMPOSITE_MIN: f64 = 0.65;
    const MIN_COMPOSITE_MIN: f64 = 0.30;
    const COMPOSITE_VARIANCE_MAX: f64 = 0.08;
    const HARMONIC_COHERENCE_MIN: f64 = 0.50; // fraction of transitions with key ≥ 0.8
    const ENERGY_FIDELITY_MIN: f64 = 0.40; // fraction of transitions with energy = 1.0
    const MAX_PITCH_ADJUSTMENT: f64 = 8.0; // max BPM pct across all transitions

    // -----------------------------------------------------------------------
    // Helper: build a TrackProfile without DB access
    // -----------------------------------------------------------------------

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
        }
    }

    fn simple_profile(id: &str, key: &str, bpm: f64, energy: f64, genre: &str) -> TrackProfile {
        synth_profile(id, key, bpm, energy, genre, None, None, None)
    }

    // -----------------------------------------------------------------------
    // Evaluation reporting
    // -----------------------------------------------------------------------

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
        let min = composites.iter().cloned().fold(f64::INFINITY, f64::min);
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

    // -----------------------------------------------------------------------
    // Pool: Camelot walk — 8 tracks forming a perfect harmonic path
    // -----------------------------------------------------------------------

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
            SequencingPriority::Harmonic,
            0,
            true,
            Some(HarmonicMixingStyle::Conservative),
            6.0,
            None,
        );
        assert_eq!(plan.ordered_ids.len(), 8, "should use all 8 tracks");

        let metrics = compute_metrics(&plan, &phases);
        assert_quality_gates(&metrics, "camelot_walk_greedy");

        // Key-specific: harmonic priority on a perfect walk should achieve near-100% coherence
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
            SequencingPriority::Harmonic,
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

        // Best beam plan should pass quality gates
        let best = &plans[0];
        let metrics = compute_metrics(best, &phases);
        assert_quality_gates(&metrics, "camelot_walk_beam");
    }

    // -----------------------------------------------------------------------
    // Pool: Adversarial — hostile distributions
    // -----------------------------------------------------------------------

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
            SequencingPriority::Balanced,
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            12.0, // wide drift tolerance for adversarial pool
            None,
        );
        assert_eq!(plan.ordered_ids.len(), 6, "should use all 6 tracks");

        // Adversarial pool: very relaxed gates (no good transitions exist)
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

    // -----------------------------------------------------------------------
    // Pool: Iso key/BPM — forces differentiation on energy/genre/brightness
    // -----------------------------------------------------------------------

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
            SequencingPriority::Energy,
            0,
            true,
            Some(HarmonicMixingStyle::Balanced),
            6.0,
            None,
        );
        assert_eq!(plan.ordered_ids.len(), 8);

        let metrics = compute_metrics(&plan, &phases);
        assert_quality_gates(&metrics, "iso_key_bpm");

        // Key and BPM are identical → key=1.0, bpm=1.0 always
        assert!(
            metrics.harmonic_coherence >= 1.0 - f64::EPSILON,
            "iso pool should have 100% harmonic coherence",
        );
    }

    // -----------------------------------------------------------------------
    // Pool: Realistic club — 20 tracks, 3 genre families
    // -----------------------------------------------------------------------

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
            SequencingPriority::Balanced,
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
            SequencingPriority::Balanced,
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

    // -----------------------------------------------------------------------
    // Beam ≥ greedy quality assertion
    // -----------------------------------------------------------------------

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
            SequencingPriority::Balanced,
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
            SequencingPriority::Balanced,
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

        // Beam should be at least as good as greedy (within small tolerance)
        assert!(
            beam_best_mean >= greedy_mean - 0.01,
            "beam best mean {beam_best_mean:.3} should be ≥ greedy mean {greedy_mean:.3} - 0.01",
        );
    }

    // -----------------------------------------------------------------------
    // Priority axis shift verification
    // -----------------------------------------------------------------------

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
            SequencingPriority::Balanced,
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
            SequencingPriority::Harmonic,
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

    // -----------------------------------------------------------------------
    // Determinism
    // -----------------------------------------------------------------------

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
            SequencingPriority::Balanced,
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
            SequencingPriority::Balanced,
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

    // -----------------------------------------------------------------------
    // Sensitivity smoke test
    // -----------------------------------------------------------------------

    #[test]
    fn eval_bpm_curve_monotonic() {
        // Verify the exponential BPM curve is monotonically decreasing
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
            SequencingPriority::Balanced,
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
            SequencingPriority::Balanced,
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

    // -----------------------------------------------------------------------
    // Adjustment presence/absence tests
    // -----------------------------------------------------------------------

    #[test]
    fn eval_clean_transition_has_no_adjustments() {
        let from = simple_profile("adj-from", "8A", 128.0, 0.5, "House");
        let to = simple_profile("adj-to", "9A", 128.5, 0.55, "House");

        let scores = score_transition_profiles(
            &from,
            &to,
            Some(EnergyPhase::Build),
            Some(EnergyPhase::Build),
            SequencingPriority::Balanced,
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
            SequencingPriority::Balanced,
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

        // Run length > 0 and < 5, same family → streak bonus
        let scores = score_transition_profiles(
            &from,
            &to,
            None,
            None,
            SequencingPriority::Balanced,
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
}
