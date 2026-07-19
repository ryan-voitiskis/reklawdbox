use crate::domain::planning::*;

use super::support::{mixing_policy, simple_profile, transition_moment};

#[test]
fn planning_transition_eval_bpm_curve_monotonic() {
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
fn planning_transition_conservative_penalty_is_stronger_than_balanced() {
    let from = simple_profile("pen-from", "8A", 128.0, 0.7, "House");
    let to = simple_profile("pen-to", "2A", 128.0, 0.7, "House"); // Clash: key=0.1

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
    let balanced = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Balanced),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
    );

    assert!(
        conservative.composite < balanced.composite,
        "conservative ({:.3}) should penalize harder than balanced ({:.3})",
        conservative.composite,
        balanced.composite,
    );
}

#[test]
fn planning_transition_eval_clean_transition_has_no_adjustments() {
    let from = simple_profile("adj-from", "8A", 128.0, 0.5, "House");
    let to = simple_profile("adj-to", "9A", 128.5, 0.55, "House");

    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Balanced),
        ),
        transition_moment(Some(EnergyPhase::Build), Some(EnergyPhase::Build), 0, None),
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
fn planning_transition_eval_harmonic_gate_produces_adjustment() {
    let from = simple_profile("hg-from", "8A", 128.0, 0.7, "House");
    let to = simple_profile("hg-to", "2A", 128.0, 0.7, "House"); // Clash

    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(
            &priority_weights(SequencingPriority::Balanced),
            true,
            Some(HarmonicMixingStyle::Conservative),
        ),
        transition_moment(Some(EnergyPhase::Peak), Some(EnergyPhase::Peak), 0, None),
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
fn planning_transition_genre_streak_produces_adjustment() {
    let from = simple_profile("gs-from", "8A", 128.0, 0.5, "House");
    let to = simple_profile("gs-to", "9A", 128.0, 0.55, "House");

    let scores = score_transition_profiles(
        &from,
        &to,
        mixing_policy(&priority_weights(SequencingPriority::Balanced), true, None),
        transition_moment(None, None, 2, None),
    );
    assert!(
        scores.adjustments.iter().any(|a| a.kind == "genre_streak"),
        "same-family transition with run_length=2 should produce genre_streak adjustment",
    );
}
