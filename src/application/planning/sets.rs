//! Set-building orchestration.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::domain::planning::{
    CandidatePlan, EnergyCurve, HarmonicMixingStyle, PriorityWeights, TrackProfile,
    build_candidate_plan, build_candidate_plan_beam, compute_bpm_trajectory, resolve_energy_curve,
    select_start_track_ids,
};

pub(crate) struct BuildSetOptions {
    pub(crate) tracks: Vec<crate::types::Track>,
    pub(crate) requested_target: usize,
    pub(crate) energy_curve: Option<EnergyCurve>,
    pub(crate) opening_track_id: Option<String>,
    pub(crate) beam_width: usize,
    pub(crate) weights: PriorityWeights,
    pub(crate) master_tempo: bool,
    pub(crate) harmonic_style: HarmonicMixingStyle,
    pub(crate) bpm_drift_pct: f64,
    pub(crate) bpm_range: Option<(f64, f64)>,
}

pub(crate) struct BuiltSetCandidates {
    pub(crate) profiles_by_id: HashMap<String, TrackProfile>,
    pub(crate) plans: Vec<CandidatePlan>,
    pub(crate) actual_target: usize,
    pub(crate) beam_width: usize,
    pub(crate) bpm_trajectory: Option<Vec<f64>>,
}

pub(crate) enum BuildSetError {
    Profile(String),
    OpeningTrack(String),
    EnergyCurve(String),
}

pub(crate) fn build_set_candidates(
    store: &Connection,
    options: BuildSetOptions,
) -> Result<BuiltSetCandidates, BuildSetError> {
    let profiles_by_id: HashMap<String, TrackProfile> =
        super::build_track_profiles(options.tracks, store)
            .map_err(BuildSetError::Profile)?
            .into_iter()
            .map(|profile| (profile.track.id.clone(), profile))
            .collect();

    if let Some(opening_track_id) = options.opening_track_id.as_deref()
        && !profiles_by_id.contains_key(opening_track_id)
    {
        return Err(BuildSetError::OpeningTrack(format!(
            "opening_track_id '{opening_track_id}' is not in track_ids"
        )));
    }

    let actual_target = options.requested_target.min(profiles_by_id.len());
    let phases = match options.energy_curve.as_ref() {
        Some(EnergyCurve::Custom(_)) => {
            resolve_energy_curve(options.energy_curve.as_ref(), options.requested_target)
                .map_err(BuildSetError::EnergyCurve)?
                .into_iter()
                .take(actual_target)
                .collect()
        }
        _ => resolve_energy_curve(options.energy_curve.as_ref(), actual_target)
            .map_err(BuildSetError::EnergyCurve)?,
    };
    let bpm_trajectory = options
        .bpm_range
        .map(|(start, end)| compute_bpm_trajectory(&phases, start, end));
    let start_tracks = select_start_track_ids(
        &profiles_by_id,
        if profiles_by_id.len() <= actual_target {
            1
        } else {
            options.beam_width
        },
        phases[0],
        options.opening_track_id.as_deref(),
    );

    let plans = if options.beam_width <= 1 {
        let effective_candidates = if profiles_by_id.len() <= actual_target {
            1
        } else {
            start_tracks.len()
        };
        (0..effective_candidates)
            .map(|variation_index| {
                let start_id = start_tracks[variation_index % start_tracks.len()].clone();
                build_candidate_plan(
                    &profiles_by_id,
                    &start_id,
                    actual_target,
                    &phases,
                    &options.weights,
                    variation_index,
                    options.master_tempo,
                    Some(options.harmonic_style),
                    options.bpm_drift_pct,
                    bpm_trajectory.as_deref(),
                )
            })
            .collect()
    } else {
        let mut all_plans = Vec::new();
        for start_id in &start_tracks {
            let mut beam_plans = build_candidate_plan_beam(
                &profiles_by_id,
                start_id,
                actual_target,
                &phases,
                &options.weights,
                options.beam_width,
                options.master_tempo,
                Some(options.harmonic_style),
                options.bpm_drift_pct,
                bpm_trajectory.as_deref(),
            );
            all_plans.append(&mut beam_plans);
        }
        let mut seen_track_sequences: HashSet<Vec<String>> = HashSet::new();
        all_plans.retain(|plan| seen_track_sequences.insert(plan.ordered_ids.clone()));
        all_plans.sort_by(|left, right| {
            let left_mean = mean_plan_score(left);
            let right_mean = mean_plan_score(right);
            right_mean
                .partial_cmp(&left_mean)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_plans.truncate(options.beam_width);
        all_plans
    };

    Ok(BuiltSetCandidates {
        profiles_by_id,
        plans,
        actual_target,
        beam_width: options.beam_width,
        bpm_trajectory,
    })
}

fn mean_plan_score(plan: &CandidatePlan) -> f64 {
    if plan.transitions.is_empty() {
        0.0
    } else {
        plan.transitions
            .iter()
            .map(|transition| transition.scores.composite)
            .sum::<f64>()
            / plan.transitions.len() as f64
    }
}
