//! Classification profile persistence in writable local state.

use std::collections::HashMap;

use rusqlite::Connection;

use crate::domain::classification::profiles::{
    FeatureStat, GenrePrototype, ProfileRegistry, SCALAR_FEATURE_NAMES,
};
use crate::domain::classification::taxonomy;

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
        let genre = match taxonomy::canonical_genre_name(&genre_str) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::state::migrations::migrate;

    fn test_registry() -> ProfileRegistry {
        let mut features = HashMap::new();
        features.insert(
            "rekordbox_bpm",
            FeatureStat {
                mean: 132.0,
                stddev: 3.5,
                fisher_weight: 0.4,
                n: 8,
            },
        );

        let prototype = GenrePrototype {
            genre: "Techno",
            features,
            mfcc_centroid: Some(vec![1.0, 2.0, 3.0]),
            mfcc_std_centroid: Some(vec![0.1, 0.2]),
            contrast_centroid: Some(vec![4.0, 5.0, 6.0]),
            mfcc_mean_dist: Some(0.7),
            mfcc_std_mean_dist: Some(0.3),
            contrast_mean_dist: Some(0.5),
            total_n: 8,
        };

        ProfileRegistry {
            prototypes: HashMap::from([("Techno", prototype)]),
            global_stats: HashMap::from([("rekordbox_bpm", (128.0, 8.0))]),
        }
    }

    #[test]
    fn empty_store_has_no_profile_registry() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        assert!(load_from_db(&conn).unwrap().is_none());
    }

    #[test]
    fn profile_registry_round_trips_through_state_store() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let registry = test_registry();

        save_to_db(&conn, &registry).unwrap();
        let loaded = load_from_db(&conn).unwrap().unwrap();

        assert_eq!(loaded.global_stats, registry.global_stats);
        let prototype = loaded.prototypes.get("Techno").unwrap();
        let feature = prototype.features.get("rekordbox_bpm").unwrap();
        assert_eq!(feature.mean, 132.0);
        assert_eq!(feature.stddev, 3.5);
        assert_eq!(feature.fisher_weight, 0.4);
        assert_eq!(feature.n, 8);
        assert_eq!(prototype.mfcc_centroid, Some(vec![1.0, 2.0, 3.0]));
        assert_eq!(prototype.mfcc_std_centroid, Some(vec![0.1, 0.2]));
        assert_eq!(prototype.contrast_centroid, Some(vec![4.0, 5.0, 6.0]));
        assert_eq!(prototype.mfcc_mean_dist, Some(0.7));
        assert_eq!(prototype.mfcc_std_mean_dist, Some(0.3));
        assert_eq!(prototype.contrast_mean_dist, Some(0.5));
        assert_eq!(prototype.total_n, 8);
    }
}
