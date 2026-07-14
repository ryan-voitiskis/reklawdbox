use super::*;
use std::path::PathBuf;

use rusqlite::{Connection, params};

use crate::domain::audit::Resolution;

#[test]
fn configured_store_path_is_used_verbatim() {
    let configured = PathBuf::from("/tmp/reklawdbox-isolated.sqlite3");
    assert_eq!(
        resolve_path_from(Some(configured.clone().into_os_string())),
        configured
    );
}

#[test]
#[ignore = "informational benchmark; run through scripts/benchmark-rust-hotspots.sh"]
fn benchmark_batch_audio_cache_reads() {
    const TRACKS: usize = 500;
    const ROUNDS: usize = 20;
    let (_dir, conn) = open_temp_store();
    let paths: Vec<String> = (0..TRACKS)
        .map(|index| format!("/generated/track-{index:04}.wav"))
        .collect();
    for path in &paths {
        set_audio_analysis_with_fingerprint(
            &conn,
            path,
            "stratum-dsp",
            1_024,
            1_700_000_000,
            "s1",
            "hmm:v1",
            "{}",
        )
        .unwrap();
        set_audio_analysis_with_fingerprint(
            &conn,
            path,
            "essentia",
            1_024,
            1_700_000_000,
            "e1",
            "",
            "{}",
        )
        .unwrap();
    }
    let identities: Vec<AudioAnalysisIdentity<'_>> = paths
        .iter()
        .map(|path| AudioAnalysisIdentity {
            file_path: path,
            file_size: 1_024,
            file_mtime: 1_700_000_000,
            input_fingerprint: "hmm:v1",
        })
        .collect();

    let point_start = std::time::Instant::now();
    for _ in 0..ROUNDS {
        for path in &paths {
            std::hint::black_box(get_audio_analysis(&conn, path, "stratum-dsp").unwrap());
            std::hint::black_box(get_audio_analysis(&conn, path, "essentia").unwrap());
        }
    }
    let point_elapsed = point_start.elapsed();

    let batch_start = std::time::Instant::now();
    for _ in 0..ROUNDS {
        let stratum =
            batch_get_fresh_audio_analysis(&conn, &identities, "stratum-dsp", "s1").unwrap();
        let essentia = batch_get_audio_analysis(
            &conn,
            &paths.iter().map(String::as_str).collect::<Vec<_>>(),
            "essentia",
            "e1",
        )
        .unwrap();
        assert_eq!(stratum.len(), TRACKS);
        assert_eq!(essentia.len(), TRACKS);
        std::hint::black_box((stratum, essentia));
    }
    let batch_elapsed = batch_start.elapsed();

    eprintln!(
        "BENCHMARK audio_cache_reads tracks={TRACKS} rounds={ROUNDS} point_ms={:.3} batch_ms={:.3} speedup={:.2}x",
        point_elapsed.as_secs_f64() * 1_000.0,
        batch_elapsed.as_secs_f64() * 1_000.0,
        point_elapsed.as_secs_f64() / batch_elapsed.as_secs_f64(),
    );
}

fn open_temp_store() -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    let conn = open(path.to_str().unwrap()).unwrap();
    (dir, conn)
}

#[test]
fn audio_analysis_input_fingerprint_migrates_legacy_rows_by_analyzer() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE audio_analysis_cache (
            file_path TEXT NOT NULL,
            analyzer TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            file_mtime INTEGER NOT NULL,
            analysis_version TEXT NOT NULL,
            features_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (file_path, analyzer)
        );
        INSERT INTO audio_analysis_cache
            (file_path, analyzer, file_size, file_mtime, analysis_version, features_json)
        VALUES
            ('/legacy/stratum.flac', 'stratum-dsp', 10, 20, 's1', '{}'),
            ('/legacy/essentia.flac', 'essentia', 30, 40, 'e1', '{}');",
    )
    .unwrap();

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    assert!(table_has_column(&conn, "audio_analysis_cache", "input_fingerprint").unwrap());
    let stratum = get_audio_analysis(&conn, "/legacy/stratum.flac", "stratum-dsp")
        .unwrap()
        .unwrap();
    let essentia = get_audio_analysis(&conn, "/legacy/essentia.flac", "essentia")
        .unwrap()
        .unwrap();
    assert_eq!(stratum.input_fingerprint, "");
    assert_eq!(essentia.input_fingerprint, "");
    assert!(!is_audio_analysis_fresh(
        Some(&stratum),
        "s1",
        10,
        20,
        "hmm:v1"
    ));
    assert!(is_audio_analysis_fresh(Some(&essentia), "e1", 30, 40, ""));

    let stratum_identity = [AudioAnalysisIdentity {
        file_path: "/legacy/stratum.flac",
        file_size: 10,
        file_mtime: 20,
        input_fingerprint: "hmm:v1",
    }];
    let essentia_identity = [AudioAnalysisIdentity {
        file_path: "/legacy/essentia.flac",
        file_size: 30,
        file_mtime: 40,
        input_fingerprint: "",
    }];
    assert!(
        batch_get_fresh_audio_analysis(&conn, &stratum_identity, "stratum-dsp", "s1")
            .unwrap()
            .is_empty()
    );
    assert!(
        batch_fresh_audio_analysis_existence(&conn, &stratum_identity, "stratum-dsp", "s1")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        batch_get_fresh_audio_analysis(&conn, &essentia_identity, "essentia", "e1")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        batch_fresh_audio_analysis_existence(&conn, &essentia_identity, "essentia", "e1")
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn audio_analysis_input_fingerprint_controls_single_freshness() {
    let (_dir, conn) = open_temp_store();
    set_audio_analysis_with_fingerprint(
        &conn,
        "/music/track.flac",
        "stratum-dsp",
        10,
        20,
        "s1",
        "grid:v1:first",
        "{}",
    )
    .unwrap();
    let cached = get_audio_analysis(&conn, "/music/track.flac", "stratum-dsp")
        .unwrap()
        .unwrap();

    assert!(is_audio_analysis_fresh(
        Some(&cached),
        "s1",
        10,
        20,
        "grid:v1:first"
    ));
    assert!(!is_audio_analysis_fresh(
        Some(&cached),
        "s1",
        10,
        20,
        "grid:v1:changed"
    ));
    assert!(
        set_audio_analysis(
            &conn,
            "/music/legacy-stratum.flac",
            crate::adapters::audio::ANALYZER_STRATUM,
            10,
            20,
            "s1",
            "{}",
        )
        .is_err(),
        "Stratum writers must supply a versioned non-empty fingerprint"
    );
}

#[test]
fn audio_analysis_input_fingerprint_controls_batch_and_conflicts_fail_closed() {
    let (_dir, conn) = open_temp_store();
    for (path, fingerprint) in [
        ("/music/grid.flac", "grid:v1:grid"),
        ("/music/hmm.flac", "hmm:v1"),
    ] {
        set_audio_analysis_with_fingerprint(
            &conn,
            path,
            "stratum-dsp",
            10,
            20,
            "s1",
            fingerprint,
            "{}",
        )
        .unwrap();
    }
    let identities = [
        AudioAnalysisIdentity {
            file_path: "/music/grid.flac",
            file_size: 10,
            file_mtime: 20,
            input_fingerprint: "grid:v1:grid",
        },
        AudioAnalysisIdentity {
            file_path: "/music/hmm.flac",
            file_size: 10,
            file_mtime: 20,
            input_fingerprint: "hmm:v1",
        },
    ];
    assert_eq!(
        batch_get_fresh_audio_analysis(&conn, &identities, "stratum-dsp", "s1")
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        batch_fresh_audio_analysis_existence(&conn, &identities, "stratum-dsp", "s1")
            .unwrap()
            .len(),
        2
    );

    let conflicting = [
        identities[0],
        AudioAnalysisIdentity {
            input_fingerprint: "grid:v1:conflict",
            ..identities[0]
        },
    ];
    assert!(
        batch_get_fresh_audio_analysis(&conn, &conflicting, "stratum-dsp", "s1")
            .unwrap()
            .is_empty()
    );
    assert!(
        batch_fresh_audio_analysis_existence(&conn, &conflicting, "stratum-dsp", "s1")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_open_creates_schema() {
    let (_dir, conn) = open_temp_store();
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, STORE_SCHEMA_VERSION);

    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tables.contains(&"enrichment_cache".to_string()));
    assert!(tables.contains(&"audio_analysis_cache".to_string()));
    assert!(tables.contains(&"broker_discogs_session".to_string()));
    assert!(tables.contains(&"audit_files".to_string()));
    assert!(tables.contains(&"audit_issues".to_string()));
}

#[test]
fn test_open_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    let path_str = path.to_str().unwrap();

    let conn1 = open(path_str).unwrap();
    drop(conn1);
    let conn2 = open(path_str).unwrap();
    let version: i32 = conn2
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .unwrap();
    assert_eq!(version, STORE_SCHEMA_VERSION);
}

#[test]
fn test_open_accepts_bare_relative_filename_path() {
    use std::sync::{Mutex, OnceLock};

    struct CwdGuard(std::path::PathBuf);
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.0);
        }
    }

    // set_current_dir is process-global, so serialize this test section.
    static CWD_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = CWD_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("cwd lock poisoned");

    let original_cwd = std::env::current_dir().unwrap();
    let _restore_cwd = CwdGuard(original_cwd);
    let dir = tempfile::tempdir().unwrap();
    std::env::set_current_dir(dir.path()).unwrap();

    let db_name = "internal.sqlite3";
    let conn = open(db_name).unwrap();
    drop(conn);

    assert!(dir.path().join(db_name).is_file());
}

#[test]
fn test_open_reports_parent_directory_creation_failure() {
    let dir = tempfile::tempdir().unwrap();
    let not_a_directory = dir.path().join("not-a-directory");
    std::fs::write(&not_a_directory, b"blocker").unwrap();
    let db_path = not_a_directory.join("test.sqlite3");

    let err = open(db_path.to_str().unwrap()).unwrap_err();
    match err {
        rusqlite::Error::SqliteFailure(_, Some(message)) => {
            assert!(message.contains("failed to create parent directory"));
            assert!(message.contains("not-a-directory"));
        }
        other => panic!("expected sqlite failure with context, got {other:?}"),
    }
}

#[test]
fn test_open_repairs_missing_tables_when_user_version_is_current() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.sqlite3");
    let path_str = path.to_str().unwrap();

    let conn = Connection::open(path_str).unwrap();
    conn.execute_batch("PRAGMA user_version = 3;").unwrap();
    drop(conn);

    let conn = open(path_str).unwrap();
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(tables.contains(&"enrichment_cache".to_string()));
    assert!(tables.contains(&"audio_analysis_cache".to_string()));
    assert!(tables.contains(&"broker_discogs_session".to_string()));
    assert!(tables.contains(&"audit_files".to_string()));
    assert!(tables.contains(&"audit_issues".to_string()));
}

#[test]
fn test_enrichment_cache_round_trip() {
    let (_dir, conn) = open_temp_store();

    set_enrichment(
        &conn,
        "discogs",
        "burial",
        "archangel",
        Some("untrue"),
        Some("exact"),
        Some(r#"{"title":"Burial - Untrue","genres":["Electronic"]}"#),
    )
    .unwrap();

    let entry = get_enrichment(
        &conn,
        "discogs",
        "burial",
        "archangel",
        Some("untrue"),
        false,
    )
    .unwrap()
    .expect("should find cached entry");
    assert_eq!(entry.provider, "discogs");
    assert_eq!(entry.query_artist, "burial");
    assert_eq!(entry.query_title, "archangel");
    assert_eq!(entry.query_album, "untrue");
    assert_eq!(entry.match_quality.as_deref(), Some("exact"));
    assert!(entry.response_json.unwrap().contains("Burial"));
    assert!(!entry.created_at.is_empty());
}

#[test]
fn test_enrichment_cache_miss() {
    let (_dir, conn) = open_temp_store();
    let entry = get_enrichment(&conn, "discogs", "nobody", "nothing", None, false).unwrap();
    assert!(entry.is_none());
}

#[test]
fn test_enrichment_cache_upsert() {
    let (_dir, conn) = open_temp_store();

    set_enrichment(
        &conn,
        "discogs",
        "burial",
        "archangel",
        None,
        Some("fuzzy"),
        Some("old"),
    )
    .unwrap();
    set_enrichment(
        &conn,
        "discogs",
        "burial",
        "archangel",
        None,
        Some("exact"),
        Some("new"),
    )
    .unwrap();

    let entry = get_enrichment(&conn, "discogs", "burial", "archangel", None, false)
        .unwrap()
        .unwrap();
    assert_eq!(entry.match_quality.as_deref(), Some("exact"));
    assert_eq!(entry.response_json.as_deref(), Some("new"));
}

#[test]
fn test_enrichment_cache_no_match() {
    let (_dir, conn) = open_temp_store();

    set_enrichment(
        &conn,
        "discogs",
        "nobody",
        "nothing",
        None,
        Some("none"),
        None,
    )
    .unwrap();

    let entry = get_enrichment(&conn, "discogs", "nobody", "nothing", None, false)
        .unwrap()
        .unwrap();
    assert_eq!(entry.match_quality.as_deref(), Some("none"));
    assert!(entry.response_json.is_none());
}

#[test]
fn test_audio_analysis_cache_round_trip() {
    let (_dir, conn) = open_temp_store();

    set_audio_analysis_with_fingerprint(
        &conn,
        "/music/track.flac",
        "stratum-dsp",
        12345678,
        1700000000,
        "1.0.0",
        "hmm:v1",
        r#"{"bpm":128.0,"key":"Am"}"#,
    )
    .unwrap();

    let entry = get_audio_analysis(&conn, "/music/track.flac", "stratum-dsp")
        .unwrap()
        .expect("should find cached entry");
    assert_eq!(entry.file_path, "/music/track.flac");
    assert_eq!(entry.analyzer, "stratum-dsp");
    assert_eq!(entry.file_size, 12345678);
    assert_eq!(entry.file_mtime, 1700000000);
    assert_eq!(entry.analysis_version, "1.0.0");
    assert!(entry.features_json.contains("128.0"));
}

#[test]
fn test_audio_analysis_cache_miss() {
    let (_dir, conn) = open_temp_store();
    let entry = get_audio_analysis(&conn, "/no/such/file.flac", "stratum-dsp").unwrap();
    assert!(entry.is_none());
}

#[test]
fn test_audio_analysis_cache_upsert() {
    let (_dir, conn) = open_temp_store();

    set_audio_analysis_with_fingerprint(
        &conn,
        "/music/track.flac",
        "stratum-dsp",
        100,
        200,
        "1.0.0",
        "hmm:v1",
        "old",
    )
    .unwrap();
    set_audio_analysis_with_fingerprint(
        &conn,
        "/music/track.flac",
        "stratum-dsp",
        100,
        300,
        "1.1.0",
        "hmm:v1",
        "new",
    )
    .unwrap();

    let entry = get_audio_analysis(&conn, "/music/track.flac", "stratum-dsp")
        .unwrap()
        .unwrap();
    assert_eq!(entry.file_mtime, 300);
    assert_eq!(entry.analysis_version, "1.1.0");
    assert_eq!(entry.features_json, "new");
}

#[test]
#[cfg(target_os = "macos")]
fn test_broker_discogs_session_round_trip() {
    let (_dir, conn) = open_temp_store();
    let url = "https://broker.example.com/store-round-trip-test";

    // Ensure clean keychain state from any prior failed run.
    let _ = crate::adapters::platform::keychain::delete_session_token(url);

    set_broker_discogs_session(&conn, url, "session-token-1", 1_800_000_000).unwrap();

    let row = get_broker_discogs_session(&conn, url)
        .unwrap()
        .expect("broker session should exist");
    assert_eq!(row.broker_url, url);
    assert_eq!(row.session_token, "session-token-1");
    assert_eq!(row.expires_at, 1_800_000_000);
    assert!(!row.created_at.is_empty());
    assert!(!row.updated_at.is_empty());

    let db_token: String = conn
        .query_row(
            "SELECT session_token FROM broker_discogs_session WHERE broker_url = ?1",
            params![url],
            |row| row.get(0),
        )
        .unwrap();
    assert!(db_token.is_empty(), "token should not be stored in SQLite");

    set_broker_discogs_session(&conn, url, "session-token-2", 1_900_000_000).unwrap();
    let row = get_broker_discogs_session(&conn, url)
        .unwrap()
        .expect("broker session should still exist");
    assert_eq!(row.session_token, "session-token-2");
    assert_eq!(row.expires_at, 1_900_000_000);

    clear_broker_discogs_session(&conn, url).unwrap();
    let missing = get_broker_discogs_session(&conn, url).unwrap();
    assert!(missing.is_none());
}

#[test]
#[cfg(target_os = "macos")]
fn test_broker_discogs_session_migrates_legacy_plaintext() {
    let (_dir, conn) = open_temp_store();
    let url = "https://broker.example.com/store-migration-test";

    // Ensure clean keychain state.
    let _ = crate::adapters::platform::keychain::delete_session_token(url);

    conn.execute(
        "INSERT INTO broker_discogs_session (broker_url, session_token, expires_at)
         VALUES (?1, ?2, ?3)",
        params![url, "legacy-plaintext-token", 1_800_000_000],
    )
    .unwrap();

    let row = get_broker_discogs_session(&conn, url)
        .unwrap()
        .expect("session should exist after migration");
    assert_eq!(row.session_token, "legacy-plaintext-token");

    let db_token: String = conn
        .query_row(
            "SELECT session_token FROM broker_discogs_session WHERE broker_url = ?1",
            params![url],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        db_token.is_empty(),
        "plaintext token should be cleared from SQLite"
    );

    let kc_token = crate::adapters::platform::keychain::get_session_token(url).unwrap();
    assert_eq!(kc_token.as_deref(), Some("legacy-plaintext-token"));

    clear_broker_discogs_session(&conn, url).unwrap();
}

#[test]
fn test_audit_file_round_trip() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(
        &conn,
        "/music/track.flac",
        "2026-02-25T12:00:00Z",
        "v2:1771581600123456789:album",
        12345,
    )
    .unwrap();

    let entry = get_audit_file(&conn, "/music/track.flac")
        .unwrap()
        .expect("should find audit file");
    assert_eq!(entry.path, "/music/track.flac");
    assert_eq!(entry.last_audited, "2026-02-25T12:00:00Z");
    assert_eq!(entry.freshness_key, "v2:1771581600123456789:album");
    assert_eq!(entry.file_size, 12345);

    upsert_audit_file(
        &conn,
        "/music/retry.flac",
        "2026-02-25T12:00:01Z",
        "retry:read:1771581601123456789",
        23456,
    )
    .unwrap();
    let retry = get_audit_file(&conn, "/music/retry.flac").unwrap().unwrap();
    assert_eq!(retry.freshness_key, "retry:read:1771581601123456789");

    upsert_audit_file(
        &conn,
        "/music/legacy.flac",
        "2026-02-25T12:00:02Z",
        "2026-02-20T10:00:00Z",
        34567,
    )
    .unwrap();
    let legacy = get_audit_file(&conn, "/music/legacy.flac")
        .unwrap()
        .unwrap();
    assert_eq!(legacy.freshness_key, "2026-02-20T10:00:00Z");
}

#[test]
fn test_audit_file_upsert() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t1", "retry:metadata:1", 100).unwrap();
    upsert_audit_file(&conn, "/music/track.flac", "t2", "v2:2:loose", 200).unwrap();

    let entry = get_audit_file(&conn, "/music/track.flac").unwrap().unwrap();
    assert_eq!(entry.last_audited, "t2");
    assert_eq!(entry.freshness_key, "v2:2:loose");
    assert_eq!(entry.file_size, 200);
}

#[test]
fn test_audit_file_miss() {
    let (_dir, conn) = open_temp_store();
    let entry = get_audit_file(&conn, "/no/such/file").unwrap();
    assert!(entry.is_none());
}

#[test]
fn test_audit_files_in_scope() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/a/1.flac", "t", "m", 100).unwrap();
    upsert_audit_file(&conn, "/music/a/2.flac", "t", "m", 200).unwrap();
    upsert_audit_file(&conn, "/music/b/1.flac", "t", "m", 300).unwrap();

    let files = get_audit_files_in_scope(&conn, "/music/a/").unwrap();
    assert_eq!(files.len(), 2);

    let files = get_audit_files_in_scope(&conn, "/music/").unwrap();
    assert_eq!(files.len(), 3);

    let files = get_audit_files_in_scope(&conn, "/other/").unwrap();
    assert_eq!(files.len(), 0);
}

#[test]
fn test_audit_issue_round_trip() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.wav", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.wav",
        "WAV_TAG3_MISSING",
        Some(r#"{"fields":["artist"]}"#),
        "open",
        "2026-02-25T12:00:00Z",
    )
    .unwrap();

    let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, "/music/track.wav");
    assert_eq!(issues[0].issue_type, "WAV_TAG3_MISSING");
    assert_eq!(issues[0].status, "open");
    assert!(issues[0].detail.as_ref().unwrap().contains("artist"));
}

#[test]
fn test_audit_issue_upsert_preserves_accepted() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t1").unwrap();

    resolve_audit_issues(&conn, &[1], Resolution::AcceptedAsIs, None, "t2").unwrap();

    upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t3").unwrap();

    let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
    assert_eq!(issue.status, "accepted");
}

#[test]
fn test_audit_issue_reopen_clears_stale_resolution() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        None,
        "open",
        "t1",
    )
    .unwrap();

    resolve_audit_issues(&conn, &[1], Resolution::Fixed, Some("fixed upstream"), "t2").unwrap();

    let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
    assert_eq!(issue.status, "resolved");
    assert_eq!(issue.resolution.as_deref(), Some("fixed"));
    assert_eq!(issue.note.as_deref(), Some("fixed upstream"));
    assert_eq!(issue.resolved_at.as_deref(), Some("t2"));

    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        Some("d2"),
        "open",
        "t3",
    )
    .unwrap();

    let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
    assert_eq!(issue.status, "open");
    assert!(
        issue.resolution.is_none(),
        "resolution should be cleared on reopen"
    );
    assert!(issue.note.is_none(), "note should be cleared on reopen");
    assert!(
        issue.resolved_at.is_none(),
        "resolved_at should be cleared on reopen"
    );
}

#[test]
fn test_audit_issue_unique_constraint() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        Some("d1"),
        "open",
        "t1",
    )
    .unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        Some("d2"),
        "open",
        "t2",
    )
    .unwrap();

    let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].detail.as_deref(), Some("d2"));
}

#[test]
fn test_audit_cascade_delete() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        None,
        "open",
        "t1",
    )
    .unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_TITLE",
        None,
        "open",
        "t1",
    )
    .unwrap();

    let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
    assert_eq!(issues.len(), 2);

    delete_audit_file(&conn, "/music/track.flac").unwrap();

    let issues = get_audit_issues(&conn, "/music/", None, None, 100, 0).unwrap();
    assert_eq!(issues.len(), 0);
}

#[test]
fn test_audit_resolve_issues() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        None,
        "open",
        "t1",
    )
    .unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_TITLE",
        None,
        "open",
        "t1",
    )
    .unwrap();

    let count = resolve_audit_issues(
        &conn,
        &[1],
        Resolution::AcceptedAsIs,
        Some("intentional"),
        "t2",
    )
    .unwrap();
    assert_eq!(count, 1);

    let issue = get_audit_issue_by_id(&conn, 1).unwrap().unwrap();
    assert_eq!(issue.status, "accepted");
    assert_eq!(issue.resolution.as_deref(), Some("accepted_as_is"));
    assert_eq!(issue.note.as_deref(), Some("intentional"));

    let issue2 = get_audit_issue_by_id(&conn, 2).unwrap().unwrap();
    assert_eq!(issue2.status, "open");
}

#[test]
fn test_audit_query_filters() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
    upsert_audit_file(&conn, "/music/b.wav", "t", "m", 200).unwrap();
    upsert_audit_issue(&conn, "/music/a.flac", "EMPTY_ARTIST", None, "open", "t1").unwrap();
    upsert_audit_issue(
        &conn,
        "/music/b.wav",
        "WAV_TAG3_MISSING",
        None,
        "open",
        "t1",
    )
    .unwrap();
    resolve_audit_issues(&conn, &[1], Resolution::AcceptedAsIs, None, "t2").unwrap();

    let open = get_audit_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].issue_type, "WAV_TAG3_MISSING");

    let wav = get_audit_issues(&conn, "/music/", None, Some("WAV_TAG3_MISSING"), 100, 0).unwrap();
    assert_eq!(wav.len(), 1);

    let both = get_audit_issues(
        &conn,
        "/music/",
        Some("accepted"),
        Some("EMPTY_ARTIST"),
        100,
        0,
    )
    .unwrap();
    assert_eq!(both.len(), 1);
}

#[test]
fn test_audit_summary() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
    upsert_audit_file(&conn, "/music/b.wav", "t", "m", 200).unwrap();
    upsert_audit_issue(&conn, "/music/a.flac", "EMPTY_ARTIST", None, "open", "t1").unwrap();
    upsert_audit_issue(
        &conn,
        "/music/b.wav",
        "WAV_TAG3_MISSING",
        None,
        "open",
        "t1",
    )
    .unwrap();

    let summary = get_audit_summary(&conn, "/music/").unwrap();
    assert_eq!(summary.by_type_status.len(), 2);
}

#[test]
fn test_audit_delete_missing_files() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/a.flac", "t", "m", 100).unwrap();
    upsert_audit_file(&conn, "/music/b.flac", "t", "m", 200).unwrap();
    upsert_audit_file(&conn, "/music/c.flac", "t", "m", 300).unwrap();

    let existing: std::collections::HashSet<String> =
        ["/music/a.flac".to_string()].into_iter().collect();
    let count = delete_missing_audit_files(&conn, "/music/", &existing).unwrap();
    assert_eq!(count, 2);

    let files = get_audit_files_in_scope(&conn, "/music/").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/music/a.flac");
}

#[test]
fn test_audit_delete_missing_files_keyset_batches() {
    let (_dir, conn) = open_temp_store();

    for i in 0..1205 {
        let path = format!("/music/{i:04}.flac");
        upsert_audit_file(&conn, &path, "t", "m", i).unwrap();
    }

    let existing: std::collections::HashSet<String> = [
        "/music/0000.flac".to_string(),
        "/music/0600.flac".to_string(),
        "/music/1204.flac".to_string(),
    ]
    .into_iter()
    .collect();

    let count = delete_missing_audit_files(&conn, "/music/", &existing).unwrap();
    assert_eq!(count, 1202);

    let mut remaining = get_audit_files_in_scope(&conn, "/music/")
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect::<Vec<_>>();
    remaining.sort();

    assert_eq!(
        remaining,
        vec![
            "/music/0000.flac".to_string(),
            "/music/0600.flac".to_string(),
            "/music/1204.flac".to_string(),
        ]
    );
}

#[test]
fn test_audit_mark_issues_resolved_for_path() {
    let (_dir, conn) = open_temp_store();

    upsert_audit_file(&conn, "/music/track.flac", "t", "m", 100).unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_ARTIST",
        None,
        "open",
        "t1",
    )
    .unwrap();
    upsert_audit_issue(
        &conn,
        "/music/track.flac",
        "EMPTY_TITLE",
        None,
        "open",
        "t1",
    )
    .unwrap();
    upsert_audit_issue(&conn, "/music/track.flac", "GENRE_SET", None, "open", "t1").unwrap();

    let count =
        mark_issues_resolved_for_path(&conn, "/music/track.flac", &["GENRE_SET"], "t2").unwrap();
    assert_eq!(count, 2);

    let open = get_audit_issues(&conn, "/music/", Some("open"), None, 100, 0).unwrap();
    assert_eq!(open.len(), 1);
    assert_eq!(open[0].issue_type, "GENRE_SET");
}

#[test]
fn test_audit_files_in_scope_escapes_like_wildcards() {
    let (_dir, conn) = open_temp_store();
    upsert_audit_file(&conn, "/music/100%_done/track.flac", "t", "m", 100).unwrap();
    upsert_audit_file(&conn, "/music/100X_done/track.flac", "t", "m", 200).unwrap();

    let files = get_audit_files_in_scope(&conn, "/music/100%_done/").unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/music/100%_done/track.flac");
}

#[test]
fn test_batch_enrichment_existence() {
    let (_dir, conn) = open_temp_store();
    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_1",
        None,
        Some("exact"),
        Some("{}"),
    )
    .unwrap();
    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_2",
        None,
        Some("exact"),
        Some("{}"),
    )
    .unwrap();
    set_enrichment(
        &conn,
        "bandcamp",
        "artist_b",
        "title_3",
        None,
        None,
        Some("{}"),
    )
    .unwrap();

    let discogs = batch_enrichment_existence(&conn, "discogs", &["artist_a", "artist_b"]).unwrap();
    assert!(discogs.contains(&("artist_a".to_string(), "title_1".to_string())));
    assert!(discogs.contains(&("artist_a".to_string(), "title_2".to_string())));
    assert!(!discogs.contains(&("artist_b".to_string(), "title_3".to_string())));

    let bandcamp =
        batch_enrichment_existence(&conn, "bandcamp", &["artist_a", "artist_b"]).unwrap();
    assert!(bandcamp.contains(&("artist_b".to_string(), "title_3".to_string())));
    assert!(!bandcamp.contains(&("artist_a".to_string(), "title_1".to_string())));

    let empty = batch_enrichment_existence(&conn, "discogs", &[]).unwrap();
    assert!(empty.is_empty());

    let unknown = batch_enrichment_existence(&conn, "discogs", &["nobody"]).unwrap();
    assert!(unknown.is_empty());
}

#[test]
fn test_batch_enrichment_existence_chunking() {
    let (_dir, conn) = open_temp_store();
    let artists: Vec<String> = (0..1000).map(|i| format!("artist_{i}")).collect();
    for a in &artists {
        set_enrichment(
            &conn,
            "discogs",
            a,
            "title",
            None,
            Some("exact"),
            Some("{}"),
        )
        .unwrap();
    }

    let artist_refs: Vec<&str> = artists.iter().map(std::string::String::as_str).collect();
    let result = batch_enrichment_existence(&conn, "discogs", &artist_refs).unwrap();
    assert_eq!(result.len(), 1000);
    for a in &artists {
        assert!(result.contains(&(a.clone(), "title".to_string())));
    }
}

#[test]
fn test_batch_enrichment_with_results() {
    let (_dir, conn) = open_temp_store();

    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_1",
        None,
        Some("exact"),
        Some(r#"{"title":"T1"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_2",
        None,
        Some("fuzzy"),
        Some(r#"{"title":"T2"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_3",
        None,
        Some("none"),
        None,
    )
    .unwrap();

    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_4",
        None,
        Some("error"),
        None,
    )
    .unwrap();

    set_enrichment(
        &conn,
        "discogs",
        "artist_a",
        "title_5",
        None,
        None,
        Some("{}"),
    )
    .unwrap();

    let results = batch_enrichment_with_results(&conn, "discogs", &["artist_a"]).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&("artist_a".to_string(), "title_1".to_string())));
    assert!(results.contains(&("artist_a".to_string(), "title_2".to_string())));

    let empty = batch_enrichment_with_results(&conn, "discogs", &[]).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_batch_enrichment_with_label() {
    let (_dir, conn) = open_temp_store();

    set_enrichment(
        &conn,
        "bandcamp",
        "artist_a",
        "title_1",
        None,
        Some("exact"),
        Some(r#"{"label":"Hyperdub","title":"T1"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "bandcamp",
        "artist_a",
        "title_2",
        None,
        Some("exact"),
        Some(r#"{"label":"","title":"T2"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "bandcamp",
        "artist_a",
        "title_3",
        None,
        Some("fuzzy"),
        Some(r#"{"title":"T3"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "bandcamp",
        "artist_a",
        "title_4",
        None,
        Some("none"),
        Some(r#"{"label":"Warp"}"#),
    )
    .unwrap();

    set_enrichment(
        &conn,
        "bandcamp",
        "artist_a",
        "title_5",
        None,
        Some("exact"),
        None,
    )
    .unwrap();

    let results = batch_enrichment_with_label(&conn, "bandcamp", &["artist_a"]).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results.contains(&("artist_a".to_string(), "title_1".to_string())));

    let empty = batch_enrichment_with_label(&conn, "bandcamp", &[]).unwrap();
    assert!(empty.is_empty());
}

#[test]
fn test_timbral_norm_stats_round_trip() {
    let (_dir, conn) = open_temp_store();

    let none = get_timbral_norm_stats(&conn).unwrap();
    assert!(none.is_none());

    let stats = TimbralNormStats {
        dims: vec![(0.5, 0.1), (1.2, 0.3), (-0.8, 0.05)],
        sample_count: 42,
        source_fingerprint: "a".repeat(64),
        analysis_version: "2".to_string(),
        vector_schema_version: "1".to_string(),
    };
    save_timbral_norm_stats(&conn, &stats).unwrap();

    let loaded = get_timbral_norm_stats(&conn)
        .unwrap()
        .expect("should find stats");
    assert_eq!(loaded, stats);

    let stats2 = TimbralNormStats {
        dims: vec![(10.0, 2.0)],
        sample_count: 99,
        source_fingerprint: "b".repeat(64),
        analysis_version: "3".to_string(),
        vector_schema_version: "2".to_string(),
    };
    save_timbral_norm_stats(&conn, &stats2).unwrap();

    let loaded2 = get_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(loaded2, stats2);

    conn.execute_batch(
        "CREATE TRIGGER reject_second_timbral_dimension
         BEFORE INSERT ON timbral_norm_stats
         WHEN NEW.dimension_index = 1
         BEGIN
             SELECT RAISE(ABORT, 'reject second dimension');
         END;",
    )
    .unwrap();
    let rejected = TimbralNormStats {
        dims: vec![(1.0, 1.0), (2.0, 1.0)],
        sample_count: 2,
        source_fingerprint: "c".repeat(64),
        analysis_version: "2".to_string(),
        vector_schema_version: "1".to_string(),
    };
    assert!(save_timbral_norm_stats(&conn, &rejected).is_err());
    assert_eq!(get_timbral_norm_stats(&conn).unwrap().unwrap(), stats2);
}

#[test]
fn test_timbral_norm_stats_legacy_migration_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.sqlite3");
    let path_str = path.to_str().unwrap();
    let conn = Connection::open(path_str).unwrap();
    conn.execute_batch(
        "CREATE TABLE timbral_norm_stats (
             dimension_index INTEGER PRIMARY KEY,
             mean REAL NOT NULL,
             stddev REAL NOT NULL,
             sample_count INTEGER NOT NULL,
             computed_at TEXT NOT NULL DEFAULT (datetime('now'))
         );
         INSERT INTO timbral_norm_stats
             (dimension_index, mean, stddev, sample_count)
         VALUES (0, 1.0, 0.5, 2);
         PRAGMA user_version = 7;",
    )
    .unwrap();
    drop(conn);

    let conn = open(path_str).unwrap();
    let migrated = get_timbral_norm_stats(&conn).unwrap().unwrap();
    assert_eq!(migrated.source_fingerprint, "");
    assert_eq!(migrated.analysis_version, "");
    assert_eq!(migrated.vector_schema_version, "");
    let version: i32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, STORE_SCHEMA_VERSION);
    drop(conn);

    let reopened = open(path_str).unwrap();
    let columns: Vec<String> = reopened
        .prepare("PRAGMA table_info(timbral_norm_stats)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        columns
            .iter()
            .filter(|column| column.as_str() == "source_fingerprint")
            .count(),
        1
    );
    assert_eq!(
        columns
            .iter()
            .filter(|column| column.as_str() == "analysis_version")
            .count(),
        1
    );
    assert_eq!(
        columns
            .iter()
            .filter(|column| column.as_str() == "vector_schema_version")
            .count(),
        1
    );
}

#[test]
fn test_timbral_norm_stats_rejects_incoherent_rows() {
    let (_dir, conn) = open_temp_store();
    let stats = TimbralNormStats {
        dims: vec![(1.0, 0.5), (2.0, 0.75), (3.0, 1.0)],
        sample_count: 3,
        source_fingerprint: "d".repeat(64),
        analysis_version: "2".to_string(),
        vector_schema_version: "1".to_string(),
    };

    save_timbral_norm_stats(&conn, &stats).unwrap();
    conn.execute(
        "UPDATE timbral_norm_stats SET sample_count = 4 WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(get_timbral_norm_stats(&conn).unwrap().is_none());

    save_timbral_norm_stats(&conn, &stats).unwrap();
    conn.execute(
        "UPDATE timbral_norm_stats SET source_fingerprint = 'mismatch'
         WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(get_timbral_norm_stats(&conn).unwrap().is_none());

    save_timbral_norm_stats(&conn, &stats).unwrap();
    conn.execute(
        "UPDATE timbral_norm_stats SET analysis_version = 'mismatch'
         WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(get_timbral_norm_stats(&conn).unwrap().is_none());

    save_timbral_norm_stats(&conn, &stats).unwrap();
    conn.execute(
        "UPDATE timbral_norm_stats SET vector_schema_version = 'mismatch'
         WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(get_timbral_norm_stats(&conn).unwrap().is_none());

    save_timbral_norm_stats(&conn, &stats).unwrap();
    conn.execute(
        "DELETE FROM timbral_norm_stats WHERE dimension_index = 1",
        [],
    )
    .unwrap();
    assert!(get_timbral_norm_stats(&conn).unwrap().is_none());
}

#[test]
fn test_weight_preset_crud() {
    let (_dir, conn) = open_temp_store();

    let all = list_weight_presets(&conn, None).unwrap();
    assert!(all.is_empty());

    save_weight_preset(&conn, "chill", "transition", r#"{"energy":0.3,"key":0.7}"#).unwrap();
    save_weight_preset(
        &conn,
        "high-energy",
        "transition",
        r#"{"energy":0.9,"key":0.5}"#,
    )
    .unwrap();
    save_weight_preset(&conn, "club", "pool", r#"{"bpm":0.8}"#).unwrap();

    let all = list_weight_presets(&conn, None).unwrap();
    assert_eq!(all.len(), 3);

    let transition = list_weight_presets(&conn, Some("transition")).unwrap();
    assert_eq!(transition.len(), 2);
    assert_eq!(transition[0].name, "chill");
    assert_eq!(transition[1].name, "high-energy");

    let pool = list_weight_presets(&conn, Some("pool")).unwrap();
    assert_eq!(pool.len(), 1);
    assert_eq!(pool[0].name, "club");

    let json = get_weight_preset(&conn, "chill", "transition")
        .unwrap()
        .expect("should find preset");
    assert!(json.contains("energy"));

    let miss = get_weight_preset(&conn, "chill", "pool").unwrap();
    assert!(miss.is_none());

    let miss2 = get_weight_preset(&conn, "nonexistent", "transition").unwrap();
    assert!(miss2.is_none());

    save_weight_preset(&conn, "chill", "transition", r#"{"energy":0.2,"key":0.8}"#).unwrap();
    let updated = get_weight_preset(&conn, "chill", "transition")
        .unwrap()
        .unwrap();
    assert!(updated.contains("0.2"));

    let deleted = delete_weight_preset(&conn, "chill", "transition").unwrap();
    assert!(deleted);

    let deleted_again = delete_weight_preset(&conn, "chill", "transition").unwrap();
    assert!(!deleted_again);

    let remaining = list_weight_presets(&conn, None).unwrap();
    assert_eq!(remaining.len(), 2);
}
