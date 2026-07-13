//! MCP planning compatibility and JSON presentation mappings.
//!
//! Scoring policy lives in domain planning; stateful profile and timbral
//! orchestration lives in application planning.

// Temporary compatibility exports for legacy colocated tests; retire in Plan 046.
#![allow(dead_code, unused_imports)]

use crate::domain::planning as domain;
use crate::mcp::planning::{
    EnergyCurveInput, EnergyCurvePreset, EnergyPhase, HarmonicMixingStyle, PoolPreset,
    SequencingPriority,
};

pub(in crate::mcp) use crate::application::planning::TimbralSourceSnapshot;
pub(in crate::mcp) use crate::application::planning::sweep_optimal_reference_bpm;
pub(in crate::mcp) use crate::application::planning::{
    build_track_profile, build_track_profiles, ensure_timbral_norm_stats,
    load_timbral_source_snapshot,
};
#[cfg(test)]
pub(in crate::mcp) use crate::application::planning::{
    compute_timbral_norm_stats, load_timbral_source_snapshot_for_test,
};
pub(in crate::mcp) use crate::domain::classification::taxonomy::{
    GenreFamily, map_genre_through_taxonomy,
};
pub(in crate::mcp) use crate::domain::planning::{
    AxisScore, CamelotKey, CandidatePlan, CandidatePoolScore, CandidateTransition, DiscoveredPool,
    PoolAxisScores, PoolCohesionResult, PoolWeights, PriorityWeights, ScoreAdjustment,
    ScoringContext, TIMBRAL_VECTOR_SCHEMA_VERSION, TimbralFeatures, TrackProfile,
};
pub(in crate::mcp) use crate::domain::planning::{
    bpm_pitch_shift, build_timbral_vector, composite_score, find_bridge_tracks, format_camelot,
    genre_family_for, key_to_camelot, musical_key_to_camelot, parse_camelot_key,
    round_to_3_decimals, score_bpm_axis, score_genre_axis, score_key_axis,
    score_key_with_pitch_shifts, score_pool_bpm_axis, score_pool_energy_axis,
    score_pool_genre_axis, transpose_camelot_key,
};

fn domain_phase(phase: EnergyPhase) -> domain::EnergyPhase {
    match phase {
        EnergyPhase::Warmup => domain::EnergyPhase::Warmup,
        EnergyPhase::Build => domain::EnergyPhase::Build,
        EnergyPhase::Peak => domain::EnergyPhase::Peak,
        EnergyPhase::Release => domain::EnergyPhase::Release,
    }
}

fn domain_curve_preset(preset: EnergyCurvePreset) -> domain::EnergyCurvePreset {
    match preset {
        EnergyCurvePreset::WarmupBuildPeakRelease => {
            domain::EnergyCurvePreset::WarmupBuildPeakRelease
        }
        EnergyCurvePreset::FlatEnergy => domain::EnergyCurvePreset::FlatEnergy,
        EnergyCurvePreset::PeakOnly => domain::EnergyCurvePreset::PeakOnly,
    }
}

fn domain_curve(input: &EnergyCurveInput) -> domain::EnergyCurve {
    match input {
        EnergyCurveInput::Preset(preset) => {
            domain::EnergyCurve::Preset(domain_curve_preset(*preset))
        }
        EnergyCurveInput::Custom(phases) => {
            domain::EnergyCurve::Custom(phases.iter().copied().map(domain_phase).collect())
        }
    }
}

fn domain_style(style: HarmonicMixingStyle) -> domain::HarmonicMixingStyle {
    match style {
        HarmonicMixingStyle::Conservative => domain::HarmonicMixingStyle::Conservative,
        HarmonicMixingStyle::Balanced => domain::HarmonicMixingStyle::Balanced,
        HarmonicMixingStyle::Adventurous => domain::HarmonicMixingStyle::Adventurous,
    }
}

pub(in crate::mcp) fn resolve_energy_curve(
    energy_curve: Option<&EnergyCurveInput>,
    target_tracks: usize,
) -> Result<Vec<EnergyPhase>, String> {
    let curve = energy_curve.map(domain_curve);
    domain::resolve_energy_curve(curve.as_ref(), target_tracks).map(|phases| {
        phases
            .into_iter()
            .map(|phase| match phase {
                domain::EnergyPhase::Warmup => EnergyPhase::Warmup,
                domain::EnergyPhase::Build => EnergyPhase::Build,
                domain::EnergyPhase::Peak => EnergyPhase::Peak,
                domain::EnergyPhase::Release => EnergyPhase::Release,
            })
            .collect()
    })
}

pub(in crate::mcp) fn select_start_track_ids(
    profiles_by_id: &std::collections::HashMap<String, TrackProfile>,
    requested_candidates: usize,
    first_phase: EnergyPhase,
    forced_start: Option<&str>,
) -> Vec<String> {
    domain::select_start_track_ids(
        profiles_by_id,
        requested_candidates,
        domain_phase(first_phase),
        forced_start,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn build_candidate_plan(
    profiles_by_id: &std::collections::HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    weights: &PriorityWeights,
    variation_index: usize,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    bpm_drift_pct: f64,
    target_bpms: Option<&[f64]>,
) -> CandidatePlan {
    let phases: Vec<_> = phases.iter().copied().map(domain_phase).collect();
    domain::build_candidate_plan(
        profiles_by_id,
        start_track_id,
        target_tracks,
        &phases,
        weights,
        variation_index,
        master_tempo,
        harmonic_style.map(domain_style),
        bpm_drift_pct,
        target_bpms,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn build_candidate_plan_beam(
    profiles_by_id: &std::collections::HashMap<String, TrackProfile>,
    start_track_id: &str,
    target_tracks: usize,
    phases: &[EnergyPhase],
    weights: &PriorityWeights,
    beam_width: usize,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    bpm_drift_pct: f64,
    target_bpms: Option<&[f64]>,
) -> Vec<CandidatePlan> {
    let phases: Vec<_> = phases.iter().copied().map(domain_phase).collect();
    domain::build_candidate_plan_beam(
        profiles_by_id,
        start_track_id,
        target_tracks,
        &phases,
        weights,
        beam_width,
        master_tempo,
        harmonic_style.map(domain_style),
        bpm_drift_pct,
        target_bpms,
    )
}

pub(in crate::mcp) fn compute_bpm_trajectory(
    phases: &[EnergyPhase],
    start_bpm: f64,
    end_bpm: f64,
) -> Vec<f64> {
    let phases: Vec<_> = phases.iter().copied().map(domain_phase).collect();
    domain::compute_bpm_trajectory(&phases, start_bpm, end_bpm)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn score_transition_profiles(
    from: &TrackProfile,
    to: &TrackProfile,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    weights: &PriorityWeights,
    master_tempo: bool,
    harmonic_style: Option<HarmonicMixingStyle>,
    context: &ScoringContext,
    play_bpms: Option<(f64, f64)>,
) -> TransitionScores {
    domain::score_transition_profiles(
        from,
        to,
        from_phase.map(domain_phase),
        to_phase.map(domain_phase),
        weights,
        master_tempo,
        harmonic_style.map(domain_style),
        context,
        play_bpms,
    )
}

pub(in crate::mcp) use crate::domain::planning::TransitionScores;

pub(in crate::mcp) fn score_energy_axis(
    from_energy: f64,
    to_energy: f64,
    from_phase: Option<EnergyPhase>,
    to_phase: Option<EnergyPhase>,
    to_loudness_range: Option<f64>,
) -> AxisScore {
    domain::score_energy_axis(
        from_energy,
        to_energy,
        from_phase.map(domain_phase),
        to_phase.map(domain_phase),
        to_loudness_range,
    )
}

pub(in crate::mcp) fn priority_weights(priority: SequencingPriority) -> PriorityWeights {
    let priority = match priority {
        SequencingPriority::Balanced => domain::SequencingPriority::Balanced,
        SequencingPriority::Harmonic => domain::SequencingPriority::Harmonic,
        SequencingPriority::Energy => domain::SequencingPriority::Energy,
        SequencingPriority::Genre => domain::SequencingPriority::Genre,
    };
    domain::priority_weights(priority)
}

pub(in crate::mcp) fn pool_weights(preset: PoolPreset) -> PoolWeights {
    let preset = match preset {
        PoolPreset::Balanced => domain::PoolPreset::Balanced,
        PoolPreset::Timbral => domain::PoolPreset::Timbral,
    };
    domain::pool_weights(preset)
}

pub(in crate::mcp) fn compute_track_energy(
    essentia: Option<&crate::audio::EssentiaOutput>,
    bpm: f64,
) -> f64 {
    domain::compute_track_energy(
        essentia.map(|output| {
            (
                output.danceability,
                output.loudness_integrated,
                output.onset_rate,
            )
        }),
        bpm,
    )
}

pub(in crate::mcp) fn normalize_timbral_vector(
    raw: &[f64],
    stats: &crate::store::TimbralNormStats,
) -> Option<Vec<f64>> {
    let stats = crate::application::planning::normalization_from_persisted(stats);
    domain::normalize_timbral_vector(raw, &stats)
}

pub(in crate::mcp) fn score_pool_timbral_axis(
    a: &TrackProfile,
    b: &TrackProfile,
    stats: &crate::store::TimbralNormStats,
) -> Option<AxisScore> {
    let stats = crate::application::planning::normalization_from_persisted(stats);
    domain::score_pool_timbral_axis(a, b, &stats)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn score_pool_compatibility_pair(
    a: &TrackProfile,
    b: &TrackProfile,
    master_tempo: bool,
    ref_bpm: f64,
    weights: &PoolWeights,
    norm_stats: Option<&crate::store::TimbralNormStats>,
) -> PoolAxisScores {
    let norm_stats = norm_stats.map(crate::application::planning::normalization_from_persisted);
    domain::score_pool_compatibility_pair(a, b, master_tempo, ref_bpm, weights, norm_stats.as_ref())
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn score_candidate_vs_pool(
    candidate: &TrackProfile,
    pool: &[&TrackProfile],
    master_tempo: bool,
    ref_bpm: f64,
    weights: &PoolWeights,
    norm_stats: Option<&crate::store::TimbralNormStats>,
) -> CandidatePoolScore {
    let norm_stats = norm_stats.map(crate::application::planning::normalization_from_persisted);
    domain::score_candidate_vs_pool(
        candidate,
        pool,
        master_tempo,
        ref_bpm,
        weights,
        norm_stats.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn compute_pool_cohesion(
    profiles: &[&TrackProfile],
    master_tempo: bool,
    ref_bpm: f64,
    weights: &PoolWeights,
    norm_stats: Option<&crate::store::TimbralNormStats>,
) -> PoolCohesionResult {
    let norm_stats = norm_stats.map(crate::application::planning::normalization_from_persisted);
    domain::compute_pool_cohesion(
        profiles,
        master_tempo,
        ref_bpm,
        weights,
        norm_stats.as_ref(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::mcp) fn discover_pools(
    profiles: &[&TrackProfile],
    master_tempo: bool,
    ref_bpm: f64,
    weights: &PoolWeights,
    norm_stats: Option<&crate::store::TimbralNormStats>,
    threshold: f64,
    min_size: usize,
    max_size: usize,
    max_pools: usize,
) -> Vec<DiscoveredPool> {
    let norm_stats = norm_stats.map(crate::application::planning::normalization_from_persisted);
    domain::discover_pools(
        profiles,
        master_tempo,
        ref_bpm,
        weights,
        norm_stats.as_ref(),
        threshold,
        min_size,
        max_size,
        max_pools,
    )
}

pub(in crate::mcp) trait TransitionScoresPresentation {
    fn to_json(&self) -> serde_json::Value;
}

impl TransitionScoresPresentation for TransitionScores {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "key": { "value": round_to_3_decimals(self.key.value), "label": self.key.label },
            "bpm": { "value": round_to_3_decimals(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_to_3_decimals(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_to_3_decimals(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_to_3_decimals(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_to_3_decimals(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_to_3_decimals(self.composite),
        });
        if !self.adjustments.is_empty() {
            json["adjustments"] = serde_json::json!(
                self.adjustments
                    .iter()
                    .map(|adjustment| serde_json::json!({
                        "kind": adjustment.kind,
                        "delta": round_to_3_decimals(adjustment.delta),
                        "composite_without": round_to_3_decimals(adjustment.composite_without),
                        "reason": adjustment.reason,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        json
    }
}

pub(in crate::mcp) trait PoolAxisScoresPresentation {
    fn to_json(&self) -> serde_json::Value;
}

impl PoolAxisScoresPresentation for PoolAxisScores {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "key": { "value": round_to_3_decimals(self.key.value), "label": self.key.label },
            "bpm": { "value": round_to_3_decimals(self.bpm.value), "label": self.bpm.label },
            "energy": { "value": round_to_3_decimals(self.energy.value), "label": self.energy.label },
            "genre": { "value": round_to_3_decimals(self.genre.value), "label": self.genre.label },
            "brightness": { "value": round_to_3_decimals(self.brightness.value), "label": self.brightness.label },
            "rhythm": { "value": round_to_3_decimals(self.rhythm.value), "label": self.rhythm.label },
            "composite": round_to_3_decimals(self.composite),
        });
        if let Some(ref timbral) = self.timbral {
            json["timbral"] = serde_json::json!({
                "value": round_to_3_decimals(timbral.value),
                "label": timbral.label,
            });
        }
        json
    }
}
