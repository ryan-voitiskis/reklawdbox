//! Pool planning and scoring orchestration.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::domain::classification::taxonomy::GenreFamily;
use crate::domain::planning::{
    CandidatePoolScore, DiscoveredPool, PoolAxisScores, PoolCohesionResult, PoolWeights,
    TimbralNormalization, TrackProfile, bpm_pitch_shift, compute_pool_cohesion, discover_pools,
    find_bridge_tracks, format_camelot, pool_weights, renormalize_pool, score_candidate_vs_pool,
    score_key_with_pitch_shifts, score_pool_compatibility_pair, transpose_camelot_key,
};

use super::{ensure_timbral_norm_stats, normalization_from_persisted};

pub(crate) struct ProfileBatch {
    pub(crate) profiles: Vec<TrackProfile>,
    pub(crate) skipped: Vec<String>,
}

pub(crate) fn build_pool_profiles(
    tracks: Vec<crate::types::Track>,
    store: &Connection,
) -> ProfileBatch {
    let skipped: Vec<_> = tracks.iter().map(|track| track.id.clone()).collect();
    match super::build_track_profiles(tracks, store) {
        Ok(profiles) => ProfileBatch {
            profiles,
            skipped: Vec::new(),
        },
        Err(error) => {
            tracing::warn!(%error, "Skipping profile batch: cache read failed");
            ProfileBatch {
                profiles: Vec::new(),
                skipped,
            }
        }
    }
}

fn available_normalization(store: &Connection) -> Option<TimbralNormalization> {
    ensure_timbral_norm_stats(store)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "Timbral norm stats unavailable, scoring without timbral axis");
            None
        })
        .as_ref()
        .map(normalization_from_persisted)
}

pub(crate) fn median_bpm(bpms: &[f64]) -> f64 {
    if bpms.is_empty() {
        return 0.0;
    }
    let mut sorted = bpms.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

pub(crate) struct PairwisePoolEvaluation {
    pub(crate) track_a: TrackProfile,
    pub(crate) track_b: TrackProfile,
    pub(crate) reference_bpm: f64,
    pub(crate) scores: PoolAxisScores,
}

pub(crate) fn evaluate_pool_pair(
    tracks: [crate::types::Track; 2],
    store: &Connection,
    master_tempo: bool,
    reference_bpm: Option<f64>,
    weights: &PoolWeights,
) -> Result<PairwisePoolEvaluation, String> {
    let mut profiles = super::build_track_profiles(Vec::from(tracks), store)?;
    let track_b = profiles
        .pop()
        .expect("two input tracks produce two profiles");
    let track_a = profiles
        .pop()
        .expect("two input tracks produce two profiles");
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&[track_a.bpm, track_b.bpm]));
    let normalization = available_normalization(store);
    let scores = score_pool_compatibility_pair(
        &track_a,
        &track_b,
        master_tempo,
        reference_bpm,
        weights,
        normalization.as_ref(),
    );
    Ok(PairwisePoolEvaluation {
        track_a,
        track_b,
        reference_bpm,
        scores,
    })
}

pub(crate) struct CandidatePoolEvaluation {
    pub(crate) candidate: TrackProfile,
    pub(crate) reference_bpm: f64,
    pub(crate) scores: CandidatePoolScore,
    pub(crate) skipped: Vec<String>,
}

pub(crate) fn evaluate_candidate_pool(
    candidate: crate::types::Track,
    pool: Vec<crate::types::Track>,
    store: &Connection,
    master_tempo: bool,
    reference_bpm: Option<f64>,
    weights: &PoolWeights,
) -> Result<CandidatePoolEvaluation, String> {
    let candidate = super::build_track_profile(candidate, store)?;
    let built = build_pool_profiles(pool, store);
    if built.profiles.is_empty() {
        return Err("Failed to build any pool profiles".to_string());
    }
    let normalization = available_normalization(store);
    let mut bpms: Vec<_> = built.profiles.iter().map(|profile| profile.bpm).collect();
    bpms.push(candidate.bpm);
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&bpms));
    let pool_refs: Vec<_> = built.profiles.iter().collect();
    let scores = score_candidate_vs_pool(
        &candidate,
        &pool_refs,
        master_tempo,
        reference_bpm,
        weights,
        normalization.as_ref(),
    );
    Ok(CandidatePoolEvaluation {
        candidate,
        reference_bpm,
        scores,
        skipped: built.skipped,
    })
}

pub(crate) struct CohesionEvaluation {
    pub(crate) reference_bpm: f64,
    pub(crate) cohesion: PoolCohesionResult,
    pub(crate) skipped: Vec<String>,
}

pub(crate) fn evaluate_pool_cohesion(
    tracks: Vec<crate::types::Track>,
    store: &Connection,
    master_tempo: bool,
    reference_bpm: Option<f64>,
    weights: &PoolWeights,
) -> Result<CohesionEvaluation, String> {
    let built = build_pool_profiles(tracks, store);
    if built.profiles.len() < 2 {
        return Err("Need at least 2 valid profiles for cohesion".to_string());
    }
    let normalization = available_normalization(store);
    let bpms: Vec<_> = built.profiles.iter().map(|profile| profile.bpm).collect();
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&bpms));
    let profile_refs: Vec<_> = built.profiles.iter().collect();
    let cohesion = compute_pool_cohesion(
        &profile_refs,
        master_tempo,
        reference_bpm,
        weights,
        normalization.as_ref(),
    );
    Ok(CohesionEvaluation {
        reference_bpm,
        cohesion,
        skipped: built.skipped,
    })
}

pub(crate) struct WeakPoolMember {
    pub(crate) track_id: String,
    pub(crate) min_score_to_pool: f64,
}

pub(crate) struct PoolDescription {
    pub(crate) cohesion: PoolCohesionResult,
    pub(crate) weak_members: Vec<WeakPoolMember>,
    pub(crate) energy_band: (f64, f64),
    pub(crate) bpm_center: f64,
    pub(crate) bpm_spread: f64,
    pub(crate) key_neighborhood: Vec<String>,
    pub(crate) dominant_genre: Option<String>,
    pub(crate) analysis_coverage: f64,
    pub(crate) track_count: usize,
    pub(crate) reference_bpm: f64,
    pub(crate) optimal_reference: Option<(f64, f64)>,
    pub(crate) median_key_stability: Option<f64>,
    pub(crate) skipped: Vec<String>,
}

pub(crate) fn describe_pool(
    tracks: Vec<crate::types::Track>,
    store: &Connection,
    master_tempo: bool,
    reference_bpm: Option<f64>,
    weights: &PoolWeights,
) -> Result<PoolDescription, String> {
    let built = build_pool_profiles(tracks, store);
    let profiles = built.profiles;
    if profiles.len() < 2 {
        return Err("Failed to build enough profiles".to_string());
    }
    let normalization = available_normalization(store);
    let bpms: Vec<_> = profiles.iter().map(|profile| profile.bpm).collect();
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&bpms));
    let profile_refs: Vec<_> = profiles.iter().collect();
    let cohesion = compute_pool_cohesion(
        &profile_refs,
        master_tempo,
        reference_bpm,
        weights,
        normalization.as_ref(),
    );

    let energies: Vec<_> = profiles.iter().map(|profile| profile.energy).collect();
    let energy_min = energies.iter().copied().reduce(f64::min).unwrap();
    let energy_max = energies.iter().copied().reduce(f64::max).unwrap();
    let bpm_min = bpms.iter().copied().reduce(f64::min).unwrap();
    let bpm_max = bpms.iter().copied().reduce(f64::max).unwrap();
    let bpm_center = median_bpm(&bpms);

    let key_neighborhood = profiles
        .iter()
        .filter_map(|profile| {
            let key = profile.camelot_key?;
            if !master_tempo && reference_bpm > 0.0 {
                let shift = bpm_pitch_shift(profile.bpm, reference_bpm).round() as i32;
                Some(format_camelot(transpose_camelot_key(key, shift)))
            } else {
                Some(format_camelot(key))
            }
        })
        .collect();

    let mut genre_counts: HashMap<&str, usize> = HashMap::new();
    for profile in &profiles {
        if let Some(ref genre) = profile.canonical_genre {
            *genre_counts.entry(genre.as_str()).or_default() += 1;
        }
    }
    let dominant_genre = genre_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(genre, _)| genre.to_string());

    let weak_candidates: HashSet<&str> = cohesion
        .per_pair
        .iter()
        .filter(|(_, _, scores)| scores.composite < 0.5)
        .flat_map(|(left, right, _)| [left.as_str(), right.as_str()])
        .collect();
    let weak_members = weak_candidates
        .into_iter()
        .filter_map(|track_id| {
            let min_score = cohesion
                .per_pair
                .iter()
                .filter(|(left, right, _)| left == track_id || right == track_id)
                .map(|(_, _, scores)| scores.composite)
                .reduce(f64::min)?;
            (min_score < 0.5).then(|| WeakPoolMember {
                track_id: track_id.to_string(),
                min_score_to_pool: min_score,
            })
        })
        .collect();

    let essentia_count = profiles
        .iter()
        .filter(|profile| profile.timbral.is_some())
        .count();
    let analysis_coverage = essentia_count as f64 / profiles.len() as f64;
    let (optimal_reference, median_key_stability) = if master_tempo {
        (None, None)
    } else {
        let median_ref = median_bpm(&bpms);
        let optimal = sweep_optimal_reference_bpm(&profiles, &bpms);
        let median_stability = optimal
            .as_ref()
            .map(|_| compute_key_stability_at_bpm(&profiles, median_ref));
        (optimal, median_stability)
    };

    Ok(PoolDescription {
        cohesion,
        weak_members,
        energy_band: (energy_min, energy_max),
        bpm_center,
        bpm_spread: bpm_max - bpm_min,
        key_neighborhood,
        dominant_genre,
        analysis_coverage,
        track_count: profiles.len(),
        reference_bpm,
        optimal_reference,
        median_key_stability,
        skipped: built.skipped,
    })
}

pub(crate) struct ExpansionSeed {
    pub(crate) profiles: Vec<TrackProfile>,
    pub(crate) skipped: Vec<String>,
    pub(crate) bpm_low: f64,
    pub(crate) bpm_high: f64,
    pub(crate) reference_bpm: f64,
    pub(crate) track_ids: HashSet<String>,
}

pub(crate) fn prepare_pool_expansion(
    tracks: Vec<crate::types::Track>,
    store: &Connection,
    reference_bpm: Option<f64>,
) -> Result<ExpansionSeed, String> {
    let built = build_pool_profiles(tracks, store);
    if built.profiles.is_empty() {
        return Err("Failed to build any seed profiles".to_string());
    }
    let bpms: Vec<_> = built.profiles.iter().map(|profile| profile.bpm).collect();
    let min_bpm = bpms.iter().copied().reduce(f64::min).unwrap();
    let max_bpm = bpms.iter().copied().reduce(f64::max).unwrap();
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&bpms));
    let track_ids = built
        .profiles
        .iter()
        .map(|profile| profile.track.id.clone())
        .collect();
    Ok(ExpansionSeed {
        profiles: built.profiles,
        skipped: built.skipped,
        bpm_low: min_bpm * 0.92,
        bpm_high: max_bpm * 1.08,
        reference_bpm,
        track_ids,
    })
}

pub(crate) struct AdditionRationale {
    pub(crate) strongest_axes: Vec<&'static str>,
    pub(crate) weakest_axis: &'static str,
    pub(crate) most_compatible_member: String,
}

pub(crate) struct PoolAddition {
    pub(crate) track_id: String,
    pub(crate) title: String,
    pub(crate) artist: String,
    pub(crate) min_score: f64,
    pub(crate) mean_score: f64,
    pub(crate) rationale: AdditionRationale,
}

pub(crate) struct ExpandedPool {
    pub(crate) additions: Vec<PoolAddition>,
    pub(crate) final_cohesion: PoolCohesionResult,
    pub(crate) stopped_early: bool,
    pub(crate) candidates_scanned: usize,
    pub(crate) reference_bpm: f64,
    pub(crate) skipped_seed_tracks: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn expand_pool(
    seed: ExpansionSeed,
    candidate_tracks: Vec<crate::types::Track>,
    store: &Connection,
    additions: usize,
    cross_genre: bool,
    master_tempo: bool,
    weights: &PoolWeights,
) -> ExpandedPool {
    let normalization = available_normalization(store);
    let seed_families: HashSet<GenreFamily> = seed
        .profiles
        .iter()
        .map(|profile| profile.genre_family)
        .collect();
    let mut candidates: Vec<_> = build_pool_profiles(candidate_tracks, store)
        .profiles
        .into_iter()
        .filter(|profile| cross_genre || seed_families.contains(&profile.genre_family))
        .collect();
    let candidates_scanned = candidates.len();
    let mut pool = seed.profiles;
    let mut added = Vec::new();
    let quality_threshold = 0.4;

    for _ in 0..additions {
        if candidates.is_empty() {
            break;
        }
        let pool_refs: Vec<_> = pool.iter().collect();
        let mut best_idx = 0;
        let mut best_min = f64::NEG_INFINITY;
        let mut best_mean = 0.0;
        let mut best_result = None;
        for (index, candidate) in candidates.iter().enumerate() {
            let result = score_candidate_vs_pool(
                candidate,
                &pool_refs,
                master_tempo,
                seed.reference_bpm,
                weights,
                normalization.as_ref(),
            );
            if result.min_score > best_min
                || (result.min_score == best_min && result.mean_score > best_mean)
            {
                best_idx = index;
                best_min = result.min_score;
                best_mean = result.mean_score;
                best_result = Some(result);
            }
        }
        if best_min < quality_threshold {
            break;
        }
        let chosen = candidates.swap_remove(best_idx);
        let result = best_result.expect("a non-empty candidate list produces a score");
        added.push(PoolAddition {
            track_id: chosen.track.id.clone(),
            title: chosen.track.title.clone(),
            artist: chosen.track.artist.clone(),
            min_score: result.min_score,
            mean_score: result.mean_score,
            rationale: addition_rationale(&result),
        });
        pool.push(chosen);
    }
    let stopped_early = added.len() < additions;
    let pool_refs: Vec<_> = pool.iter().collect();
    let final_cohesion = compute_pool_cohesion(
        &pool_refs,
        master_tempo,
        seed.reference_bpm,
        weights,
        normalization.as_ref(),
    );
    ExpandedPool {
        additions: added,
        final_cohesion,
        stopped_early,
        candidates_scanned,
        reference_bpm: seed.reference_bpm,
        skipped_seed_tracks: seed.skipped,
    }
}

fn addition_rationale(result: &CandidatePoolScore) -> AdditionRationale {
    if result.per_member.is_empty() {
        return AdditionRationale {
            strongest_axes: Vec::new(),
            weakest_axis: "unknown",
            most_compatible_member: String::new(),
        };
    }
    let most_compatible_member = result
        .per_member
        .iter()
        .max_by(|(_, left), (_, right)| {
            left.composite
                .partial_cmp(&right.composite)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map_or_else(String::new, |(track_id, _)| track_id.clone());
    let mut axis_sums: HashMap<&'static str, (f64, u32)> = HashMap::new();
    for (_, scores) in &result.per_member {
        for (name, value) in [
            ("key", scores.key.value),
            ("bpm", scores.bpm.value),
            ("energy", scores.energy.value),
            ("genre", scores.genre.value),
            ("brightness", scores.brightness.value),
            ("rhythm", scores.rhythm.value),
        ] {
            let entry = axis_sums.entry(name).or_insert((0.0, 0));
            entry.0 += value;
            entry.1 += 1;
        }
        if let Some(ref timbral) = scores.timbral {
            let entry = axis_sums.entry("timbral").or_insert((0.0, 0));
            entry.0 += timbral.value;
            entry.1 += 1;
        }
    }
    let mut axis_means: Vec<_> = axis_sums
        .iter()
        .map(|(name, (sum, count))| (*name, sum / *count as f64))
        .collect();
    axis_means.sort_by(|(_, left), (_, right)| {
        right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
    });
    let strongest_axes = axis_means
        .iter()
        .take(2)
        .filter(|(_, value)| *value >= 0.7)
        .map(|(name, _)| *name)
        .collect();
    let weakest_axis = axis_means.last().map_or("unknown", |(name, _)| *name);
    AdditionRationale {
        strongest_axes,
        weakest_axis,
        most_compatible_member,
    }
}

pub(crate) struct DiscoveredPools {
    pub(crate) profiles: Vec<TrackProfile>,
    pub(crate) pools: Vec<DiscoveredPool>,
    pub(crate) bridges: Vec<(String, Vec<usize>)>,
    pub(crate) reference_bpm: f64,
    pub(crate) skipped: Vec<String>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn discover_track_pools(
    tracks: Vec<crate::types::Track>,
    store: &Connection,
    master_tempo: bool,
    reference_bpm: Option<f64>,
    weights: &PoolWeights,
    threshold: f64,
    min_size: usize,
    max_size: usize,
    max_pools: usize,
) -> Result<DiscoveredPools, String> {
    let built = build_pool_profiles(tracks, store);
    if built.profiles.len() < min_size {
        return Err(format!(
            "Only {} profiles built (need {min_size})",
            built.profiles.len()
        ));
    }
    let normalization = available_normalization(store);
    let bpms: Vec<_> = built.profiles.iter().map(|profile| profile.bpm).collect();
    let reference_bpm = reference_bpm.unwrap_or_else(|| median_bpm(&bpms));
    let refs: Vec<_> = built.profiles.iter().collect();
    let pools = discover_pools(
        &refs,
        master_tempo,
        reference_bpm,
        weights,
        normalization.as_ref(),
        threshold,
        min_size,
        max_size,
        max_pools,
    );
    let bridges = find_bridge_tracks(&pools);
    Ok(DiscoveredPools {
        profiles: built.profiles,
        pools,
        bridges,
        reference_bpm,
        skipped: built.skipped,
    })
}

pub(crate) fn sweep_optimal_reference_bpm(
    profiles: &[TrackProfile],
    bpms: &[f64],
) -> Option<(f64, f64)> {
    let semitone_ratio = 2.0_f64.powf(1.0 / 12.0);
    let mut interval_lo = f64::NEG_INFINITY;
    let mut interval_hi = f64::INFINITY;
    for &bpm in bpms {
        if bpm <= 0.0 {
            continue;
        }
        let lo = bpm / semitone_ratio;
        let hi = bpm * semitone_ratio;
        if lo > interval_lo {
            interval_lo = lo;
        }
        if hi < interval_hi {
            interval_hi = hi;
        }
    }
    if interval_lo > interval_hi || interval_lo <= 0.0 {
        return None;
    }
    let step = 0.1;
    let mut best_bpm = interval_lo;
    let mut best_stability = f64::NEG_INFINITY;
    let mut reference_bpm = interval_lo;
    while reference_bpm <= interval_hi {
        let stability = compute_key_stability_at_bpm(profiles, reference_bpm);
        if stability > best_stability {
            best_stability = stability;
            best_bpm = reference_bpm;
        }
        reference_bpm += step;
    }
    let stability_hi = compute_key_stability_at_bpm(profiles, interval_hi);
    if stability_hi > best_stability {
        best_stability = stability_hi;
        best_bpm = interval_hi;
    }
    (best_stability > f64::NEG_INFINITY).then_some((best_bpm, best_stability))
}

fn compute_key_stability_at_bpm(profiles: &[TrackProfile], reference_bpm: f64) -> f64 {
    if profiles.len() < 2 {
        return 1.0;
    }
    let mut sum = 0.0;
    let mut count = 0u32;
    for left in 0..profiles.len() {
        for right in (left + 1)..profiles.len() {
            let score = score_key_with_pitch_shifts(
                profiles[left].camelot_key,
                profiles[right].camelot_key,
                bpm_pitch_shift(profiles[left].bpm, reference_bpm),
                bpm_pitch_shift(profiles[right].bpm, reference_bpm),
            );
            sum += score.value;
            count += 1;
        }
    }
    if count > 0 { sum / count as f64 } else { 1.0 }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPoolWeights {
    bpm: Option<f64>,
    energy: Option<f64>,
    timbral: Option<f64>,
    key: Option<f64>,
    genre: Option<f64>,
    brightness: Option<f64>,
    rhythm: Option<f64>,
}

fn saved_pool_weights(input: SavedPoolWeights) -> Result<PoolWeights, String> {
    let base = pool_weights(crate::domain::planning::PoolPreset::Balanced);
    let mut weights = PoolWeights {
        bpm: input.bpm.unwrap_or(base.bpm),
        energy: input.energy.unwrap_or(base.energy),
        timbral: input.timbral.unwrap_or(base.timbral),
        key: input.key.unwrap_or(base.key),
        genre: input.genre.unwrap_or(base.genre),
        brightness: input.brightness.unwrap_or(base.brightness),
        rhythm: input.rhythm.unwrap_or(base.rhythm),
    };
    renormalize_pool(&mut weights)?;
    Ok(weights)
}

fn pool_builtin(name: &str) -> Option<PoolWeights> {
    let preset = match name {
        "balanced" => crate::domain::planning::PoolPreset::Balanced,
        "timbral" => crate::domain::planning::PoolPreset::Timbral,
        _ => return None,
    };
    Some(pool_weights(preset))
}

fn resolve_pool_named_with_loader(
    name: &str,
    load_saved: impl FnOnce() -> Result<Option<String>, String>,
) -> Result<PoolWeights, String> {
    if let Some(weights) = pool_builtin(name) {
        return Ok(weights);
    }
    let json = load_saved()?
        .ok_or_else(|| format!("Unknown pool preset '{name}'. Built-in: balanced, timbral"))?;
    let input: SavedPoolWeights =
        serde_json::from_str(&json).map_err(|error| format!("Invalid saved preset: {error}"))?;
    saved_pool_weights(input)
}

pub(crate) fn resolve_pool_named(name: &str, store: &Connection) -> Result<PoolWeights, String> {
    resolve_pool_named_with_loader(name, || {
        crate::adapters::state::get_weight_preset(store, name, "pool")
            .map_err(|error| format!("DB error: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn planning_pool_use_case_preserves_saved_preset_precedence() {
        let loaded = Cell::new(false);
        let weights = resolve_pool_named_with_loader("balanced", || {
            loaded.set(true);
            Ok(Some(r#"{"timbral":1.0}"#.to_string()))
        })
        .unwrap();
        assert!(!loaded.get());
        assert_eq!(weights.timbral, 0.18);
    }
}
