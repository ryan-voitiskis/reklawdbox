//! Genre Audio Profiles — Fisher discriminant scoring for genre classification.
//!
//! Builds per-genre prototypes from verified tracks using Fisher discriminant
//! weights. At classification time, computes weighted distance from a track's
//! audio features to each prototype and returns per-genre affinity scores that
//! inject as votes into the existing `gather_votes` pipeline.

use std::collections::HashMap;

use rusqlite::Connection;
use serde::Serialize;

use crate::classify::AudioFeatures;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum vote weight for audio-profile votes (below Beatport 1.0, label 0.6).
const AFFINITY_CAP: f32 = 0.5;
/// Tracks beyond this many weighted stddev from prototype score 0.
const SCALE: f64 = 2.5;
/// Minimum verified tracks to generate a prototype.
pub(crate) const MIN_TRACKS: u32 = 5;
/// No single feature dominates scoring.
const FISHER_WEIGHT_CAP: f64 = 0.4;
/// Mild regularization floor — no feature fully ignored.
const FISHER_WEIGHT_FLOOR: f64 = 0.02;

/// Ordered list of scalar feature names used for scoring.
/// Must match the extraction order in `extract_scalar_features`.
const SCALAR_FEATURE_NAMES: &[&str] = &[
    "rekordbox_bpm",
    "danceability",
    "onset_rate",
    "rhythm_regularity",
    "spectral_centroid_mean",
    "spectral_centroid_cv",
    "dynamic_complexity",
    "loudness_integrated",
    "decay_mid_tau",
    "decay_high_tau",
    "spectral_flux_mean",
    "dissonance_mean",
    "key_clarity",
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Per-feature statistics for a genre prototype.
#[derive(Debug, Clone)]
pub(crate) struct FeatureStat {
    pub(crate) mean: f64,
    pub(crate) stddev: f64,
    pub(crate) fisher_weight: f64,
    pub(crate) n: u32,
}

/// A complete genre prototype with scalar features and optional timbral centroids.
#[derive(Debug, Clone)]
pub(crate) struct GenrePrototype {
    pub(crate) genre: &'static str,
    pub(crate) features: HashMap<&'static str, FeatureStat>,
    /// MFCC mean centroid (coefficients 1-8) for timbral distance.
    pub(crate) mfcc_centroid: Option<Vec<f64>>,
    /// MFCC std centroid (coefficients 1-5) for variability distance.
    pub(crate) mfcc_std_centroid: Option<Vec<f64>>,
    /// Spectral contrast centroid (bands 0, 2, 4) for shape distance.
    pub(crate) contrast_centroid: Option<Vec<f64>>,
    /// Mean within-genre MFCC distance (for normalizing timbral distances).
    pub(crate) mfcc_mean_dist: Option<f64>,
    pub(crate) mfcc_std_mean_dist: Option<f64>,
    pub(crate) contrast_mean_dist: Option<f64>,
    pub(crate) total_n: u32,
}

/// Global statistics for normalization and regularization.
#[derive(Debug, Clone)]
pub(crate) struct ProfileRegistry {
    pub(crate) prototypes: HashMap<&'static str, GenrePrototype>,
    /// Global stats per feature: (mean, stddev).
    pub(crate) global_stats: HashMap<&'static str, (f64, f64)>,
}

/// Per-genre audio affinity score for a single track.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AudioAffinity {
    pub(crate) genre: &'static str,
    pub(crate) distance: f64,
    pub(crate) vote_weight: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) contributions: Vec<FeatureContribution>,
}

/// How much a single feature contributed to the affinity score.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FeatureContribution {
    pub(crate) name: &'static str,
    pub(crate) track_value: f64,
    pub(crate) proto_mean: f64,
    pub(crate) z_score: f64,
    pub(crate) fisher_weight: f64,
}

// ---------------------------------------------------------------------------
// Feature extraction
// ---------------------------------------------------------------------------

/// Filter out NaN/infinity — convert to None which the scoring code handles gracefully.
fn finite(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite())
}

/// Extract scalar features from AudioFeatures in the canonical order.
/// Must match `SCALAR_FEATURE_NAMES` in both order and count.
fn extract_scalar_features(audio: &AudioFeatures) -> Vec<Option<f64>> {
    let result = vec![
        finite(Some(audio.rekordbox_bpm)),
        finite(audio.danceability),
        finite(audio.onset_rate),
        finite(audio.rhythm_regularity),
        finite(audio.spectral_centroid_mean),
        finite(audio.spectral_centroid_cv),
        finite(audio.dynamic_complexity),
        finite(audio.loudness_integrated),
        finite(audio.decay_mid_tau),
        finite(audio.decay_high_tau),
        finite(audio.spectral_flux_mean),
        finite(audio.dissonance_mean),
        finite(audio.key_clarity),
    ];
    debug_assert_eq!(
        result.len(),
        SCALAR_FEATURE_NAMES.len(),
        "SCALAR_FEATURE_NAMES and extract_scalar_features are out of sync"
    );
    result
}

/// Extract MFCC mean coefficients 1-8 (skip index 0 and 9-12).
fn extract_mfcc_mean(audio: &AudioFeatures) -> Option<Vec<f64>> {
    audio.mfcc_mean.as_ref().and_then(|v| {
        if v.len() >= 9 {
            Some(v[1..9].to_vec())
        } else {
            None
        }
    })
}

/// Extract MFCC std coefficients 1-5 (skip index 0 and 6-12).
fn extract_mfcc_std(audio: &AudioFeatures) -> Option<Vec<f64>> {
    audio.mfcc_std.as_ref().and_then(|v| {
        if v.len() >= 6 {
            Some(v[1..6].to_vec())
        } else {
            None
        }
    })
}

/// Extract spectral contrast bands 0, 2, 4.
fn extract_contrast(audio: &AudioFeatures) -> Option<Vec<f64>> {
    audio.spectral_contrast_mean.as_ref().and_then(|v| {
        if v.len() >= 5 {
            Some(vec![v[0], v[2], v[4]])
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

/// Per-track data for calibration.
struct TrackSample {
    genre: &'static str,
    scalars: Vec<Option<f64>>,
    mfcc_mean: Option<Vec<f64>>,
    mfcc_std: Option<Vec<f64>>,
    contrast: Option<Vec<f64>>,
}

fn euclidean_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// Compute mean for a set of Option<f64> values, ignoring None.
fn mean_of(values: &[Option<f64>]) -> Option<f64> {
    let (sum, count) = values.iter().fold((0.0, 0usize), |(s, c), v| {
        if let Some(val) = v {
            (s + val, c + 1)
        } else {
            (s, c)
        }
    });
    if count >= 2 {
        Some(sum / count as f64)
    } else {
        None
    }
}

/// Compute population stddev for a set of Option<f64> values, ignoring None.
fn stddev_of(values: &[Option<f64>], mean: f64) -> Option<f64> {
    let (sum_sq, count) = values.iter().fold((0.0, 0usize), |(s, c), v| {
        if let Some(val) = v {
            (s + (val - mean).powi(2), c + 1)
        } else {
            (s, c)
        }
    });
    if count >= 2 {
        Some((sum_sq / count as f64).sqrt())
    } else {
        None
    }
}

/// Compute element-wise centroid of a list of vectors.
fn vector_centroid(vecs: &[Vec<f64>]) -> Vec<f64> {
    if vecs.is_empty() {
        return vec![];
    }
    let dim = vecs[0].len();
    let n = vecs.len() as f64;
    (0..dim)
        .map(|i| vecs.iter().map(|v| v[i]).sum::<f64>() / n)
        .collect()
}

/// Calibrate genre audio profiles from verified tracks.
///
/// Returns a `ProfileRegistry` with per-genre prototypes and global stats.
pub(crate) fn calibrate(samples: &[(&'static str, &AudioFeatures)]) -> ProfileRegistry {
    // Collect samples
    let track_samples: Vec<TrackSample> = samples
        .iter()
        .map(|(genre, audio)| TrackSample {
            genre,
            scalars: extract_scalar_features(audio),
            mfcc_mean: extract_mfcc_mean(audio),
            mfcc_std: extract_mfcc_std(audio),
            contrast: extract_contrast(audio),
        })
        .collect();

    let n_features = SCALAR_FEATURE_NAMES.len();

    // 1. Compute global stats per feature
    let mut global_stats: HashMap<&'static str, (f64, f64)> = HashMap::new();
    for (fi, &fname) in SCALAR_FEATURE_NAMES.iter().enumerate() {
        let values: Vec<Option<f64>> = track_samples.iter().map(|s| s.scalars[fi]).collect();
        if let (Some(m), Some(s)) = (mean_of(&values), {
            let m = mean_of(&values);
            m.and_then(|mv| stddev_of(&values, mv))
        }) {
            global_stats.insert(fname, (m, s.max(1e-10)));
        }
    }

    // 2. Group by genre
    let mut genre_groups: HashMap<&'static str, Vec<&TrackSample>> = HashMap::new();
    for s in &track_samples {
        genre_groups.entry(s.genre).or_default().push(s);
    }

    // 3. Build prototypes
    let mut prototypes: HashMap<&'static str, GenrePrototype> = HashMap::new();

    for (&genre, group) in &genre_groups {
        let n = group.len() as u32;
        if n < MIN_TRACKS {
            continue;
        }

        let max_features = (n as usize / 5).min(n_features);
        let mut features: HashMap<&'static str, FeatureStat> = HashMap::new();

        // Compute per-feature stats and Fisher scores
        let mut fisher_scores: Vec<(&'static str, f64, FeatureStat)> = Vec::new();

        for (fi, &fname) in SCALAR_FEATURE_NAMES.iter().enumerate() {
            let genre_values: Vec<Option<f64>> = group.iter().map(|s| s.scalars[fi]).collect();
            let genre_mean = match mean_of(&genre_values) {
                Some(m) => m,
                None => continue,
            };
            let genre_var = stddev_of(&genre_values, genre_mean)
                .map(|s| s * s)
                .unwrap_or(0.0);

            let (global_mean, global_std) = match global_stats.get(fname) {
                Some(&(m, s)) => (m, s),
                None => continue,
            };
            let global_var = global_std * global_std;

            // Regularize variance: alpha * var_sample + (1-alpha) * var_global
            let alpha = (n as f64 - 1.0) / (n as f64 - 1.0 + n_features as f64 / 2.0);
            let var_reg = alpha * genre_var + (1.0 - alpha) * global_var;

            // Fisher score
            let fisher = (genre_mean - global_mean).powi(2) / (var_reg + global_var).max(1e-10);

            let stat = FeatureStat {
                mean: genre_mean,
                stddev: var_reg.sqrt().max(1e-10),
                fisher_weight: fisher, // will be normalized below
                n,
            };
            fisher_scores.push((fname, fisher, stat));
        }

        // Sort by Fisher score descending, keep top max_features
        fisher_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        fisher_scores.truncate(max_features);

        // Normalize weights: cap, floor, then normalize to sum to 1.0
        let total_fisher: f64 = fisher_scores.iter().map(|(_, f, _)| f).sum();
        if total_fisher < 1e-10 {
            continue; // no discriminative features
        }

        for (fname, _, mut stat) in fisher_scores {
            let w =
                (stat.fisher_weight / total_fisher).clamp(FISHER_WEIGHT_FLOOR, FISHER_WEIGHT_CAP);
            stat.fisher_weight = w;
            features.insert(fname, stat);
        }

        // Re-normalize after cap/floor
        let total_w: f64 = features.values().map(|s| s.fisher_weight).sum();
        if total_w > 0.0 {
            for stat in features.values_mut() {
                stat.fisher_weight /= total_w;
            }
        }

        // Timbral centroids (only for genres with N >= 10)
        let (mfcc_centroid, mfcc_mean_dist) = if n >= 10 {
            let vecs: Vec<Vec<f64>> = group.iter().filter_map(|s| s.mfcc_mean.clone()).collect();
            if vecs.len() >= 5 {
                let centroid = vector_centroid(&vecs);
                let mean_dist = vecs
                    .iter()
                    .map(|v| euclidean_distance(v, &centroid))
                    .sum::<f64>()
                    / vecs.len() as f64;
                (Some(centroid), Some(mean_dist.max(1e-10)))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let (mfcc_std_centroid, mfcc_std_mean_dist) = if n >= 10 {
            let vecs: Vec<Vec<f64>> = group.iter().filter_map(|s| s.mfcc_std.clone()).collect();
            if vecs.len() >= 5 {
                let centroid = vector_centroid(&vecs);
                let mean_dist = vecs
                    .iter()
                    .map(|v| euclidean_distance(v, &centroid))
                    .sum::<f64>()
                    / vecs.len() as f64;
                (Some(centroid), Some(mean_dist.max(1e-10)))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let (contrast_centroid, contrast_mean_dist) = if n >= 10 {
            let vecs: Vec<Vec<f64>> = group.iter().filter_map(|s| s.contrast.clone()).collect();
            if vecs.len() >= 5 {
                let centroid = vector_centroid(&vecs);
                let mean_dist = vecs
                    .iter()
                    .map(|v| euclidean_distance(v, &centroid))
                    .sum::<f64>()
                    / vecs.len() as f64;
                (Some(centroid), Some(mean_dist.max(1e-10)))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        prototypes.insert(
            genre,
            GenrePrototype {
                genre,
                features,
                mfcc_centroid,
                mfcc_std_centroid,
                contrast_centroid,
                mfcc_mean_dist,
                mfcc_std_mean_dist,
                contrast_mean_dist,
                total_n: n,
            },
        );
    }

    ProfileRegistry {
        prototypes,
        global_stats,
    }
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

/// Score a track against all genre prototypes. Returns affinities sorted by
/// vote weight descending, filtered to weight >= 0.05.
pub(crate) fn score_all(audio: &AudioFeatures, registry: &ProfileRegistry) -> Vec<AudioAffinity> {
    let scalars = extract_scalar_features(audio);
    let mfcc = extract_mfcc_mean(audio);
    let mfcc_s = extract_mfcc_std(audio);
    let contrast = extract_contrast(audio);

    let mut affinities: Vec<AudioAffinity> = registry
        .prototypes
        .values()
        .map(|proto| score_track(&scalars, &mfcc, &mfcc_s, &contrast, proto, registry))
        .filter(|a| a.vote_weight >= 0.05)
        .collect();

    affinities.sort_by(|a, b| {
        b.vote_weight
            .partial_cmp(&a.vote_weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    affinities
}

fn score_track(
    scalars: &[Option<f64>],
    mfcc: &Option<Vec<f64>>,
    mfcc_s: &Option<Vec<f64>>,
    contrast: &Option<Vec<f64>>,
    proto: &GenrePrototype,
    registry: &ProfileRegistry,
) -> AudioAffinity {
    let mut total_contribution = 0.0;
    let mut contributions = Vec::new();

    // Tier 1: Scalar features
    for (fi, &fname) in SCALAR_FEATURE_NAMES.iter().enumerate() {
        let track_val = match scalars[fi] {
            Some(v) => v,
            None => continue,
        };
        let stat = match proto.features.get(fname) {
            Some(s) if s.fisher_weight > 0.0 => s,
            _ => continue,
        };

        let global_std = registry
            .global_stats
            .get(fname)
            .map(|(_, s)| *s)
            .unwrap_or(1.0);
        let effective_std = stat.stddev.max(global_std * 0.1);
        let z = (track_val - stat.mean) / effective_std;
        let contrib = stat.fisher_weight * z * z;
        total_contribution += contrib;

        contributions.push(FeatureContribution {
            name: fname,
            track_value: track_val,
            proto_mean: stat.mean,
            z_score: z,
            fisher_weight: stat.fisher_weight,
        });
    }

    // Tier 2: Timbral distances (if genre has centroids)
    if let (Some(track_mfcc), Some(centroid), Some(mean_dist)) =
        (mfcc, &proto.mfcc_centroid, proto.mfcc_mean_dist)
    {
        let dist = euclidean_distance(track_mfcc, centroid);
        let z = dist / mean_dist;
        // Timbral distance gets a fixed weight budget of ~0.15 total for all 3 dims
        total_contribution += 0.05 * z * z;
    }

    if let (Some(track_mfcc_s), Some(centroid), Some(mean_dist)) =
        (mfcc_s, &proto.mfcc_std_centroid, proto.mfcc_std_mean_dist)
    {
        let dist = euclidean_distance(track_mfcc_s, centroid);
        let z = dist / mean_dist;
        total_contribution += 0.05 * z * z;
    }

    if let (Some(track_c), Some(centroid), Some(mean_dist)) =
        (contrast, &proto.contrast_centroid, proto.contrast_mean_dist)
    {
        let dist = euclidean_distance(track_c, centroid);
        let z = dist / mean_dist;
        total_contribution += 0.05 * z * z;
    }

    let distance = total_contribution.sqrt();

    // Confidence penalty for small genres
    let n_active = proto.features.len() as f64;
    let penalty = 0.5 * (n_active / proto.total_n as f64).sqrt();
    let adjusted_distance = distance + penalty;

    let vote_weight = (AFFINITY_CAP as f64 * (1.0 - adjusted_distance / SCALE))
        .clamp(0.0, AFFINITY_CAP as f64) as f32;

    // Keep only top 3 contributions for evidence strings
    contributions.sort_by(|a, b| {
        (b.fisher_weight * b.z_score * b.z_score)
            .partial_cmp(&(a.fisher_weight * a.z_score * a.z_score))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    contributions.truncate(3);

    AudioAffinity {
        genre: proto.genre,
        distance: adjusted_distance,
        vote_weight,
        contributions,
    }
}

// ---------------------------------------------------------------------------
// Persistence (SQLite)
// ---------------------------------------------------------------------------

/// Save calibrated prototypes to SQLite.
pub(crate) fn save_to_db(
    conn: &Connection,
    registry: &ProfileRegistry,
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;

    // Clear existing data
    tx.execute("DELETE FROM genre_audio_profiles", [])?;
    tx.execute("DELETE FROM genre_timbral_centroids", [])?;
    tx.execute("DELETE FROM genre_global_stats", [])?;

    {
        let mut insert_feature = tx.prepare(
            "INSERT INTO genre_audio_profiles (genre, feature, mean, stddev, fisher_weight, n_verified)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;

        let mut insert_centroid = tx.prepare(
            "INSERT INTO genre_timbral_centroids (genre, centroid_type, values_json, mean_dist, n_verified)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;

        let mut insert_global = tx.prepare(
            "INSERT INTO genre_global_stats (feature, mean, stddev) VALUES (?1, ?2, ?3)",
        )?;

        for proto in registry.prototypes.values() {
            for (&fname, stat) in &proto.features {
                insert_feature.execute(rusqlite::params![
                    proto.genre,
                    fname,
                    stat.mean,
                    stat.stddev,
                    stat.fisher_weight,
                    stat.n,
                ])?;
            }

            // Vec<f64> serialization is infallible for finite values
            if let Some(ref centroid) = proto.mfcc_centroid {
                let json =
                    serde_json::to_string(centroid).expect("Vec<f64> serialization is infallible");
                insert_centroid.execute(rusqlite::params![
                    proto.genre,
                    "mfcc_mean",
                    json,
                    proto.mfcc_mean_dist,
                    proto.total_n,
                ])?;
            }
            if let Some(ref centroid) = proto.mfcc_std_centroid {
                let json =
                    serde_json::to_string(centroid).expect("Vec<f64> serialization is infallible");
                insert_centroid.execute(rusqlite::params![
                    proto.genre,
                    "mfcc_std",
                    json,
                    proto.mfcc_std_mean_dist,
                    proto.total_n,
                ])?;
            }
            if let Some(ref centroid) = proto.contrast_centroid {
                let json =
                    serde_json::to_string(centroid).expect("Vec<f64> serialization is infallible");
                insert_centroid.execute(rusqlite::params![
                    proto.genre,
                    "contrast",
                    json,
                    proto.contrast_mean_dist,
                    proto.total_n,
                ])?;
            }
        }

        // Persist global stats for faithful round-trip
        for (&fname, &(mean, stddev)) in &registry.global_stats {
            insert_global.execute(rusqlite::params![fname, mean, stddev])?;
        }
    }

    tx.commit()?;
    Ok(())
}

/// Load calibrated prototypes from SQLite.
pub(crate) fn load_from_db(conn: &Connection) -> Result<Option<ProfileRegistry>, rusqlite::Error> {
    // Check if table has data
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM genre_audio_profiles", [], |row| {
        row.get(0)
    })?;
    if count == 0 {
        return Ok(None);
    }

    let mut stmt = conn.prepare(
        "SELECT genre, feature, mean, stddev, fisher_weight, n_verified
         FROM genre_audio_profiles",
    )?;

    let mut proto_map: HashMap<String, HashMap<String, FeatureStat>> = HashMap::new();
    let mut n_map: HashMap<String, u32> = HashMap::new();

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, f64>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, u32>(5)?,
        ))
    })?;

    for row in rows {
        let (genre, feature, mean, stddev, fisher_weight, n) = row?;
        n_map.entry(genre.clone()).or_insert(n);
        proto_map.entry(genre).or_default().insert(
            feature,
            FeatureStat {
                mean,
                stddev,
                fisher_weight,
                n,
            },
        );
    }

    // Load timbral centroids
    type CentroidEntry = (Vec<f64>, Option<f64>, u32);
    let mut centroid_map: HashMap<String, HashMap<String, CentroidEntry>> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT genre, centroid_type, values_json, mean_dist, n_verified
             FROM genre_timbral_centroids",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?;
        for row in rows {
            let (genre, ctype, json, mean_dist, n) = row?;
            if let Ok(values) = serde_json::from_str::<Vec<f64>>(&json) {
                centroid_map
                    .entry(genre)
                    .or_default()
                    .insert(ctype, (values, mean_dist, n));
            }
        }
    }

    // Convert to static str references via canonical_genre_name
    let mut prototypes: HashMap<&'static str, GenrePrototype> = HashMap::new();

    for (genre_str, feat_map) in proto_map {
        let genre = match crate::genre::canonical_genre_name(&genre_str) {
            Some(g) => g,
            None => continue,
        };
        let n = n_map.get(&genre_str).copied().unwrap_or(0);
        let features: HashMap<&'static str, FeatureStat> = feat_map
            .into_iter()
            .filter_map(|(f, s)| {
                SCALAR_FEATURE_NAMES
                    .iter()
                    .find(|&&n| n == f)
                    .map(|&n| (n, s))
            })
            .collect();

        let centroids = centroid_map.get(&genre_str);
        let (mfcc_centroid, mfcc_mean_dist) = centroids
            .and_then(|c| c.get("mfcc_mean"))
            .map(|(v, d, _)| (Some(v.clone()), *d))
            .unwrap_or((None, None));
        let (mfcc_std_centroid, mfcc_std_mean_dist) = centroids
            .and_then(|c| c.get("mfcc_std"))
            .map(|(v, d, _)| (Some(v.clone()), *d))
            .unwrap_or((None, None));
        let (contrast_centroid, contrast_mean_dist) = centroids
            .and_then(|c| c.get("contrast"))
            .map(|(v, d, _)| (Some(v.clone()), *d))
            .unwrap_or((None, None));

        // Skip prototypes with no valid features (would score as perfect match for everything)
        if features.is_empty() {
            continue;
        }
        prototypes.insert(
            genre,
            GenrePrototype {
                genre,
                features,
                mfcc_centroid,
                mfcc_std_centroid,
                contrast_centroid,
                mfcc_mean_dist,
                mfcc_std_mean_dist,
                contrast_mean_dist,
                total_n: n,
            },
        );
    }

    // Load persisted global stats (faithful round-trip, not lossy approximation)
    let mut global_stats: HashMap<&'static str, (f64, f64)> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT feature, mean, stddev FROM genre_global_stats")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows {
            let (feature, mean, stddev) = row?;
            if let Some(&static_name) = SCALAR_FEATURE_NAMES.iter().find(|&&n| n == feature) {
                global_stats.insert(static_name, (mean, stddev));
            }
        }
    }

    Ok(Some(ProfileRegistry {
        prototypes,
        global_stats,
    }))
}

/// Format an evidence string for a scored affinity.
pub(crate) fn format_evidence(affinity: &AudioAffinity) -> String {
    let parts: Vec<String> = affinity
        .contributions
        .iter()
        .map(|c| {
            format!(
                "{}={:.0}~{:.0} [F={:.1}]",
                short_name(c.name),
                c.track_value,
                c.proto_mean,
                c.fisher_weight * 10.0
            )
        })
        .collect();
    format!(
        "audio-profile: {} {:.2} ({})",
        affinity.genre,
        affinity.vote_weight,
        parts.join(", ")
    )
}

fn short_name(name: &str) -> &str {
    match name {
        "rekordbox_bpm" => "bpm",
        "spectral_centroid_mean" => "centroid",
        "spectral_centroid_cv" => "centroid_cv",
        "dynamic_complexity" => "dc",
        "rhythm_regularity" => "rr",
        "loudness_integrated" => "lufs",
        "decay_mid_tau" => "decay",
        "decay_high_tau" => "decay_hi",
        "spectral_flux_mean" => "flux",
        "dissonance_mean" => "diss",
        "key_clarity" => "key_cl",
        "onset_rate" => "onset",
        other => other,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_audio(bpm: f64, dance: f64, dc: f64, centroid: f64) -> AudioFeatures {
        AudioFeatures {
            rekordbox_bpm: bpm,
            stratum_bpm: None,
            bpm_agreement: None,
            essentia_bpm: None,
            duration_seconds: Some(300.0),
            danceability: Some(dance),
            dynamic_complexity: Some(dc),
            rhythm_regularity: Some(0.9),
            spectral_centroid_mean: Some(centroid),
            decay_mid_tau: None,
            decay_high_tau: None,
            onset_rate: Some(4.0),
            loudness_integrated: Some(-8.0),
            loudness_range: None,
            spectral_centroid_cv: None,
            spectral_flux_mean: None,
            dissonance_mean: None,
            key_clarity: None,
            key_confidence: None,
            kick_pattern: None,
            kick_pattern_confidence: None,
            kick_kicks_per_bar: None,
            kick_onset_count: None,
            kick_rate_basis: None,
            kick_histogram: None,
            mfcc_mean: None,
            mfcc_std: None,
            spectral_contrast_mean: None,
        }
    }

    #[test]
    fn calibrate_produces_prototypes() {
        // Create 6 "House" tracks and 6 "Ambient" tracks with distinct features
        let house: Vec<AudioFeatures> = (0..6)
            .map(|i| test_audio(124.0 + i as f64, 2.0, 3.0, 1800.0))
            .collect();
        let ambient: Vec<AudioFeatures> = (0..6)
            .map(|i| test_audio(90.0 + i as f64, 0.8, 7.0, 400.0))
            .collect();

        let samples: Vec<(&str, &AudioFeatures)> = house
            .iter()
            .map(|a| ("House", a))
            .chain(ambient.iter().map(|a| ("Ambient", a)))
            .collect();

        let registry = calibrate(&samples);

        assert_eq!(registry.prototypes.len(), 2);
        assert!(registry.prototypes.contains_key("House"));
        assert!(registry.prototypes.contains_key("Ambient"));

        // Fisher weights should sum to ~1.0 per genre
        for proto in registry.prototypes.values() {
            let sum: f64 = proto.features.values().map(|s| s.fisher_weight).sum();
            assert!(
                (sum - 1.0).abs() < 0.01,
                "Fisher weights should sum to 1.0, got {sum}"
            );
        }
    }

    #[test]
    fn scoring_prefers_closer_genre() {
        let house: Vec<AudioFeatures> = (0..6)
            .map(|i| test_audio(124.0 + i as f64, 2.0, 3.0, 1800.0))
            .collect();
        let ambient: Vec<AudioFeatures> = (0..6)
            .map(|i| test_audio(90.0 + i as f64, 0.8, 7.0, 400.0))
            .collect();

        let samples: Vec<(&str, &AudioFeatures)> = house
            .iter()
            .map(|a| ("House", a))
            .chain(ambient.iter().map(|a| ("Ambient", a)))
            .collect();

        let registry = calibrate(&samples);

        // A house-like track should score higher for House
        let house_track = test_audio(125.0, 2.1, 2.8, 1750.0);
        let affinities = score_all(&house_track, &registry);
        assert!(!affinities.is_empty());
        assert_eq!(
            affinities[0].genre, "House",
            "House track should match House first"
        );

        // An ambient-like track should score higher for Ambient
        let ambient_track = test_audio(88.0, 0.7, 7.5, 350.0);
        let affinities = score_all(&ambient_track, &registry);
        assert!(!affinities.is_empty());
        assert_eq!(
            affinities[0].genre, "Ambient",
            "Ambient track should match Ambient first"
        );
    }

    #[test]
    fn below_min_tracks_produces_no_prototype() {
        let tracks: Vec<AudioFeatures> = (0..3)
            .map(|i| test_audio(130.0 + i as f64, 2.0, 3.0, 1500.0))
            .collect();
        let samples: Vec<(&str, &AudioFeatures)> = tracks.iter().map(|a| ("Garage", a)).collect();

        let registry = calibrate(&samples);
        assert!(
            !registry.prototypes.contains_key("Garage"),
            "3 tracks is below MIN_TRACKS=5"
        );
    }

    #[test]
    fn vote_weight_capped_at_affinity_cap() {
        let tracks: Vec<AudioFeatures> = (0..10)
            .map(|i| test_audio(130.0 + i as f64 * 0.1, 2.0, 3.0, 1500.0))
            .collect();
        let samples: Vec<(&str, &AudioFeatures)> = tracks.iter().map(|a| ("Techno", a)).collect();

        let registry = calibrate(&samples);

        // Score a track identical to the prototype mean
        let exact_match = test_audio(130.5, 2.0, 3.0, 1500.0);
        let affinities = score_all(&exact_match, &registry);

        for a in &affinities {
            assert!(
                a.vote_weight <= AFFINITY_CAP,
                "Vote weight {} exceeds cap {}",
                a.vote_weight,
                AFFINITY_CAP
            );
        }
    }

    #[test]
    fn scalar_feature_names_matches_extraction() {
        // Verify SCALAR_FEATURE_NAMES and extract_scalar_features stay in sync.
        // Use distinct values per field so a swap would be detectable.
        let audio = AudioFeatures {
            rekordbox_bpm: 1.0,
            stratum_bpm: None,
            bpm_agreement: None,
            essentia_bpm: None,
            duration_seconds: Some(300.0),
            danceability: Some(2.0),
            onset_rate: Some(3.0),
            rhythm_regularity: Some(4.0),
            spectral_centroid_mean: Some(5.0),
            spectral_centroid_cv: Some(6.0),
            dynamic_complexity: Some(7.0),
            loudness_integrated: Some(8.0),
            loudness_range: None,
            decay_mid_tau: Some(9.0),
            decay_high_tau: Some(10.0),
            spectral_flux_mean: Some(11.0),
            dissonance_mean: Some(12.0),
            key_clarity: Some(13.0),
            key_confidence: None,
            kick_pattern: None,
            kick_pattern_confidence: None,
            kick_kicks_per_bar: None,
            kick_onset_count: None,
            kick_rate_basis: None,
            kick_histogram: None,
            mfcc_mean: None,
            mfcc_std: None,
            spectral_contrast_mean: None,
        };
        let scalars = extract_scalar_features(&audio);
        assert_eq!(scalars.len(), SCALAR_FEATURE_NAMES.len());
        // Verify each named feature maps to the expected value
        for (i, &name) in SCALAR_FEATURE_NAMES.iter().enumerate() {
            let expected = (i + 1) as f64;
            assert_eq!(
                scalars[i],
                Some(expected),
                "Feature '{}' at index {} should be {}, got {:?}",
                name,
                i,
                expected,
                scalars[i]
            );
        }
    }

    #[test]
    fn nan_filtered_in_extraction() {
        let mut audio = test_audio(130.0, 2.0, 3.0, 1500.0);
        audio.danceability = Some(f64::NAN);
        audio.onset_rate = Some(f64::INFINITY);
        let scalars = extract_scalar_features(&audio);
        // NaN and infinity should become None
        assert!(
            scalars[1].is_none(),
            "NaN danceability should be filtered to None"
        );
        assert!(
            scalars[2].is_none(),
            "Infinity onset_rate should be filtered to None"
        );
        // Normal values should remain
        assert!(scalars[0].is_some(), "Finite BPM should remain");
    }
}
