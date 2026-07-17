//! Classification profile persistence in writable local state.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};

use crate::adapters::audio;
use crate::domain::classification::profiles::{
    FeatureStat, GenrePrototype, PROFILE_SCHEMA_VERSION, ProfileLoad, ProfileLoadStatus,
    ProfileMetadata, ProfileRegistry, SCALAR_FEATURE_NAMES,
};
use crate::domain::classification::taxonomy;

// ---------------------------------------------------------------------------
// Persistence (SQLite)
// ---------------------------------------------------------------------------

/// Save calibrated prototypes to SQLite.
pub(crate) fn save_to_db(
    conn: &Connection,
    registry: &ProfileRegistry,
    metadata: &ProfileMetadata,
) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;

    // Clear existing data
    tx.execute("DELETE FROM genre_audio_profiles", [])?;
    tx.execute("DELETE FROM genre_timbral_centroids", [])?;
    tx.execute("DELETE FROM genre_global_stats", [])?;
    tx.execute("DELETE FROM genre_profile_metadata", [])?;

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

    tx.execute(
        "INSERT INTO genre_profile_metadata (
             id,
             classifier_profile_schema_version,
             stratum_schema_version,
             essentia_schema_version,
             playlist_name,
             training_fingerprint,
             scorable_sample_count,
             calibrated_at
         ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            metadata.classifier_profile_schema_version,
            metadata.stratum_schema_version,
            metadata.essentia_schema_version,
            metadata.playlist_name,
            metadata.training_fingerprint,
            metadata.scorable_sample_count,
            metadata.calibrated_at,
        ],
    )?;

    tx.commit()?;
    Ok(())
}

/// Load calibrated prototypes only when persisted metadata is structurally
/// compatible. A changed training corpus remains usable but is reported.
pub(crate) fn load_from_db(
    conn: &Connection,
    expected_training_fingerprint: Option<&str>,
) -> Result<ProfileLoad, rusqlite::Error> {
    let Some(registry) = load_registry_rows(conn)? else {
        return Ok(ProfileLoad {
            status: ProfileLoadStatus::Missing,
            registry: None,
            metadata: None,
            reason: None,
        });
    };

    let metadata_table_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'genre_profile_metadata')",
        [],
        |row| row.get(0),
    )?;
    let metadata = if metadata_table_exists {
        conn.query_row(
            "SELECT classifier_profile_schema_version,
                    stratum_schema_version,
                    essentia_schema_version,
                    playlist_name,
                    training_fingerprint,
                    scorable_sample_count,
                    calibrated_at
             FROM genre_profile_metadata WHERE id = 1",
            [],
            |row| {
                Ok(ProfileMetadata {
                    classifier_profile_schema_version: row.get(0)?,
                    stratum_schema_version: row.get(1)?,
                    essentia_schema_version: row.get(2)?,
                    playlist_name: row.get(3)?,
                    training_fingerprint: row.get(4)?,
                    scorable_sample_count: row.get(5)?,
                    calibrated_at: row.get(6)?,
                })
            },
        )
        .optional()?
    } else {
        None
    };

    let Some(metadata) = metadata else {
        return Ok(ProfileLoad {
            status: ProfileLoadStatus::Incompatible,
            registry: None,
            metadata: None,
            reason: Some("profile metadata is missing".into()),
        });
    };

    let incompatible_reason =
        if metadata.classifier_profile_schema_version != PROFILE_SCHEMA_VERSION {
            Some(format!(
                "classifier profile schema {} != {}",
                metadata.classifier_profile_schema_version, PROFILE_SCHEMA_VERSION
            ))
        } else if metadata.stratum_schema_version != audio::STRATUM_SCHEMA_VERSION {
            Some(format!(
                "Stratum schema {} != {}",
                metadata.stratum_schema_version,
                audio::STRATUM_SCHEMA_VERSION
            ))
        } else if metadata.essentia_schema_version != audio::ESSENTIA_SCHEMA_VERSION {
            Some(format!(
                "Essentia schema {} != {}",
                metadata.essentia_schema_version,
                audio::ESSENTIA_SCHEMA_VERSION
            ))
        } else {
            None
        };
    if let Some(reason) = incompatible_reason {
        return Ok(ProfileLoad {
            status: ProfileLoadStatus::Incompatible,
            registry: None,
            metadata: Some(metadata),
            reason: Some(reason),
        });
    }

    let training_changed = expected_training_fingerprint
        .is_some_and(|expected| expected != metadata.training_fingerprint);
    Ok(ProfileLoad {
        status: if training_changed {
            ProfileLoadStatus::TrainingChanged
        } else {
            ProfileLoadStatus::Fresh
        },
        registry: Some(registry),
        metadata: Some(metadata),
        reason: training_changed.then(|| "verified training corpus changed".into()),
    })
}

fn load_registry_rows(conn: &Connection) -> Result<Option<ProfileRegistry>, rusqlite::Error> {
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

    fn test_metadata(fingerprint: &str) -> ProfileMetadata {
        ProfileMetadata {
            classifier_profile_schema_version: PROFILE_SCHEMA_VERSION.into(),
            stratum_schema_version: audio::STRATUM_SCHEMA_VERSION.into(),
            essentia_schema_version: audio::ESSENTIA_SCHEMA_VERSION.into(),
            playlist_name: "genre_verified".into(),
            training_fingerprint: fingerprint.into(),
            scorable_sample_count: 8,
            calibrated_at: "2026-07-14T00:00:00Z".into(),
        }
    }

    #[test]
    fn empty_store_has_no_profile_registry() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();

        let loaded = load_from_db(&conn, None).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Missing);
        assert!(loaded.registry.is_none());
    }

    #[test]
    fn profile_registry_round_trips_through_state_store() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let registry = test_registry();
        let metadata = test_metadata("fingerprint-a");

        save_to_db(&conn, &registry, &metadata).unwrap();
        let loaded = load_from_db(&conn, Some("fingerprint-a")).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Fresh);
        assert_eq!(loaded.metadata.as_ref(), Some(&metadata));
        let loaded = loaded.registry.unwrap();

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

    #[test]
    fn compatible_training_change_remains_usable_and_reported() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        save_to_db(&conn, &test_registry(), &test_metadata("old")).unwrap();

        let loaded = load_from_db(&conn, Some("new")).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::TrainingChanged);
        assert!(loaded.registry.is_some());
        assert!(loaded.reason.unwrap().contains("corpus changed"));
    }

    #[test]
    fn missing_or_mismatched_metadata_suppresses_registry_without_deleting_rows() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        save_to_db(&conn, &test_registry(), &test_metadata("same")).unwrap();

        conn.execute("DELETE FROM genre_profile_metadata", [])
            .unwrap();
        let legacy = load_from_db(&conn, None).unwrap();
        assert_eq!(legacy.status, ProfileLoadStatus::Incompatible);
        assert!(legacy.registry.is_none());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_audio_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(rows > 0);

        let mut incompatible = test_metadata("same");
        incompatible.classifier_profile_schema_version = "future".into();
        save_to_db(&conn, &test_registry(), &incompatible).unwrap();
        let loaded = load_from_db(&conn, None).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Incompatible);
        assert!(loaded.registry.is_none());
    }

    #[test]
    fn essentia_v2_profile_metadata_is_incompatible_and_preserved() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut metadata = test_metadata("essentia-v2");
        metadata.essentia_schema_version = "2".into();
        save_to_db(&conn, &test_registry(), &metadata).unwrap();

        let loaded = load_from_db(&conn, None).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Incompatible);
        assert!(loaded.registry.is_none());
        assert!(loaded.reason.unwrap().contains("Essentia schema 2 != 3"));
        let profile_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_audio_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        let metadata_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_profile_metadata", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(profile_rows > 0);
        assert_eq!(metadata_rows, 1);
    }

    #[test]
    fn classification_calibration_v1_profile_is_incompatible_and_preserved() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let mut metadata = test_metadata("profile-v1");
        metadata.classifier_profile_schema_version = "1".into();
        save_to_db(&conn, &test_registry(), &metadata).unwrap();

        let loaded = load_from_db(&conn, None).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Incompatible);
        assert!(loaded.registry.is_none());
        assert!(
            loaded
                .reason
                .unwrap()
                .contains("classifier profile schema 1 != 2")
        );
        let profile_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_audio_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        let metadata_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_profile_metadata", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(profile_rows > 0);
        assert_eq!(metadata_rows, 1);
    }

    #[test]
    fn v9_profile_rows_migrate_as_incompatible_and_are_preserved() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        save_to_db(&conn, &test_registry(), &test_metadata("legacy")).unwrap();
        conn.execute_batch(
            "DROP TABLE genre_profile_metadata;
             PRAGMA user_version = 9;",
        )
        .unwrap();

        migrate(&conn).unwrap();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            version,
            crate::adapters::state::migrations::STORE_SCHEMA_VERSION
        );
        let loaded = load_from_db(&conn, None).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Incompatible);
        assert!(loaded.registry.is_none());
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM genre_audio_profiles", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert!(rows > 0);
    }

    #[test]
    fn failed_metadata_insert_rolls_back_registry_replacement() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        save_to_db(&conn, &test_registry(), &test_metadata("original")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_profile_metadata_insert
             BEFORE INSERT ON genre_profile_metadata
             BEGIN SELECT RAISE(ABORT, 'metadata write failed'); END;",
        )
        .unwrap();

        let empty = ProfileRegistry {
            prototypes: HashMap::new(),
            global_stats: HashMap::new(),
        };
        let result = save_to_db(&conn, &empty, &test_metadata("new"));
        assert!(result.is_err());
        let loaded = load_from_db(&conn, Some("original")).unwrap();
        assert_eq!(loaded.status, ProfileLoadStatus::Fresh);
        assert!(loaded.registry.unwrap().prototypes.contains_key("Techno"));
    }
}
