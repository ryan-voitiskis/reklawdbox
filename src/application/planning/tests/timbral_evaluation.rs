use rusqlite::Connection;

use crate::application::planning::*;
use crate::domain::planning::*;

#[allow(clippy::too_many_arguments)]
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
        track: crate::domain::library::Track {
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
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
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
        timbral: None,
    }
}

fn simple_profile(id: &str, key: &str, bpm: f64, energy: f64, genre: &str) -> TrackProfile {
    synth_profile(id, key, bpm, energy, genre, None, None, None)
}

fn open_timbral_norm_test_store() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("store.sqlite3");
    let conn = crate::adapters::state::open(store_path.to_str().unwrap()).unwrap();
    (dir, conn)
}

fn write_timbral_test_file(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_string()
}

fn timbral_test_json(base: f64) -> String {
    serde_json::json!({
        "mfcc_mean": [base],
        "mfcc_std": [base + 1.0],
        "spectral_contrast_mean": [base + 2.0],
        "spectral_centroid_cv": base + 3.0,
        "dissonance_mean": base + 4.0,
    })
    .to_string()
}

fn cache_timbral_test_entry(conn: &Connection, file_path: &str, features_json: &str) {
    let identity = crate::application::analysis::identity::audio_cache_identity(file_path).unwrap();
    crate::adapters::state::set_audio_analysis(
        conn,
        &identity.cache_key,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        identity.file_size,
        identity.file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        features_json,
    )
    .unwrap();
}

fn cache_stale_timbral_test_entry(conn: &Connection, file_path: &str, features_json: &str) {
    let identity = crate::application::analysis::identity::audio_cache_identity(file_path).unwrap();
    crate::adapters::state::set_audio_analysis(
        conn,
        &identity.cache_key,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        identity.file_size + 1,
        identity.file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        features_json,
    )
    .unwrap();
}

#[test]
fn eval_describe_pool_optimal_bpm_sweep() {
    // 5 tracks at 124-132 BPM, all 8A key
    let profiles = vec![
        simple_profile("bpm1", "8A", 124.0, 0.5, "House"),
        simple_profile("bpm2", "8A", 126.0, 0.55, "House"),
        simple_profile("bpm3", "8A", 128.0, 0.6, "House"),
        simple_profile("bpm4", "8A", 130.0, 0.65, "House"),
        simple_profile("bpm5", "8A", 132.0, 0.7, "House"),
    ];

    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

    let result = crate::application::planning::sweep_optimal_reference_bpm(&profiles, &bpms);

    assert!(
        result.is_some(),
        "should find an optimal reference BPM for 124-132 range",
    );

    let (optimal_bpm, optimal_stability) = result.unwrap();
    assert!(
        (optimal_bpm - 128.0).abs() <= 2.0,
        "optimal BPM {optimal_bpm:.1} should be near median 128.0 (±2.0)",
    );
    assert!(
        optimal_stability > 0.5,
        "optimal stability {optimal_stability:.3} should be reasonable",
    );
}

#[test]
fn timbral_source_snapshot_fingerprint_is_deterministic_and_versioned() {
    let (dir, conn) = open_timbral_norm_test_store();
    let path_a = write_timbral_test_file(&dir, "a.flac", b"audio-a");
    let path_b = write_timbral_test_file(&dir, "b.flac", b"audio-b");
    let json_a = timbral_test_json(1.0);
    let json_b = timbral_test_json(3.0);

    cache_timbral_test_entry(&conn, &path_b, &json_b);
    cache_timbral_test_entry(&conn, &path_a, &json_a);
    let first = load_timbral_source_snapshot(&conn).unwrap();
    assert_eq!(first.vectors.len(), 2);
    assert_eq!(first.source_fingerprint.len(), 64);
    assert!(
        first
            .source_fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "fingerprint should be lowercase SHA-256 hex",
    );

    conn.execute("DELETE FROM audio_analysis_cache", [])
        .unwrap();
    cache_timbral_test_entry(&conn, &path_a, &json_a);
    cache_timbral_test_entry(&conn, &path_b, &json_b);
    let opposite_insertion_order = load_timbral_source_snapshot(&conn).unwrap();
    assert_eq!(
        opposite_insertion_order.source_fingerprint,
        first.source_fingerprint
    );

    let changed_json = timbral_test_json(5.0);
    cache_timbral_test_entry(&conn, &path_a, &changed_json);
    let after_json_change = load_timbral_source_snapshot(&conn).unwrap();
    assert_ne!(
        after_json_change.source_fingerprint,
        first.source_fingerprint
    );

    std::fs::write(&path_a, b"audio-a-with-a-new-identity").unwrap();
    cache_timbral_test_entry(&conn, &path_a, &json_a);
    let after_identity_change = load_timbral_source_snapshot(&conn).unwrap();
    assert_ne!(
        after_identity_change.source_fingerprint,
        first.source_fingerprint
    );

    let alternate_vector_schema =
        load_timbral_source_snapshot_for_test(&conn, "test-alternate-schema").unwrap();
    assert_ne!(
        alternate_vector_schema.source_fingerprint,
        after_identity_change.source_fingerprint
    );
}

#[test]
fn timbral_source_snapshot_excludes_unusable_sources_without_changing_digest() {
    let (dir, conn) = open_timbral_norm_test_store();
    let valid_a = write_timbral_test_file(&dir, "00-valid-a.flac", b"valid-a");
    let valid_b = write_timbral_test_file(&dir, "01-valid-b.flac", b"valid-b");
    cache_timbral_test_entry(&conn, &valid_a, &timbral_test_json(1.0));
    cache_timbral_test_entry(&conn, &valid_b, &timbral_test_json(3.0));
    let baseline = load_timbral_source_snapshot(&conn).unwrap();
    assert_eq!(baseline.vectors.len(), 2);

    let stale = write_timbral_test_file(&dir, "10-stale.flac", b"stale");
    cache_stale_timbral_test_entry(&conn, &stale, &timbral_test_json(100.0));

    let invalid = write_timbral_test_file(&dir, "11-invalid.flac", b"invalid");
    cache_timbral_test_entry(&conn, &invalid, "{");

    let incomplete = write_timbral_test_file(&dir, "12-incomplete.flac", b"incomplete");
    cache_timbral_test_entry(
        &conn,
        &incomplete,
        r#"{"mfcc_mean":[1.0],"mfcc_std":[2.0],"spectral_contrast_mean":[3.0],"spectral_centroid_cv":4.0}"#,
    );

    let nonfinite = write_timbral_test_file(&dir, "13-nonfinite.flac", b"nonfinite");
    cache_timbral_test_entry(
        &conn,
        &nonfinite,
        r#"{"mfcc_mean":[1e400],"mfcc_std":[2.0],"spectral_contrast_mean":[3.0],"spectral_centroid_cv":4.0,"dissonance_mean":5.0}"#,
    );

    let mismatched = write_timbral_test_file(&dir, "14-mismatched.flac", b"mismatched");
    cache_timbral_test_entry(
        &conn,
        &mismatched,
        r#"{"mfcc_mean":[1.0,2.0],"mfcc_std":[3.0],"spectral_contrast_mean":[4.0],"spectral_centroid_cv":5.0,"dissonance_mean":6.0}"#,
    );

    let missing_path = dir.path().join("15-missing.flac");
    crate::adapters::state::set_audio_analysis(
        &conn,
        missing_path.to_str().unwrap(),
        crate::adapters::audio::ANALYZER_ESSENTIA,
        0,
        0,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &timbral_test_json(200.0),
    )
    .unwrap();

    let old_schema = write_timbral_test_file(&dir, "16-old-schema.flac", b"old-schema");
    let old_identity =
        crate::application::analysis::identity::audio_cache_identity(&old_schema).unwrap();
    crate::adapters::state::set_audio_analysis(
        &conn,
        &old_identity.cache_key,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        old_identity.file_size,
        old_identity.file_mtime,
        "old-schema",
        &timbral_test_json(300.0),
    )
    .unwrap();

    let filtered = load_timbral_source_snapshot(&conn).unwrap();
    assert_eq!(filtered.vectors.len(), 2);
    assert_eq!(filtered.source_fingerprint, baseline.source_fingerprint);
}

#[test]
fn timbral_norm_compute_uses_one_exact_fresh_snapshot() {
    let (dir, conn) = open_timbral_norm_test_store();
    let path_a = write_timbral_test_file(&dir, "a.flac", b"audio-a");
    let path_b = write_timbral_test_file(&dir, "b.flac", b"audio-b");
    let stale = write_timbral_test_file(&dir, "stale.flac", b"stale");
    cache_timbral_test_entry(&conn, &path_a, &timbral_test_json(1.0));
    cache_timbral_test_entry(&conn, &path_b, &timbral_test_json(3.0));
    cache_stale_timbral_test_entry(&conn, &stale, &timbral_test_json(100.0));

    let snapshot = load_timbral_source_snapshot(&conn).unwrap();
    let stats = compute_timbral_norm_stats(&conn).unwrap();
    assert_eq!(snapshot.vectors.len(), 2);
    assert_eq!(stats.sample_count, 2);
    assert_eq!(stats.source_fingerprint, snapshot.source_fingerprint);
    assert_eq!(
        stats.analysis_version,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION
    );
    assert_eq!(stats.vector_schema_version, TIMBRAL_VECTOR_SCHEMA_VERSION);
    assert_eq!(stats.dims.len(), 5);
    for (index, (mean, stddev)) in stats.dims.iter().enumerate() {
        let expected_mean = index as f64 + 2.0;
        assert!((mean - expected_mean).abs() < 1e-12);
        assert!((stddev - 2.0_f64.sqrt()).abs() < 1e-12);
    }

    let ensured = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(ensured, stats);
    assert_eq!(
        crate::adapters::state::get_timbral_norm_stats(&conn)
            .unwrap()
            .unwrap(),
        stats
    );
}

#[test]
fn timbral_norm_ensure_reuses_exact_sources_and_recomputes_same_count_changes() {
    let (dir, conn) = open_timbral_norm_test_store();
    let path_a = write_timbral_test_file(&dir, "a.flac", b"audio-a");
    let path_b = write_timbral_test_file(&dir, "b.flac", b"audio-b");
    cache_timbral_test_entry(&conn, &path_a, &timbral_test_json(1.0));
    cache_timbral_test_entry(&conn, &path_b, &timbral_test_json(3.0));

    let first = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    conn.execute(
        "UPDATE timbral_norm_stats SET computed_at = 'reuse-sentinel'",
        [],
    )
    .unwrap();
    let reused = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(reused, first);
    let sentinel_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM timbral_norm_stats
             WHERE computed_at = 'reuse-sentinel'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(sentinel_rows, first.dims.len() as i64);

    cache_timbral_test_entry(&conn, &path_a, &timbral_test_json(5.0));
    let after_json_replace = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(after_json_replace.sample_count, 2);
    assert_ne!(
        after_json_replace.source_fingerprint,
        first.source_fingerprint
    );
    assert!((after_json_replace.dims[0].0 - 4.0).abs() < 1e-12);

    conn.execute(
        "DELETE FROM audio_analysis_cache WHERE file_path = ?1",
        rusqlite::params![path_b],
    )
    .unwrap();
    let path_c = write_timbral_test_file(&dir, "c.flac", b"audio-c");
    cache_timbral_test_entry(&conn, &path_c, &timbral_test_json(7.0));
    let after_remove_add = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(after_remove_add.sample_count, 2);
    assert_ne!(
        after_remove_add.source_fingerprint,
        after_json_replace.source_fingerprint
    );

    std::fs::write(&path_a, b"audio-a-now-stale-because-size-changed").unwrap();
    assert!(ensure_timbral_norm_stats(&conn).unwrap().is_none());
    let persisted_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM timbral_norm_stats", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(persisted_rows, 0);
}

#[test]
fn timbral_norm_ensure_rebuilds_mismatched_or_incoherent_stats() {
    let (dir, conn) = open_timbral_norm_test_store();
    let path_a = write_timbral_test_file(&dir, "a.flac", b"audio-a");
    let path_b = write_timbral_test_file(&dir, "b.flac", b"audio-b");
    cache_timbral_test_entry(&conn, &path_a, &timbral_test_json(1.0));
    cache_timbral_test_entry(&conn, &path_b, &timbral_test_json(3.0));
    let baseline = ensure_timbral_norm_stats(&conn).unwrap().unwrap();

    conn.execute(
        "UPDATE timbral_norm_stats SET mean = 999.0, analysis_version = 'old'",
        [],
    )
    .unwrap();
    let analysis_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(analysis_rebuilt, baseline);

    conn.execute(
        "UPDATE timbral_norm_stats
         SET mean = 999.0, vector_schema_version = 'old'",
        [],
    )
    .unwrap();
    let vector_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(vector_rebuilt, baseline);

    conn.execute(
        "UPDATE timbral_norm_stats
         SET mean = 999.0, source_fingerprint = 'old'",
        [],
    )
    .unwrap();
    let source_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(source_rebuilt, baseline);

    let mut wrong_dimensions = baseline.clone();
    wrong_dimensions.dims.pop();
    crate::adapters::state::save_timbral_norm_stats(&conn, &wrong_dimensions).unwrap();
    let dimensions_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(dimensions_rebuilt, baseline);

    let mut wrong_count = baseline.clone();
    wrong_count.sample_count = 999;
    crate::adapters::state::save_timbral_norm_stats(&conn, &wrong_count).unwrap();
    let count_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(count_rebuilt, baseline);

    conn.execute(
        "UPDATE timbral_norm_stats
         SET mean = 999.0, sample_count = sample_count + 1
         WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(
        crate::adapters::state::get_timbral_norm_stats(&conn)
            .unwrap()
            .is_none()
    );
    let incoherent_rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(incoherent_rebuilt, baseline);
}

#[test]
fn timbral_norm_legacy_rows_invalidate_and_recompute_once() {
    let dir = tempfile::tempdir().unwrap();
    let store_path = dir.path().join("legacy-store.sqlite3");
    let raw = Connection::open(&store_path).unwrap();
    raw.execute_batch(
        "CREATE TABLE timbral_norm_stats (
             dimension_index INTEGER PRIMARY KEY,
             mean REAL NOT NULL,
             stddev REAL NOT NULL,
             sample_count INTEGER NOT NULL,
             computed_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO timbral_norm_stats
             (dimension_index, mean, stddev, sample_count)
         VALUES (0, 999.0, 1.0, 2);
         PRAGMA user_version = 7;",
    )
    .unwrap();
    drop(raw);

    let conn = crate::adapters::state::open(store_path.to_str().unwrap()).unwrap();
    let legacy = crate::adapters::state::get_timbral_norm_stats(&conn)
        .unwrap()
        .unwrap();
    assert!(legacy.source_fingerprint.is_empty());
    assert!(legacy.analysis_version.is_empty());
    assert!(legacy.vector_schema_version.is_empty());

    let path_a = write_timbral_test_file(&dir, "a.flac", b"audio-a");
    let path_b = write_timbral_test_file(&dir, "b.flac", b"audio-b");
    cache_timbral_test_entry(&conn, &path_a, &timbral_test_json(1.0));
    cache_timbral_test_entry(&conn, &path_b, &timbral_test_json(3.0));

    let rebuilt = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(rebuilt.sample_count, 2);
    assert!(!rebuilt.source_fingerprint.is_empty());
    assert_eq!(
        rebuilt.analysis_version,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION
    );
    assert_eq!(rebuilt.vector_schema_version, TIMBRAL_VECTOR_SCHEMA_VERSION);
    assert!((rebuilt.dims[0].0 - 2.0).abs() < 1e-12);

    let reused = ensure_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(reused, rebuilt);
}

#[test]
fn eval_bpm_sweep_too_wide_returns_none() {
    let profiles = vec![
        simple_profile("wide1", "8A", 100.0, 0.5, "House"),
        simple_profile("wide2", "8A", 140.0, 0.5, "House"),
    ];
    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

    let result = crate::application::planning::sweep_optimal_reference_bpm(&profiles, &bpms);
    assert!(
        result.is_none(),
        "100-140 BPM spread should be too wide for any single reference",
    );
}

#[test]
fn eval_bpm_sweep_tight_range_succeeds() {
    let profiles = vec![
        simple_profile("tight1", "8A", 126.0, 0.5, "House"),
        simple_profile("tight2", "8A", 127.0, 0.5, "House"),
        simple_profile("tight3", "8A", 128.0, 0.5, "House"),
    ];
    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

    let result = crate::application::planning::sweep_optimal_reference_bpm(&profiles, &bpms);
    assert!(result.is_some(), "126-128 range should find optimal BPM");

    let (optimal, stability) = result.unwrap();
    assert!(
        (optimal - 127.0).abs() <= 1.0,
        "optimal should be near center 127.0, got {optimal:.1}",
    );
    assert!(stability > 0.8, "tight range should have high stability");
}

#[test]
fn eval_bpm_sweep_narrow_interval_found() {
    let profiles = vec![
        simple_profile("narrow1", "8A", 100.0, 0.5, "House"),
        simple_profile("narrow2", "8A", 112.0, 0.5, "House"),
    ];
    let bpms: Vec<f64> = profiles.iter().map(|p| p.bpm).collect();

    let result = crate::application::planning::sweep_optimal_reference_bpm(&profiles, &bpms);
    assert!(
        result.is_some(),
        "100-112 BPM should find a valid reference (narrow interval ~105.7-105.9)",
    );

    let (optimal, _) = result.unwrap();
    assert!(
        (105.5..=106.1).contains(&optimal),
        "optimal should be near 105.8, got {optimal:.2}",
    );
}
