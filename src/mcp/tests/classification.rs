use crate::application::classification::evaluate::{self, EvaluationCase};
use crate::application::{analysis::identity, classification as classification_workflow};
use crate::mcp::analysis::CacheCoverageParams;
use crate::mcp::classification;
use crate::mcp::classification::{
    AuditGenresParams, CalibrationCoverageParams, ClassifyFormat, ClassifyTracksParams,
};
use crate::mcp::library::SearchFilterParams;
use std::collections::HashSet;

use rmcp::handler::server::wrapper::Parameters;
use rusqlite::{Connection, params};
use serde::Deserialize;

use crate::adapters::{rekordbox as db, state as store};
use crate::domain::classification::taxonomy as genre;

use super::common::{
    call_tool_via_router, create_server_with_connections, create_single_track_test_db,
    default_http_client_for_tests, extract_json, insert_test_track, set_test_audio_analysis,
    valid_test_essentia_payload, write_test_audio_file,
};

const GOLDEN_GENRES_FIXTURE_PATH: &str = "src/mcp/classification/fixtures/golden_genres.json";

#[derive(Debug, Deserialize)]
struct GoldenGenreFixtureEntry {
    artist: String,
    title: String,
    expected_genre: String,
    notes: String,
}

fn load_golden_genres_fixture() -> Vec<GoldenGenreFixtureEntry> {
    let raw = std::fs::read_to_string(GOLDEN_GENRES_FIXTURE_PATH)
        .expect("golden genres fixture should be readable");
    serde_json::from_str(&raw).expect("golden genres fixture should be valid JSON")
}

fn find_track_by_artist_and_title(
    conn: &Connection,
    artist: &str,
    title: &str,
) -> Option<crate::domain::library::Track> {
    let sql = format!(
        "{}
             WHERE c.rb_local_deleted = 0
               AND lower(COALESCE(a.Name, '')) = lower(?1)
               AND lower(COALESCE(c.Title, '')) = lower(?2)
             LIMIT 1",
        db::TRACK_SELECT
    );
    let mut stmt = conn
        .prepare(&sql)
        .expect("fixture lookup query should prepare");
    let mut rows = stmt
        .query_map(params![artist, title], db::row_to_track)
        .expect("fixture lookup query should run");
    match rows.next() {
        Some(Ok(track)) => Some(track),
        Some(Err(_)) => panic!("private fixture lookup failed"),
        None => None,
    }
}

fn make_result(
    genre: Option<&'static str>,
    confidence: crate::domain::classification::ClassificationConfidence,
    action: crate::domain::classification::ClassificationAction,
    artist: &str,
) -> crate::domain::classification::ClassificationResult {
    crate::domain::classification::ClassificationResult {
        track_id: String::new(),
        artist: artist.to_string(),
        title: String::new(),
        current_genre: String::new(),
        genre,
        confidence,
        action,
        mode: crate::domain::classification::ClassificationMode::Full,
        degraded_reasons: vec![],
        evidence: vec![],
        candidates: vec![],
        flags: vec![],
        review_hint: None,
    }
}

#[tokio::test]
async fn get_genre_taxonomy_via_router_returns_genres() {
    let result = call_tool_via_router("get_genre_taxonomy", None).await;
    let payload = extract_json(&result);

    let genres = payload
        .get("genres")
        .and_then(serde_json::Value::as_array)
        .expect("genres should be present");
    assert!(
        !genres.is_empty(),
        "genres should include configured taxonomy entries"
    );
}

#[tokio::test]
async fn classify_tracks_does_not_auto_stage_stratum_only_audio() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("classify-stratum-only.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1000);
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("classify-stratum-only", &audio_path_str);
    db_conn
        .execute(
            "UPDATE djmdContent SET GenreID = '', BPM = 16000 WHERE ID = ?1",
            ["classify-stratum-only"],
        )
        .expect("BPM-only test track should be ungenred and fast");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    set_test_audio_analysis(
        &store_conn,
        &audio_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":160.0,"duration_seconds":240.0,"analyzer_version":"18"}"#,
    )
    .expect("fresh Stratum cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .classify_tracks(Parameters(ClassifyTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["classify-stratum-only".to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            offset: None,
            genre_overrides: None,
            format: Some(ClassifyFormat::Full),
            auto_stage: Some(vec![crate::mcp::classification::StageLevel::Medium]),
        }))
        .await
        .expect("classify_tracks should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["staging"]["staged"], 0);
    assert_eq!(payload["staging"]["total_pending"], 0);
    assert_eq!(payload["results"][0]["genre"], serde_json::Value::Null);
    assert_eq!(payload["results"][0]["confidence"], "insufficient");
}

#[tokio::test]
async fn auto_stage_degraded_low_is_skipped_even_when_low_is_requested() {
    let db_conn = create_single_track_test_db("degraded-stage", "/missing/degraded-stage.flac");
    db_conn
        .execute(
            "UPDATE djmdContent SET GenreID = '' WHERE ID = 'degraded-stage'",
            [],
        )
        .unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store_conn = store::open(store_dir.path().join("store.sqlite3").to_str().unwrap()).unwrap();
    let artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let title = crate::domain::metadata::normalize_for_matching("Señorita");
    let album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &artist,
        &title,
        Some(&album),
        Some("exact"),
        Some(r#"{"styles":["Techno"]}"#),
    )
    .unwrap();
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let payload = extract_json(
        &server
            .classify_tracks(Parameters(ClassifyTracksParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["degraded-stage".into()]),
                playlist_id: None,
                max_tracks: Some(1),
                offset: None,
                genre_overrides: None,
                format: Some(ClassifyFormat::Full),
                auto_stage: Some(vec![crate::mcp::classification::StageLevel::Low]),
            }))
            .await
            .unwrap(),
    );

    assert_eq!(payload["results"][0]["mode"], "degraded");
    assert_eq!(payload["results"][0]["confidence"], "low");
    assert_eq!(payload["summary"]["auto_stage_skipped_degraded"], 1);
    assert_eq!(payload["staging"]["skipped_degraded"], 1);
    assert_eq!(payload["staging"]["staged"], 0);
    assert_eq!(payload["staging"]["total_pending"], 0);
}

#[tokio::test]
async fn auto_stage_full_keeps_existing_action_and_confidence_filters() {
    let audio_dir = tempfile::tempdir().unwrap();
    let audio_path = audio_dir.path().join("full-stage.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 2048);
    let audio_path = audio_path.to_string_lossy().to_string();
    let db_conn = create_single_track_test_db("full-stage", &audio_path);
    db_conn
        .execute(
            "UPDATE djmdContent SET GenreID = '' WHERE ID = 'full-stage'",
            [],
        )
        .unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store_conn = store::open(store_dir.path().join("store.sqlite3").to_str().unwrap()).unwrap();
    let artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let title = crate::domain::metadata::normalize_for_matching("Señorita");
    let album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &artist,
        &title,
        Some(&album),
        Some("exact"),
        Some(r#"{"styles":["Techno"]}"#),
    )
    .unwrap();
    let essentia_payload = valid_test_essentia_payload(serde_json::json!({}));
    set_test_audio_analysis(
        &store_conn,
        &audio_path,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        "{}",
    )
    .unwrap();
    set_test_audio_analysis(
        &store_conn,
        &audio_path,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        file_size,
        file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &essentia_payload,
    )
    .unwrap();
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let payload = extract_json(
        &server
            .classify_tracks(Parameters(ClassifyTracksParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["full-stage".into()]),
                playlist_id: None,
                max_tracks: Some(1),
                offset: None,
                genre_overrides: None,
                format: Some(ClassifyFormat::Full),
                auto_stage: Some(vec![
                    crate::mcp::classification::StageLevel::High,
                    crate::mcp::classification::StageLevel::Medium,
                    crate::mcp::classification::StageLevel::Low,
                ]),
            }))
            .await
            .unwrap(),
    );

    assert_eq!(payload["results"][0]["mode"], "full");
    assert_eq!(payload["staging"]["skipped_degraded"], 0);
    assert_eq!(payload["staging"]["staged"], 1);
    assert_eq!(payload["staging"]["total_pending"], 1);
}

#[tokio::test]
async fn weak_confirmations_remain_visible_on_every_review_surface() {
    let db_conn = create_single_track_test_db("weak-confirm", "/missing/weak-confirm.flac");
    db_conn
        .execute(
            "UPDATE djmdContent SET BPM = 17000, LabelID = '' WHERE ID = 'weak-confirm'",
            [],
        )
        .unwrap();
    let store_dir = tempfile::tempdir().unwrap();
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).unwrap();
    let artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let title = crate::domain::metadata::normalize_for_matching("Señorita");
    let album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &artist,
        &title,
        Some(&album),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .unwrap();
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    for format in [
        ClassifyFormat::Full,
        ClassifyFormat::Compact,
        ClassifyFormat::Dispatch,
    ] {
        let payload = extract_json(
            &server
                .classify_tracks(Parameters(ClassifyTracksParams {
                    filters: SearchFilterParams::default(),
                    track_ids: Some(vec!["weak-confirm".into()]),
                    playlist_id: None,
                    max_tracks: Some(1),
                    offset: None,
                    genre_overrides: None,
                    format: Some(format),
                    auto_stage: Some(vec![crate::mcp::classification::StageLevel::Low]),
                }))
                .await
                .unwrap(),
        );
        assert_eq!(payload["summary"]["review_required"], 1);
        assert_eq!(payload["staging"]["staged"], 0);
        if format == ClassifyFormat::Dispatch {
            assert_eq!(payload["dispatch_stats"]["total_tracks"], 1);
        } else {
            assert_eq!(payload["results"].as_array().unwrap().len(), 1);
            assert_eq!(payload["results"][0]["action"], "confirm");
            assert_eq!(payload["results"][0]["confidence"], "low");
        }
        if format == ClassifyFormat::Full {
            assert_eq!(payload["needs_review"].as_array().unwrap().len(), 1);
        }
    }

    let audit = extract_json(
        &server
            .audit_genres(Parameters(AuditGenresParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["weak-confirm".into()]),
                playlist_id: None,
                max_tracks: Some(1),
                offset: None,
                include_confirmed: Some(false),
            }))
            .await
            .unwrap(),
    );
    assert_eq!(audit["summary"]["review_required"], 1);
    assert_eq!(audit["results"].as_array().unwrap().len(), 1);
    assert_eq!(audit["results"][0]["action"], "confirm");
}

#[tokio::test]
async fn classification_calibration_coverage_reports_verified_playlist_readiness() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let mut deep_paths = Vec::new();
    for i in 1..=5 {
        let path = audio_dir.path().join(format!("cal-deep-{i}.flac"));
        let (file_size, file_mtime) = write_test_audio_file(&path, 1000 + i);
        deep_paths.push((path.to_string_lossy().to_string(), file_size, file_mtime));
    }
    let tech_path = audio_dir.path().join("cal-tech-1.flac");
    let (tech_size, tech_mtime) = write_test_audio_file(&tech_path, 1100);
    let tech_path_str = tech_path.to_string_lossy().to_string();
    let no_genre_path = audio_dir.path().join("cal-no-genre.flac");
    write_test_audio_file(&no_genre_path, 1101);
    let no_genre_path_str = no_genre_path.to_string_lossy().to_string();
    let unknown_path = audio_dir.path().join("cal-unknown.flac");
    write_test_audio_file(&unknown_path, 1102);
    let unknown_path_str = unknown_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("cal-deep-1", &deep_paths[0].0);
    db_conn
        .execute_batch(
            "
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'Techno');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Imaginary Style');
            CREATE TABLE djmdPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                ParentID VARCHAR(255) DEFAULT '',
                Attribute INTEGER DEFAULT 0,
                Seq INTEGER DEFAULT 0,
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdSongPlaylist (
                PlaylistID VARCHAR(255),
                ContentID VARCHAR(255),
                TrackNo INTEGER
            );
            INSERT INTO djmdPlaylist (ID, Name, Seq) VALUES ('pl-verified', 'genre_verified', 1);
            ",
        )
        .expect("calibration coverage schema should initialize");

    for i in 2..=5 {
        insert_test_track(
            &db_conn,
            &format!("cal-deep-{i}"),
            &format!("Deep Verified {i}"),
            "g1",
            &deep_paths[i as usize - 1].0,
        );
    }
    insert_test_track(
        &db_conn,
        "cal-tech-1",
        "Techno Missing Audio",
        "g2",
        &tech_path_str,
    );
    insert_test_track(
        &db_conn,
        "cal-no-genre",
        "No Genre Verified",
        "",
        &no_genre_path_str,
    );
    insert_test_track(
        &db_conn,
        "cal-unknown",
        "Unknown Verified",
        "g3",
        &unknown_path_str,
    );

    for (track_no, track_id) in [
        "cal-deep-1",
        "cal-deep-2",
        "cal-deep-3",
        "cal-deep-4",
        "cal-deep-5",
        "cal-tech-1",
        "cal-no-genre",
        "cal-unknown",
    ]
    .iter()
    .enumerate()
    {
        db_conn
            .execute(
                "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
                params!["pl-verified", track_id, track_no as i64 + 1],
            )
            .expect("playlist entry should insert");
    }

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    for (path, file_size, file_mtime) in &deep_paths {
        let essentia_payload = valid_test_essentia_payload(serde_json::json!({
            "danceability": 0.71,
            "onset_rate": 4.2,
        }));
        set_test_audio_analysis(
            &store_conn,
            path,
            crate::adapters::audio::ANALYZER_STRATUM,
            *file_size,
            *file_mtime,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            r#"{"bpm":127.0,"decay_mid_tau":0.21,"key_clarity":0.72}"#,
        )
        .expect("stratum analysis should be seeded");
        set_test_audio_analysis(
            &store_conn,
            path,
            crate::adapters::audio::ANALYZER_ESSENTIA,
            *file_size,
            *file_mtime,
            crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
            &essentia_payload,
        )
        .expect("essentia analysis should be seeded");
    }
    set_test_audio_analysis(
        &store_conn,
        &tech_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        tech_size + 1,
        tech_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":132.0,"decay_mid_tau":240.0,"key_clarity":0.40}"#,
    )
    .expect("stale-identity stratum analysis should be seeded");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .calibration_coverage(Parameters(CalibrationCoverageParams {
            playlist: Some("genre_verified".to_string()),
        }))
        .await
        .expect("calibration_coverage should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["playlist"], "genre_verified");
    assert_eq!(payload["total_tracks"], 8);
    assert_eq!(payload["tracks_with_canonical_genre"], 6);
    assert_eq!(payload["tracks_with_audio_features"], 5);
    assert_eq!(payload["missing_audio_features"], 1);
    assert_eq!(payload["tracks_with_scorable_features"], 5);
    assert_eq!(payload["missing_scorable_features"], 1);
    assert_eq!(payload["tracks_with_stratum_features"], 5);
    assert_eq!(payload["missing_stratum_features"], 1);
    assert_eq!(payload["tracks_with_essentia_features"], 5);
    assert_eq!(payload["missing_essentia_features"], 1);
    assert_eq!(payload["tracks_with_complete_classification_audio"], 5);
    assert_eq!(payload["missing_required_stratum"], 1);
    assert_eq!(payload["missing_required_essentia"], 1);
    assert_eq!(payload["skipped_no_genre"], 1);
    assert_eq!(payload["skipped_unknown_genre"], 1);
    assert_eq!(
        payload["min_tracks_per_genre"],
        crate::domain::classification::profiles::MIN_TRACKS
    );
    assert_eq!(payload["genres_ready_to_calibrate"], 0);
    assert_eq!(payload["genres_below_min_tracks"], 1);

    let genres = payload["genres"]
        .as_array()
        .expect("genres should be an array");
    let deep_house = genres
        .iter()
        .find(|g| g["genre"] == "Deep House")
        .expect("Deep House coverage should be present");
    assert_eq!(deep_house["playlist_tracks"], 5);
    assert_eq!(deep_house["tracks_with_audio_features"], 5);
    assert_eq!(deep_house["tracks_with_scorable_features"], 5);
    assert_eq!(deep_house["tracks_with_stratum_features"], 5);
    assert_eq!(deep_house["tracks_with_essentia_features"], 5);
    assert_eq!(deep_house["tracks_with_complete_classification_audio"], 5);
    assert_eq!(deep_house["prototype_ready"], false);
    assert_eq!(deep_house["status"], "candidate_not_scorable");
    assert_eq!(deep_house["readiness_reason"], "candidate_not_scorable");

    let techno = genres
        .iter()
        .find(|g| g["genre"] == "Techno")
        .expect("Techno coverage should be present");
    assert_eq!(techno["playlist_tracks"], 1);
    assert_eq!(techno["tracks_with_audio_features"], 0);
    assert_eq!(techno["missing_audio_features"], 1);
    assert_eq!(techno["tracks_with_stratum_features"], 0);
    assert_eq!(techno["missing_stratum_features"], 1);
    assert_eq!(techno["status"], "needs_more_verified_audio");
    assert_eq!(
        techno["readiness_reason"],
        "incomplete_classification_audio"
    );

    let cache_payload = extract_json(
        &server
            .cache_coverage(Parameters(CacheCoverageParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["cal-deep-1".into(), "cal-tech-1".into()]),
                playlist_id: None,
                max_tracks: None,
            }))
            .await
            .expect("shared-fixture cache coverage should succeed"),
    );
    assert_eq!(cache_payload["classification_readiness"]["full"], 1);
    assert_eq!(cache_payload["classification_readiness"]["degraded"], 1);
    assert_eq!(
        cache_payload["classification_readiness"]["degraded_reasons"]["missing_stratum"],
        1
    );
    assert_eq!(
        cache_payload["classification_readiness"]["degraded_reasons"]["missing_essentia"],
        1
    );

    let classification_payload = extract_json(
        &server
            .classify_tracks(Parameters(ClassifyTracksParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["cal-deep-1".into(), "cal-tech-1".into()]),
                playlist_id: None,
                max_tracks: Some(2),
                offset: None,
                genre_overrides: None,
                format: Some(ClassifyFormat::Full),
                auto_stage: None,
            }))
            .await
            .expect("shared-fixture classification should succeed"),
    );
    let results = classification_payload["results"].as_array().unwrap();
    let deep_result = results
        .iter()
        .find(|result| result["track_id"] == "cal-deep-1")
        .unwrap();
    let tech_result = results
        .iter()
        .find(|result| result["track_id"] == "cal-tech-1")
        .unwrap();
    assert_eq!(deep_result["mode"], "full");
    assert_eq!(tech_result["mode"], "degraded");
    assert_eq!(
        tech_result["degraded_reasons"],
        serde_json::json!(["missing_stratum", "missing_essentia"])
    );
}

#[tokio::test]
async fn classification_calibration_coverage_reads_verified_playlist_without_ordinary_limit() {
    let db_conn = create_single_track_test_db("cal-cap-1", "/music/cal-cap-1.flac");
    db_conn
        .execute_batch(
            "
            CREATE TABLE djmdPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                ParentID VARCHAR(255) DEFAULT '',
                Attribute INTEGER DEFAULT 0,
                Seq INTEGER DEFAULT 0,
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdSongPlaylist (
                PlaylistID VARCHAR(255),
                ContentID VARCHAR(255),
                TrackNo INTEGER
            );
            INSERT INTO djmdPlaylist (ID, Name, Seq) VALUES ('pl-verified', 'genre_verified', 1);
            ",
        )
        .expect("calibration coverage schema should initialize");

    let mut track_ids = vec!["cal-cap-1".to_string()];
    for i in 2..=201 {
        let track_id = format!("cal-cap-{i}");
        insert_test_track(
            &db_conn,
            &track_id,
            &format!("Calibration Cap {i}"),
            "g1",
            &format!("/music/cal-cap-{i}.flac"),
        );
        track_ids.push(track_id);
    }

    for (track_no, track_id) in track_ids.iter().enumerate() {
        db_conn
            .execute(
                "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
                params!["pl-verified", track_id, track_no as i64 + 1],
            )
            .expect("playlist entry should insert");
    }

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .calibration_coverage(Parameters(CalibrationCoverageParams {
            playlist: Some("genre_verified".to_string()),
        }))
        .await
        .expect("calibration_coverage should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["total_tracks"], 201);
    assert_eq!(payload["tracks_with_canonical_genre"], 201);
    assert_eq!(payload["missing_audio_features"], 201);
    assert_eq!(payload["missing_stratum_features"], 201);
    assert_eq!(payload["missing_essentia_features"], 201);
    assert_eq!(
        payload["genres"][0]["playlist_tracks"], 201,
        "calibration coverage must not use the ordinary 200-track playlist cap"
    );
}

#[test]
fn golden_genres_fixture_is_well_formed() {
    let entries = load_golden_genres_fixture();
    assert!(
        !entries.is_empty(),
        "golden genres fixture should contain at least one entry"
    );

    let mut unique = HashSet::new();
    for entry in &entries {
        assert!(
            !entry.artist.trim().is_empty(),
            "fixture artist must be non-empty"
        );
        assert!(
            !entry.title.trim().is_empty(),
            "fixture title must be non-empty"
        );
        assert!(
            !entry.notes.trim().is_empty(),
            "fixture notes must be non-empty"
        );
        assert!(
            genre::is_known_genre(&entry.expected_genre),
            "expected_genre '{}' must be in taxonomy",
            entry.expected_genre
        );
        assert!(
            genre::canonical_genre_from_alias(&entry.expected_genre).is_none(),
            "expected_genre '{}' must be canonical, not alias",
            entry.expected_genre
        );

        let key = format!(
            "{}::{}",
            entry.artist.to_lowercase(),
            entry.title.to_lowercase()
        );
        assert!(unique.insert(key), "duplicate (artist, title) in fixture");
    }
}

#[test]
#[ignore]
fn private_rekordbox_golden_dataset_genre_accuracy() {
    let entries = load_golden_genres_fixture();
    let fixture = crate::adapters::rekordbox::test_support::PrivateRekordboxFixture::from_env()
        .expect("classifier benchmark requires REKORDBOX_TEST_BACKUP");
    let conn = fixture
        .open()
        .expect("classifier benchmark fixture should open read-only");
    let store_path = store::resolve_path();
    let store_conn = store::open_read_only(
        store_path
            .to_str()
            .expect("classifier benchmark store path should be UTF-8"),
    )
    .unwrap_or_else(|_| {
        panic!("classifier benchmark requires the existing local enrichment/audio store")
    });

    let mut predictive_tracks = Vec::new();
    let mut truths = Vec::new();
    let mut missing_track_count = 0;

    for entry in &entries {
        let Some(mut track) = find_track_by_artist_and_title(&conn, &entry.artist, &entry.title)
        else {
            missing_track_count += 1;
            continue;
        };
        let truth = genre::resolve_genre(&entry.expected_genre)
            .expect("validated fixture truth should resolve canonically");
        // The field under evaluation is withheld from every predictive mode.
        track.genre.clear();
        predictive_tracks.push(track);
        truths.push(truth);
    }

    assert!(
        !predictive_tracks.is_empty(),
        "classifier benchmark must evaluate at least one fixture track"
    );

    let audio_identities = identity::audio_cache_identities_with_rekordbox_connection(
        predictive_tracks
            .iter()
            .map(|track| track.file_path.as_str()),
        &conn,
    );
    let (rules_results, _) = classification_workflow::classify_batch_with_audio_identities(
        &store_conn,
        &predictive_tracks,
        &[],
        false,
        audio_identities.clone(),
    )
    .unwrap_or_else(|_| panic!("rules-only benchmark classification should succeed"));
    let rules_cases: Vec<_> = truths
        .iter()
        .zip(&rules_results)
        .map(|(&truth, result)| EvaluationCase {
            truth,
            result,
            source_stratum: benchmark_source_stratum(result),
            discogs_match_quality: benchmark_discogs_quality(result),
        })
        .collect();

    let deployed_registry_present = store::classification::load_from_db(&store_conn, None)
        .unwrap_or_else(|_| panic!("profile registry diagnostic should load"))
        .registry
        .is_some();
    let (deployed_results, _) = classification_workflow::classify_batch_with_audio_identities(
        &store_conn,
        &predictive_tracks,
        &[],
        true,
        audio_identities,
    )
    .unwrap_or_else(|_| panic!("deployed-registry diagnostic classification should succeed"));
    let deployed_cases: Vec<_> = truths
        .iter()
        .zip(&deployed_results)
        .map(|(&truth, result)| EvaluationCase {
            truth,
            result,
            source_stratum: benchmark_source_stratum(result),
            discogs_match_quality: benchmark_discogs_quality(result),
        })
        .collect();

    let summary = serde_json::json!({
        "benchmark_schema": 1,
        "predictive_current_genre": "withheld",
        "fixture_rows": entries.len(),
        "missing_rows": missing_track_count,
        "rules_only": evaluate::evaluate(&rules_cases, missing_track_count),
        "deployed_registry_diagnostic": {
            "acceptance_evidence": false,
            "registry_present": deployed_registry_present,
            "metrics": evaluate::evaluate(&deployed_cases, missing_track_count),
        },
        "versions": {
            "classifier_profile_schema": crate::domain::classification::profiles::PROFILE_SCHEMA_VERSION,
            "stratum": crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            "essentia": crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        },
    });
    let _serialized_summary =
        serde_json::to_string_pretty(&summary).expect("benchmark summary should serialize");
}

fn benchmark_source_stratum(
    result: &crate::domain::classification::ClassificationResult,
) -> &'static str {
    let discogs = result
        .evidence
        .iter()
        .any(|line| line.starts_with("discogs:"));
    let label = result
        .evidence
        .iter()
        .any(|line| line.starts_with("label:") && line.contains("[source=rekordbox]"));
    let audio = result.evidence.iter().any(|line| {
        line.starts_with("audio:") || line.starts_with("audio ") || line.starts_with("D.")
    });
    match (discogs, label, audio) {
        (true, true, true) => "discogs+label+audio",
        (true, true, false) => "discogs+label",
        (true, false, true) => "discogs+audio",
        (false, true, true) => "label+audio",
        (true, false, false) => "discogs",
        (false, true, false) => "label",
        (false, false, true) => "audio",
        (false, false, false) => "none",
    }
}

fn benchmark_discogs_quality(
    result: &crate::domain::classification::ClassificationResult,
) -> &'static str {
    if result
        .flags
        .iter()
        .any(|flag| flag == "discogs-match-invalid")
    {
        return "invalid";
    }
    let discogs = result
        .evidence
        .iter()
        .find(|line| line.starts_with("discogs:"));
    match discogs {
        Some(line) if line.contains("[match=exact]") => "exact",
        Some(line) if line.contains("[match=fuzzy]") => "fuzzy",
        Some(line) if line.contains("[match=invalid]") => "invalid",
        Some(_) => "unknown",
        None => "not_usable",
    }
}

#[test]
fn genre_distribution_empty_input() {
    let dist = classification::build_genre_distribution(&[]);
    assert_eq!(dist, serde_json::json!([]));
}

#[test]
fn genre_distribution_excludes_strong_confirm_but_keeps_weak_confirm() {
    use crate::domain::classification::{ClassificationAction as A, ClassificationConfidence as C};

    let results = vec![
        make_result(Some("Techno"), C::High, A::Confirm, "Artist A"),
        make_result(Some("Techno"), C::Low, A::Confirm, "Artist Weak"),
        make_result(None, C::Insufficient, A::Suggest, "Artist B"),
        make_result(Some("House"), C::Medium, A::Suggest, "Artist C"),
    ];
    let dist = classification::build_genre_distribution(&results);
    let arr = dist.as_array().unwrap();
    assert_eq!(
        arr.len(),
        2,
        "House and the weak Techno confirmation appear"
    );
    assert_eq!(arr[0]["count"], 1);
}

#[test]
fn genre_distribution_groups_and_sorts_by_count() {
    use crate::domain::classification::{ClassificationAction as A, ClassificationConfidence as C};

    let results = vec![
        make_result(Some("Techno"), C::High, A::Suggest, "A"),
        make_result(Some("Techno"), C::High, A::Suggest, "B"),
        make_result(Some("Techno"), C::Medium, A::Suggest, "A"),
        make_result(Some("House"), C::High, A::Suggest, "C"),
    ];
    let dist = classification::build_genre_distribution(&results);
    let arr = dist.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Techno (3) should be first, House (1) second
    assert_eq!(arr[0]["genre"], "Techno");
    assert_eq!(arr[0]["count"], 3);
    assert_eq!(arr[0]["by_confidence"]["high"], 2);
    assert_eq!(arr[0]["by_confidence"]["medium"], 1);
    assert_eq!(arr[1]["genre"], "House");
    assert_eq!(arr[1]["count"], 1);
}

#[test]
fn genre_distribution_top_artists_capped_and_counted() {
    use crate::domain::classification::{ClassificationAction as A, ClassificationConfidence as C};

    let mut results = Vec::new();
    // 6 different artists -- top_artists should cap at 5
    for i in 0..6 {
        results.push(make_result(
            Some("Techno"),
            C::High,
            A::Suggest,
            &format!("Artist {i}"),
        ));
    }
    // Artist 0 appears at two confidence levels -- should show "(2)"
    results.push(make_result(
        Some("Techno"),
        C::Medium,
        A::Suggest,
        "Artist 0",
    ));

    let dist = classification::build_genre_distribution(&results);
    let arr = dist.as_array().unwrap();
    let top = arr[0]["top_artists"].as_array().unwrap();
    assert_eq!(top.len(), 5, "top_artists capped at 5");
    // First entry should be "Artist 0 (2)" since it has the highest count
    assert_eq!(top[0].as_str().unwrap(), "Artist 0 (2)");
}
