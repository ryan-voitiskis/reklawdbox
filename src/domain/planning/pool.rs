//! Symmetric pool scoring, cohesion, and deterministic discovery.

use std::collections::{HashMap, HashSet};

use crate::domain::classification::taxonomy::GenreFamily;

use super::{
    AxisScore, CandidatePoolScore, DiscoveredPool, PoolAxisScores, PoolCohesionResult,
    PoolDiscoveryBounds, PoolScoringPolicy, TimbralNormalization, TrackProfile, bpm_pitch_shift,
    build_timbral_vector, normalize_timbral_vector, score_brightness_axis, score_key_axis,
    score_key_with_pitch_shifts, score_rhythm_axis,
};

pub(crate) fn score_pool_bpm_axis(a_bpm: f64, b_bpm: f64) -> AxisScore {
    if a_bpm <= 0.0 || b_bpm <= 0.0 {
        return AxisScore {
            value: 0.5,
            label: "Unknown BPM".to_string(),
        };
    }
    let delta = (a_bpm - b_bpm).abs();
    let denom = a_bpm.max(b_bpm);
    let pct = delta / denom * 100.0;
    let value = (-0.019 * pct * pct).exp();
    let label_category = if pct < 2.0 {
        "Seamless"
    } else if pct < 4.0 {
        "Comfortable"
    } else if pct < 6.0 {
        "Noticeable"
    } else if pct < 9.0 {
        "Creative transition needed"
    } else {
        "Jarring"
    };
    AxisScore {
        value,
        label: format!("{label_category} ({pct:.1}%, {delta:.1} BPM)"),
    }
}

/// Gaussian decay on absolute energy distance.
pub(crate) fn score_pool_energy_axis(a_energy: f64, b_energy: f64) -> AxisScore {
    let delta = (a_energy - b_energy).abs();
    // exp(-25 * delta^2): 0.0 → 1.0, 0.1 → 0.78, 0.2 → 0.37, 0.3 → 0.11
    let value = (-25.0 * delta * delta).exp();
    let label = if delta < 0.05 {
        format!("Same energy band (delta {delta:.2})")
    } else if delta < 0.15 {
        format!("Close energy (delta {delta:.2})")
    } else if delta < 0.25 {
        format!("Moderate energy gap (delta {delta:.2})")
    } else {
        format!("Wide energy gap (delta {delta:.2})")
    };
    AxisScore { value, label }
}

/// Genre match without streak logic (1.0 / 0.7 / 0.3).
pub(crate) fn score_pool_genre_axis(
    genre_a: Option<&str>,
    genre_b: Option<&str>,
    family_a: GenreFamily,
    family_b: GenreFamily,
) -> AxisScore {
    let Some(genre_a) = genre_a else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };
    let Some(genre_b) = genre_b else {
        return AxisScore {
            value: 0.5,
            label: "Unknown genre".to_string(),
        };
    };

    if genre_a.eq_ignore_ascii_case(genre_b) {
        AxisScore {
            value: 1.0,
            label: "Same genre".to_string(),
        }
    } else if family_a == family_b && family_a != GenreFamily::Other {
        AxisScore {
            value: 0.7,
            label: "Same family".to_string(),
        }
    } else {
        AxisScore {
            value: 0.3,
            label: "Different families".to_string(),
        }
    }
}

/// Euclidean distance on z-score-normalized timbral vectors.
/// Returns None if either track lacks timbral data.
pub(crate) fn score_pool_timbral_axis(
    a: &TrackProfile,
    b: &TrackProfile,
    norm_stats: &TimbralNormalization,
) -> Option<AxisScore> {
    let raw_a = build_timbral_vector(a)?;
    let raw_b = build_timbral_vector(b)?;

    if raw_a.len() != raw_b.len() || raw_a.len() != norm_stats.dims.len() {
        return None;
    }

    let norm_a = normalize_timbral_vector(&raw_a, norm_stats)?;
    let norm_b = normalize_timbral_vector(&raw_b, norm_stats)?;

    let dist_sq: f64 = norm_a
        .iter()
        .zip(norm_b.iter())
        .map(|(a, b)| (a - b).powi(2))
        .sum();
    let dist = dist_sq.sqrt();

    // Map to [0,1] via exp(-k * dist^2). k chosen so that dist=4 → ~0.45
    // (typical "different but not extreme" in z-score space)
    let k = 0.05;
    let value = (-k * dist_sq).exp();

    let label = if value > 0.8 {
        format!("Very similar timbre (dist {dist:.1})")
    } else if value > 0.5 {
        format!("Similar timbre (dist {dist:.1})")
    } else if value > 0.3 {
        format!("Moderate timbral distance (dist {dist:.1})")
    } else {
        format!("Distant timbre (dist {dist:.1})")
    };

    Some(AxisScore { value, label })
}

pub(crate) fn score_pool_compatibility_pair(
    a: &TrackProfile,
    b: &TrackProfile,
    scoring: PoolScoringPolicy<'_>,
) -> PoolAxisScores {
    let key = if !scoring.master_tempo && scoring.reference_bpm > 0.0 {
        score_key_with_pitch_shifts(
            a.camelot_key,
            b.camelot_key,
            bpm_pitch_shift(a.bpm, scoring.reference_bpm),
            bpm_pitch_shift(b.bpm, scoring.reference_bpm),
        )
    } else {
        score_key_axis(a.camelot_key, b.camelot_key)
    };

    let bpm = score_pool_bpm_axis(a.bpm, b.bpm);
    let energy = score_pool_energy_axis(a.energy, b.energy);
    let genre = score_pool_genre_axis(
        a.canonical_genre.as_deref(),
        b.canonical_genre.as_deref(),
        a.genre_family,
        b.genre_family,
    );
    let brightness = score_brightness_axis(a.brightness, b.brightness);
    let rhythm = score_rhythm_axis(a.rhythm_regularity, b.rhythm_regularity);

    let timbral = scoring
        .timbral_normalization
        .and_then(|stats| score_pool_timbral_axis(a, b, stats));

    let brightness_available = a.brightness.is_some() && b.brightness.is_some();
    let rhythm_available = a.rhythm_regularity.is_some() && b.rhythm_regularity.is_some();
    let mut weighted_sum = (scoring.weights.bpm * bpm.value)
        + (scoring.weights.energy * energy.value)
        + (scoring.weights.key * key.value)
        + (scoring.weights.genre * genre.value);
    let mut total_weight =
        scoring.weights.bpm + scoring.weights.energy + scoring.weights.key + scoring.weights.genre;

    if brightness_available {
        weighted_sum += scoring.weights.brightness * brightness.value;
        total_weight += scoring.weights.brightness;
    }
    if rhythm_available {
        weighted_sum += scoring.weights.rhythm * rhythm.value;
        total_weight += scoring.weights.rhythm;
    }
    if let Some(ref t) = timbral {
        weighted_sum += scoring.weights.timbral * t.value;
        total_weight += scoring.weights.timbral;
    }

    let composite = if total_weight > f64::EPSILON {
        weighted_sum / total_weight
    } else {
        0.0
    };

    PoolAxisScores {
        key,
        bpm,
        energy,
        genre,
        brightness,
        rhythm,
        timbral,
        composite,
    }
}

pub(crate) fn score_candidate_vs_pool(
    candidate: &TrackProfile,
    pool: &[&TrackProfile],
    scoring: PoolScoringPolicy<'_>,
) -> CandidatePoolScore {
    let mut min_score = f64::INFINITY;
    let mut sum = 0.0;
    let mut per_member = Vec::with_capacity(pool.len());

    for member in pool {
        let scores = score_pool_compatibility_pair(candidate, member, scoring);
        if scores.composite < min_score {
            min_score = scores.composite;
        }
        sum += scores.composite;
        per_member.push((member.track.id.clone(), scores));
    }

    let mean_score = if pool.is_empty() {
        0.0
    } else {
        sum / pool.len() as f64
    };

    CandidatePoolScore {
        min_score: if min_score.is_infinite() {
            0.0
        } else {
            min_score
        },
        mean_score,
        per_member,
    }
}

pub(crate) fn compute_pool_cohesion(
    profiles: &[&TrackProfile],
    scoring: PoolScoringPolicy<'_>,
) -> PoolCohesionResult {
    let n = profiles.len();
    if n < 2 {
        return PoolCohesionResult {
            mean_pairwise: 1.0,
            min_pairwise: 1.0,
            weakest_member_id: None,
            medoid_id: profiles.first().map(|p| p.track.id.clone()),
            per_pair: Vec::new(),
        };
    }

    let mut per_pair = Vec::with_capacity(n * (n - 1) / 2);
    let mut global_min = f64::INFINITY;
    let mut global_sum = 0.0;
    let pair_count = n * (n - 1) / 2;

    let mut member_min: Vec<f64> = vec![f64::INFINITY; n];
    let mut member_sum: Vec<f64> = vec![0.0; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let scores = score_pool_compatibility_pair(profiles[i], profiles[j], scoring);
            let c = scores.composite;

            if c < global_min {
                global_min = c;
            }
            global_sum += c;

            if c < member_min[i] {
                member_min[i] = c;
            }
            if c < member_min[j] {
                member_min[j] = c;
            }
            member_sum[i] += c;
            member_sum[j] += c;

            per_pair.push((
                profiles[i].track.id.clone(),
                profiles[j].track.id.clone(),
                scores,
            ));
        }
    }

    let mean_pairwise = if pair_count > 0 {
        global_sum / pair_count as f64
    } else {
        0.0
    };

    let weakest_idx = member_min
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i);

    let medoid_idx = member_sum
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(i, _)| i);

    PoolCohesionResult {
        mean_pairwise,
        min_pairwise: if global_min.is_infinite() {
            0.0
        } else {
            global_min
        },
        weakest_member_id: weakest_idx.map(|i| profiles[i].track.id.clone()),
        medoid_id: medoid_idx.map(|i| profiles[i].track.id.clone()),
        per_pair,
    }
}

// ---------------------------------------------------------------------------
// Pool discovery — Bron-Kerbosch maximal clique enumeration
// ---------------------------------------------------------------------------

pub(crate) fn discover_pools(
    profiles: &[&TrackProfile],
    scoring: PoolScoringPolicy<'_>,
    bounds: PoolDiscoveryBounds,
) -> Vec<DiscoveredPool> {
    let n = profiles.len();
    if n < bounds.minimum_size() {
        return Vec::new();
    }

    let mut compat: Vec<Vec<f64>> = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in (i + 1)..n {
            let s = score_pool_compatibility_pair(profiles[i], profiles[j], scoring);
            compat[i][j] = s.composite;
            compat[j][i] = s.composite;
        }
    }

    let mut adj: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for i in 0..n {
        for j in (i + 1)..n {
            if compat[i][j] >= bounds.threshold() {
                adj[i].insert(j);
                adj[j].insert(i);
            }
        }
    }

    let mut cliques: Vec<Vec<usize>> = Vec::new();
    let all: HashSet<usize> = (0..n).collect();
    bron_kerbosch_pivot(
        &adj,
        &mut HashSet::new(),
        &mut all.clone(),
        &mut HashSet::new(),
        &mut cliques,
        bounds.maximum_size(),
    );

    let mut pools: Vec<DiscoveredPool> = cliques
        .into_iter()
        .filter(|c| c.len() >= bounds.minimum_size() && c.len() <= bounds.maximum_size())
        .map(|c| build_discovered_pool(&c, profiles, &compat))
        .collect();

    pools.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.track_ids.len().cmp(&a.track_ids.len()))
    });

    let mut selected: Vec<DiscoveredPool> = Vec::new();
    for pool in pools {
        if selected.len() >= bounds.maximum_results() {
            break;
        }
        let set: HashSet<&str> = pool
            .track_ids
            .iter()
            .map(std::string::String::as_str)
            .collect();
        let is_subset = selected.iter().any(|s| {
            let ss: HashSet<&str> = s
                .track_ids
                .iter()
                .map(std::string::String::as_str)
                .collect();
            set.is_subset(&ss)
        });
        if !is_subset {
            selected.push(pool);
        }
    }
    selected
}

fn build_discovered_pool(
    clique: &[usize],
    profiles: &[&TrackProfile],
    compat: &[Vec<f64>],
) -> DiscoveredPool {
    let (mean_c, min_c) = clique_compatibility(clique, compat);
    let size_bonus = match clique.len() {
        2..=3 => 0.85,
        4..=8 => 1.0,
        _ => 0.90,
    };

    let mut member_means: Vec<(usize, f64)> = clique
        .iter()
        .map(|&i| {
            let sum: f64 = clique
                .iter()
                .filter(|&&j| j != i)
                .map(|&j| compat[i][j])
                .sum();
            (i, sum / (clique.len() - 1).max(1) as f64)
        })
        .collect();
    member_means.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let median_mean = member_means
        .get((member_means.len().saturating_sub(1)) / 2)
        .map_or(0.0, |m| m.1);

    DiscoveredPool {
        track_ids: clique
            .iter()
            .map(|&i| profiles[i].track.id.clone())
            .collect(),
        mean_compatibility: mean_c,
        min_compatibility: min_c,
        core_members: member_means
            .iter()
            .filter(|(_, m)| *m >= median_mean)
            .map(|(i, _)| profiles[*i].track.id.clone())
            .collect(),
        edge_members: member_means
            .iter()
            .filter(|(_, m)| *m < median_mean)
            .map(|(i, _)| profiles[*i].track.id.clone())
            .collect(),
        score: mean_c * size_bonus,
    }
}

const MAX_CLIQUES: usize = 50_000;

fn bron_kerbosch_pivot(
    adj: &[HashSet<usize>],
    r: &mut HashSet<usize>,
    p: &mut HashSet<usize>,
    x: &mut HashSet<usize>,
    cliques: &mut Vec<Vec<usize>>,
    max_size: usize,
) {
    if cliques.len() >= MAX_CLIQUES {
        return;
    }
    if p.is_empty() && x.is_empty() {
        if r.len() >= 2 {
            let mut c: Vec<usize> = r.iter().copied().collect();
            c.sort_unstable();
            cliques.push(c);
        }
        return;
    }
    if r.len() >= max_size {
        let mut c: Vec<usize> = r.iter().copied().collect();
        c.sort_unstable();
        cliques.push(c);
        return;
    }

    let pivot = p
        .union(x)
        .max_by_key(|&&v| adj[v].intersection(p).count())
        .copied();
    let Some(pivot) = pivot else { return };

    let candidates: Vec<usize> = p.difference(&adj[pivot]).copied().collect();
    for v in candidates {
        r.insert(v);
        let mut new_p: HashSet<usize> = p.intersection(&adj[v]).copied().collect();
        let mut new_x: HashSet<usize> = x.intersection(&adj[v]).copied().collect();
        bron_kerbosch_pivot(adj, r, &mut new_p, &mut new_x, cliques, max_size);
        r.remove(&v);
        p.remove(&v);
        x.insert(v);
    }
}

fn clique_compatibility(clique: &[usize], compat: &[Vec<f64>]) -> (f64, f64) {
    let mut sum = 0.0;
    let mut min = f64::INFINITY;
    let mut count = 0u32;
    for (idx, &i) in clique.iter().enumerate() {
        for &j in &clique[idx + 1..] {
            sum += compat[i][j];
            if compat[i][j] < min {
                min = compat[i][j];
            }
            count += 1;
        }
    }
    if count > 0 {
        (sum / count as f64, min)
    } else {
        (0.0, 0.0)
    }
}

pub(crate) fn find_bridge_tracks(pools: &[DiscoveredPool]) -> Vec<(String, Vec<usize>)> {
    let mut track_pools: HashMap<String, Vec<usize>> = HashMap::new();
    for (idx, pool) in pools.iter().enumerate() {
        for id in &pool.track_ids {
            track_pools.entry(id.clone()).or_default().push(idx);
        }
    }
    let mut bridges: Vec<(String, Vec<usize>)> = track_pools
        .into_iter()
        .filter(|(_, p)| p.len() >= 2)
        .collect();
    bridges.sort_by_key(|b| std::cmp::Reverse(b.1.len()));
    bridges
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_pool_bridge_order_preserves_membership_count() {
        let pool = |track_ids: &[&str]| DiscoveredPool {
            track_ids: track_ids.iter().map(|id| (*id).to_string()).collect(),
            mean_compatibility: 1.0,
            min_compatibility: 1.0,
            core_members: Vec::new(),
            edge_members: Vec::new(),
            score: 1.0,
        };
        let bridges = find_bridge_tracks(&[
            pool(&["shared", "twice"]),
            pool(&["shared", "twice"]),
            pool(&["shared"]),
        ]);
        assert_eq!(bridges[0], ("shared".to_string(), vec![0, 1, 2]));
        assert_eq!(bridges[1], ("twice".to_string(), vec![0, 1]));
    }
}
