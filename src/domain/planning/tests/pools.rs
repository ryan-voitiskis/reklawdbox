use std::collections::HashSet;

use crate::domain::classification::taxonomy::GenreFamily;
use crate::domain::planning::*;

use super::support::{
    ProfileAnalysis, ProfileSpec, pool_discovery_bounds, pool_scoring_policy, simple_profile,
    synth_profile,
};

fn timbral_profile(spec: ProfileSpec<'_>, timbral: TimbralFeatures) -> TrackProfile {
    let mut profile = synth_profile(spec, ProfileAnalysis::measured(2000.0, 0.6, 7.0));
    profile.timbral = Some(timbral);
    profile
}

fn dummy_norm_stats(dims: usize) -> TimbralNormalization {
    TimbralNormalization {
        dims: vec![(0.0, 1.0); dims],
        sample_count: 100,
    }
}

#[test]
fn planning_pool_scoring_is_symmetric() {
    let a = simple_profile("sym-a", "8A", 126.0, 0.6, "House");
    let b = simple_profile("sym-b", "10A", 128.0, 0.7, "Tech House");

    let ab = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
    );
    let ba = score_pool_compatibility_pair(
        &b,
        &a,
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_pool_planted_cluster_separation() {
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
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
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
                pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_expand_pool_greedy_selects_compatible_tracks() {
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
                pool_scoring_policy(true, ref_bpm, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_pool_energy_axis_gaussian() {
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

#[test]
fn planning_pool_eval_build_timbral_vector_requires_all_fields() {
    let full = timbral_profile(
        ProfileSpec::new("tv1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![0.0; 13],
            mfcc_std: vec![0.0; 13],
            spectral_contrast_mean: vec![0.0; 6],
            spectral_centroid_cv: 0.5,
            dissonance_mean: 0.3,
        },
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
fn planning_pool_eval_normalize_timbral_vector_zscore() {
    let stats = TimbralNormalization {
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
fn planning_pool_eval_normalize_timbral_vector_dimension_mismatch() {
    let stats = TimbralNormalization {
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
fn planning_pool_eval_pool_timbral_axis_identical_vectors() {
    let dims = 34;
    let stats = dummy_norm_stats(dims);
    let a = timbral_profile(
        ProfileSpec::new("ta1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
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
fn planning_pool_eval_pool_timbral_axis_distant_vectors() {
    let dims = 34;
    let stats = dummy_norm_stats(dims);
    let a = timbral_profile(
        ProfileSpec::new("td1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![0.0; 13],
            mfcc_std: vec![0.0; 13],
            spectral_contrast_mean: vec![0.0; 6],
            spectral_centroid_cv: 0.0,
            dissonance_mean: 0.0,
        },
    );
    let b = timbral_profile(
        ProfileSpec::new("td2", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![5.0; 13],
            mfcc_std: vec![5.0; 13],
            spectral_contrast_mean: vec![5.0; 6],
            spectral_centroid_cv: 5.0,
            dissonance_mean: 5.0,
        },
    );
    let score = score_pool_timbral_axis(&a, &b, &stats);
    assert!(score.is_some());
    assert!(
        score.unwrap().value < 0.3,
        "distant vectors should score low",
    );
}

#[test]
fn planning_pool_eval_pool_timbral_axis_missing_data_returns_none() {
    let dims = 34;
    let stats = dummy_norm_stats(dims);
    let with_data = timbral_profile(
        ProfileSpec::new("tm1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );
    let without_data = simple_profile("tm2", "8A", 126.0, 0.5, "House");

    assert!(
        score_pool_timbral_axis(&with_data, &without_data, &stats).is_none(),
        "missing timbral data should return None",
    );
}

#[test]
fn planning_pool_eval_pool_timbral_axis_dimension_mismatch_returns_none() {
    let stats = dummy_norm_stats(34);
    let a = timbral_profile(
        ProfileSpec::new("tdm1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 2],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );
    let b = a.clone();
    assert!(
        score_pool_timbral_axis(&a, &b, &stats).is_none(),
        "dimension mismatch with norm stats should return None",
    );
}

#[test]
fn planning_pool_eval_pool_composite_with_vs_without_timbral() {
    let dims = 34;
    let stats = dummy_norm_stats(dims);

    let a_timbral = timbral_profile(
        ProfileSpec::new("ct1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );
    let b_timbral = timbral_profile(
        ProfileSpec::new("ct2", "9A", 126.5, 0.55, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );

    let with_timbral = score_pool_compatibility_pair(
        &a_timbral,
        &b_timbral,
        pool_scoring_policy(
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            Some(&stats),
        ),
    );
    let without_timbral = score_pool_compatibility_pair(
        &a_timbral,
        &b_timbral,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_pool_preset_timbral_vs_balanced() {
    let dims = 34;
    let stats = dummy_norm_stats(dims);

    let a = timbral_profile(
        ProfileSpec::new("pt1", "8A", 126.0, 0.5, "House"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );
    let b = timbral_profile(
        ProfileSpec::new("pt2", "2A", 126.0, 0.5, "Techno"),
        TimbralFeatures {
            mfcc_mean: vec![1.0; 13],
            mfcc_std: vec![0.5; 13],
            spectral_contrast_mean: vec![0.3; 6],
            spectral_centroid_cv: 0.4,
            dissonance_mean: 0.2,
        },
    );

    let balanced = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(
            true,
            126.0,
            &pool_weights(PoolPreset::Balanced),
            Some(&stats),
        ),
    );
    let timbral = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(
            true,
            126.0,
            &pool_weights(PoolPreset::Timbral),
            Some(&stats),
        ),
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
fn planning_pool_eval_expand_pool_stops_below_quality_threshold() {
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
                pool_scoring_policy(true, ref_bpm, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_pool_cohesion_single_track() {
    let profiles = [simple_profile("single", "8A", 126.0, 0.5, "House")];
    let refs: Vec<&TrackProfile> = profiles.iter().collect();
    let result = compute_pool_cohesion(
        &refs,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
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
fn planning_pool_eval_candidate_vs_empty_pool() {
    let candidate = simple_profile("c", "8A", 126.0, 0.5, "House");
    let result = score_candidate_vs_pool(
        &candidate,
        &[],
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
    );
    assert!(
        (result.mean_score - 0.0).abs() < 1e-10,
        "empty pool should give mean 0.0",
    );
    assert!(result.per_member.is_empty());
}

#[test]
fn planning_pool_eval_pool_scoring_master_tempo_off_changes_key() {
    let a = simple_profile("mto-a", "8A", 126.0, 0.5, "House");
    let b = simple_profile("mto-b", "8A", 132.0, 0.5, "House");

    let mt_on = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(true, 129.0, &pool_weights(PoolPreset::Balanced), None),
    );
    assert!(
        mt_on.key.value > 0.9,
        "master_tempo ON, same key should score high: {:.3}",
        mt_on.key.value,
    );

    let mt_off = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(false, 129.0, &pool_weights(PoolPreset::Balanced), None),
    );

    assert!(
        (mt_on.key.value - mt_off.key.value).abs() > 0.01,
        "master_tempo OFF should change key scoring: on={:.3} off={:.3}",
        mt_on.key.value,
        mt_off.key.value,
    );
}

#[test]
fn planning_pool_eval_pool_scoring_master_tempo_off_symmetric() {
    let a = simple_profile("mts-a", "8A", 124.0, 0.5, "House");
    let b = simple_profile("mts-b", "10A", 128.0, 0.55, "Deep House");

    let ab = score_pool_compatibility_pair(
        &a,
        &b,
        pool_scoring_policy(false, 126.0, &pool_weights(PoolPreset::Balanced), None),
    );
    let ba = score_pool_compatibility_pair(
        &b,
        &a,
        pool_scoring_policy(false, 126.0, &pool_weights(PoolPreset::Balanced), None),
    );

    assert!(
        (ab.composite - ba.composite).abs() < 0.01,
        "pool score should be symmetric with master_tempo off: A→B={:.6}, B→A={:.6}",
        ab.composite,
        ba.composite,
    );
}

#[test]
fn planning_pool_eval_pool_genre_axis_unknown() {
    let score = score_pool_genre_axis(None, Some("House"), GenreFamily::Other, GenreFamily::House);
    assert!(
        (score.value - 0.5).abs() < 1e-10,
        "unknown genre should score 0.5, got {:.3}",
        score.value,
    );
}

#[test]
fn planning_pool_eval_pool_genre_axis_other_family_not_matched() {
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
fn planning_pool_discovery_finds_planted_clusters() {
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
        pool_scoring_policy(true, 130.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.65, 3, 12, 10),
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
        let pool_ids: HashSet<&str> = pool
            .track_ids
            .iter()
            .map(std::string::String::as_str)
            .collect();
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
fn planning_pool_discovery_bounds_validate_domain_relationships() {
    assert!(PoolDiscoveryBounds::new(f64::NAN, 3, 12, 10).is_none());
    assert!(PoolDiscoveryBounds::new(-0.1, 3, 12, 10).is_none());
    assert!(PoolDiscoveryBounds::new(1.1, 3, 12, 10).is_none());
    assert!(PoolDiscoveryBounds::new(0.7, 1, 12, 10).is_none());
    assert!(PoolDiscoveryBounds::new(0.7, 4, 3, 10).is_none());

    let zero_results = PoolDiscoveryBounds::new(0.7, 3, 12, 0)
        .expect("zero max results preserves the existing empty-result behavior");
    assert_eq!(zero_results.maximum_results(), 0);
}

#[test]
fn planning_pool_discovery_respects_min_max_size() {
    let profiles: Vec<TrackProfile> = (0..8)
        .map(|i| simple_profile(&format!("sz{i}"), "8A", 126.0, 0.5, "House"))
        .collect();
    let refs: Vec<&TrackProfile> = profiles.iter().collect();

    let pools = discover_pools(
        &refs,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.5, 4, 6, 10),
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
fn planning_pool_eval_discover_pools_empty_below_min_size() {
    let profiles = [simple_profile("small1", "8A", 126.0, 0.5, "House")];
    let refs: Vec<&TrackProfile> = profiles.iter().collect();

    let pools = discover_pools(
        &refs,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.7, 3, 12, 10),
    );
    assert!(
        pools.is_empty(),
        "should return no pools for 1 track with min_size=3"
    );
}

#[test]
fn planning_pool_eval_discover_pools_high_threshold_yields_fewer_pools() {
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
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.5, 2, 12, 10),
    );
    let pools_high = discover_pools(
        &refs,
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.85, 2, 12, 10),
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
fn planning_pool_eval_find_bridge_tracks() {
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
fn planning_pool_discovery_respects_max_pool_count() {
    let profiles: Vec<TrackProfile> = (0..8)
        .map(|i| simple_profile(&format!("mc{i}"), "8A", 126.0, 0.5, "House"))
        .collect();
    let refs: Vec<&TrackProfile> = profiles.iter().collect();

    let pools = discover_pools(
        &refs,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.5, 2, 12, 2),
    );
    assert!(
        pools.len() <= 2,
        "max_pools=2 should cap output, got {}",
        pools.len(),
    );
    assert!(!pools.is_empty(), "should find at least one pool");
}

#[test]
fn planning_pool_eval_discover_pools_no_subset_duplicates() {
    let profiles: Vec<TrackProfile> = (0..8)
        .map(|i| simple_profile(&format!("sd{i}"), "8A", 126.0, 0.5, "House"))
        .collect();
    let refs: Vec<&TrackProfile> = profiles.iter().collect();

    let pools = discover_pools(
        &refs,
        pool_scoring_policy(true, 126.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.5, 2, 8, 20),
    );

    for (i, a) in pools.iter().enumerate() {
        let a_set: HashSet<&str> = a
            .track_ids
            .iter()
            .map(std::string::String::as_str)
            .collect();
        for (j, b) in pools.iter().enumerate() {
            if i == j {
                continue;
            }
            let b_set: HashSet<&str> = b
                .track_ids
                .iter()
                .map(std::string::String::as_str)
                .collect();
            assert!(
                !a_set.is_subset(&b_set),
                "pool {i} should not be a subset of pool {j}",
            );
        }
    }
}

#[test]
fn planning_pool_eval_discover_pools_all_incompatible() {
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
        pool_scoring_policy(true, 120.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.8, 2, 12, 10),
    );
    assert!(
        pools.is_empty(),
        "all-incompatible tracks should produce no pools at threshold 0.8, got {}",
        pools.len(),
    );
}

#[test]
fn planning_pool_eval_discover_pools_core_edge_classification() {
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
        pool_scoring_policy(true, 127.0, &pool_weights(PoolPreset::Balanced), None),
        pool_discovery_bounds(0.5, 3, 12, 10),
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
