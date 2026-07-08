use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rusqlite::{Connection, params};
use schemars::JsonSchema;
use serde::Deserialize;
use tempfile::TempDir;

use crate::genre;
use crate::types::TrackChange;

fn extract_json(result: &CallToolResult) -> serde_json::Value {
    let text = result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("tool result should include text content");

    serde_json::from_str(text).expect("tool text content should be valid JSON")
}

async fn call_tool_via_router(
    tool_name: &str,
    arguments: Option<serde_json::Map<String, serde_json::Value>>,
) -> CallToolResult {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let (server_result, client_result) = tokio::join!(
        ReklawdboxServer::new(None).serve(server_io),
        ().serve(client_io)
    );
    let mut server = server_result.expect("server should start over in-memory transport");
    let mut client = client_result.expect("client should connect over in-memory transport");

    let result = client
        .call_tool(CallToolRequestParams {
            meta: None,
            name: tool_name.to_owned().into(),
            arguments,
            task: None,
        })
        .await
        .expect("tool call through router should succeed");

    client
        .close()
        .await
        .expect("client should close cleanly after tool call");
    server
        .close()
        .await
        .expect("server should close cleanly after tool call");

    result
}

const GOLDEN_GENRES_FIXTURE_PATH: &str = "src/tools/fixtures/golden_genres.json";

#[derive(Debug, Deserialize)]
struct GoldenGenreFixtureEntry {
    artist: String,
    title: String,
    expected_genre: String,
    notes: String,
}

fn default_http_client_for_tests() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Reklawdbox/0.1")
        .build()
        .expect("default test HTTP client should build")
}

fn backup_script_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn create_server_with_connections(
    db_conn: Connection,
    store_conn: Connection,
    http: reqwest::Client,
) -> ReklawdboxServer {
    create_server_with_store_path(db_conn, store_conn, http, None)
}

fn create_server_with_store_path(
    db_conn: Connection,
    store_conn: Connection,
    http: reqwest::Client,
    store_path: Option<String>,
) -> ReklawdboxServer {
    let server = ReklawdboxServer {
        state: Arc::new(ServerState {
            db: OnceLock::new(),
            internal_db: OnceLock::new(),
            essentia_python: OnceLock::new(),
            essentia_python_override: Mutex::new(None),
            essentia_setup_lock: tokio::sync::Mutex::new(()),
            discogs_pending: Mutex::new(None),
            db_path: None,
            store_path,
            changes: ChangeManager::new(),
            http,
            label_research_gate: std::sync::atomic::AtomicU32::new(0),
        }),
        tool_router: ReklawdboxServer::tool_router(),
    };

    server
        .state
        .db
        .set(Ok(Mutex::new(db_conn)))
        .expect("test DB should initialize exactly once");
    server
        .state
        .internal_db
        .set(Ok(Mutex::new(store_conn)))
        .expect("test internal store should initialize exactly once");

    server
}

fn create_real_server_with_temp_store(
    http: reqwest::Client,
) -> Option<(ReklawdboxServer, TempDir)> {
    let db_conn = db::open_real_db()?;
    let store_dir = tempfile::tempdir().ok()?;
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn =
        store::open(&store_path_str).expect("internal store should open for integration test");

    let server = create_server_with_store_path(db_conn, store_conn, http, Some(store_path_str));
    Some((server, store_dir))
}

fn sample_real_tracks(server: &ReklawdboxServer, limit: u32) -> Vec<crate::types::Track> {
    let conn = server
        .rekordbox_conn()
        .expect("real DB connection should be available for integration test");
    db::search_tracks(
        &conn,
        &db::SearchParams {
            has_genre: Some(true),
            exclude_samples: true,
            limit: Some(limit),
            ..Default::default()
        },
    )
    .expect("sample search should succeed")
    .into_iter()
    .filter(|t| !t.artist.trim().is_empty() && !t.title.trim().is_empty())
    .collect()
}

fn write_test_audio_file(path: &std::path::Path, size: usize) -> (i64, i64) {
    let data = vec![b'a'; size];
    std::fs::write(path, data).expect("test audio file should write");
    let metadata = std::fs::metadata(path).expect("test audio file metadata should exist");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    (file_size, file_mtime)
}

fn create_single_track_test_db(track_id: &str, raw_file_path: &str) -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory DB should open");
    conn.execute_batch(
        "
            CREATE TABLE djmdArtist (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdAlbum (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdGenre (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdKey (
                ID VARCHAR(255) PRIMARY KEY,
                ScaleName VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdLabel (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdColor (
                ID VARCHAR(255) PRIMARY KEY,
                ColorCode INTEGER,
                Commnt VARCHAR(255),
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdContent (
                ID VARCHAR(255) PRIMARY KEY,
                Title VARCHAR(255),
                ArtistID VARCHAR(255),
                AlbumID VARCHAR(255),
                GenreID VARCHAR(255),
                KeyID VARCHAR(255),
                ColorID VARCHAR(255),
                LabelID VARCHAR(255),
                RemixerID VARCHAR(255),
                BPM INTEGER DEFAULT 0,
                Rating INTEGER DEFAULT 0,
                Commnt TEXT DEFAULT '',
                ReleaseYear INTEGER DEFAULT 0,
                Length INTEGER DEFAULT 0,
                FolderPath VARCHAR(255) DEFAULT '',
                DJPlayCount VARCHAR(255) DEFAULT '0',
                BitRate INTEGER DEFAULT 0,
                SampleRate INTEGER DEFAULT 0,
                FileType INTEGER DEFAULT 0,
                created_at TEXT DEFAULT '',
                rb_local_deleted INTEGER DEFAULT 0
            );

            INSERT INTO djmdArtist (ID, Name) VALUES ('a1', 'Aníbal');
            INSERT INTO djmdAlbum (ID, Name) VALUES ('al1', 'Encoded Paths');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g1', 'Deep House');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k1', 'Am');
            INSERT INTO djmdLabel (ID, Name) VALUES ('l1', 'Test Label');
            INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c1', 16711935, 'Rose');
            ",
    )
    .expect("test schema should initialize");

    conn.execute(
        "INSERT INTO djmdContent (
                ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                SampleRate, FileType, created_at, rb_local_deleted
            ) VALUES (
                ?1, 'Señorita', 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                12800, 204, 'percent path test', 2025, 240, ?2, '0', 1411,
                44100, 5, '2025-01-01', 0
            )",
        params![track_id, raw_file_path],
    )
    .expect("test track should insert");

    conn
}

fn insert_test_track(
    conn: &Connection,
    track_id: &str,
    title: &str,
    genre_id: &str,
    file_path: &str,
) {
    conn.execute(
        "INSERT INTO djmdContent (
                ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                SampleRate, FileType, created_at, rb_local_deleted
            ) VALUES (
                ?1, ?2, 'a1', 'al1', ?3, 'k1', 'c1', 'l1', '',
                12700, 102, 'cache coverage test', 2025, 220, ?4, '0', 1411,
                44100, 5, '2025-01-02', 0
            )",
        params![track_id, title, genre_id, file_path],
    )
    .expect("test track should insert");
}

fn canonical_genre_name(raw_genre: &str) -> String {
    if let Some(canonical) = genre::canonical_genre_name(raw_genre) {
        return canonical.to_string();
    }
    if let Some(alias_target) = genre::canonical_genre_from_alias(raw_genre) {
        return alias_target.to_string();
    }
    raw_genre.to_string()
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
) -> Option<crate::types::Track> {
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
        Some(Err(e)) => panic!("fixture lookup failed for {artist} - {title}: {e}"),
        None => None,
    }
}

fn create_build_set_test_db() -> (Connection, Vec<String>, TempDir) {
    let audio_dir = tempfile::tempdir().expect("build_set temp audio dir should create");
    let first_track_path = audio_dir.path().join("set-track-1.flac");
    let conn = create_single_track_test_db(
        "set-track-1",
        first_track_path
            .to_str()
            .expect("first build_set track path should be UTF-8"),
    );
    conn.execute_batch(
        "
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'House');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Tech House');

            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Bm');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k4', 'F#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k5', 'C#m');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k6', 'Dm');
            ",
    )
    .expect("build_set fixture taxonomy inserts should succeed");

    let tracks: [(&str, &str, &str, &str, i32, i32); 5] = [
        ("set-track-2", "Second Step", "g1", "k2", 12400, 300),
        ("set-track-3", "Third Wave", "g2", "k3", 12600, 0),
        ("set-track-4", "Fourth Lift", "g3", "k4", 12800, 360),
        ("set-track-5", "Fifth Peak", "g3", "k5", 12950, 420),
        ("set-track-6", "Sixth Release", "g2", "k6", 12350, 250),
    ];

    for (index, (track_id, title, genre_id, key_id, bpm, length)) in tracks.iter().enumerate() {
        conn.execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', ?3, ?4, 'c1', 'l1', '',
                    ?5, 153, 'build_set fixture', 2025, ?6, ?7, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params![
                *track_id,
                *title,
                *genre_id,
                *key_id,
                *bpm,
                *length,
                audio_dir
                    .path()
                    .join(format!("{track_id}.flac"))
                    .to_string_lossy()
                    .to_string(),
            ],
        )
        .unwrap_or_else(|e| panic!("fixture track insert {index} should succeed: {e}"));
    }

    (
        conn,
        vec![
            "set-track-1".to_string(),
            "set-track-2".to_string(),
            "set-track-3".to_string(),
            "set-track-4".to_string(),
            "set-track-5".to_string(),
            "set-track-6".to_string(),
        ],
        audio_dir,
    )
}

fn seed_build_set_cache(store_conn: &Connection, audio_dir: &std::path::Path) {
    let rows: [(&str, f64, &str, f64); 6] = [
        ("set-track-1.flac", 122.0, "8A", 1.02),
        ("set-track-2.flac", 124.0, "9A", 1.20),
        ("set-track-3.flac", 126.0, "10A", 1.44),
        ("set-track-4.flac", 128.0, "11A", 1.80),
        ("set-track-5.flac", 130.0, "12A", 2.22),
        ("set-track-6.flac", 123.5, "7A", 1.26),
    ];

    for (index, (file_name, bpm, key_camelot, danceability)) in rows.iter().enumerate() {
        let path = audio_dir.join(file_name);
        let (file_size, file_mtime) = write_test_audio_file(&path, 1000 + index);
        let stratum = serde_json::json!({
            "bpm": *bpm,
            "key": "Am",
            "key_camelot": *key_camelot,
            "analyzer_version": "stratum-dsp-test"
        });
        let essentia = serde_json::json!({
            "danceability": *danceability,
            "loudness_integrated": -18.0 + (*danceability * 4.0),
            "onset_rate": 2.5 + (*danceability * 2.0),
            "analyzer_version": "essentia-test"
        });
        store::set_audio_analysis(
            store_conn,
            path.to_str().expect("seed path should be UTF-8"),
            "stratum-dsp",
            file_size,
            file_mtime,
            "stratum-dsp-test",
            &stratum.to_string(),
        )
        .unwrap_or_else(|e| panic!("stratum cache seed {index} should succeed: {e}"));
        store::set_audio_analysis(
            store_conn,
            path.to_str().expect("seed path should be UTF-8"),
            "essentia",
            file_size,
            file_mtime,
            "essentia-test",
            &essentia.to_string(),
        )
        .unwrap_or_else(|e| panic!("essentia cache seed {index} should succeed: {e}"));
    }
}

#[tokio::test]
async fn build_set_generates_candidates_and_transition_scores() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: Some(EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            opening_track_id: None,
            candidates: Some(3),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed for fixture pool");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 6);
    assert_eq!(payload["tracks_used"], 4);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert_eq!(candidates.len(), 3);

    for candidate in candidates {
        let tracks = candidate["tracks"]
            .as_array()
            .expect("candidate tracks should be an array");
        let transitions = candidate["transitions"]
            .as_array()
            .expect("candidate transitions should be an array");
        assert_eq!(tracks.len(), 4);
        assert_eq!(transitions.len(), 3);
        assert!(
            candidate["set_score"].as_f64().is_some(),
            "set_score should be numeric"
        );
        let set_score = candidate["set_score"]
            .as_f64()
            .expect("set_score should be numeric");
        assert!(
            (set_score - round_to_3_decimals(set_score)).abs() < 1e-9,
            "set_score should be rounded to 3 decimal places"
        );
        assert!(
            candidate["estimated_duration_minutes"].as_i64().is_some(),
            "estimated_duration_minutes should be numeric"
        );
        for transition in transitions {
            assert!(
                transition["scores"]["composite"].as_f64().is_some(),
                "each transition should include numeric composite score"
            );
            assert!(
                transition["key_relation"].is_string(),
                "each transition should include key_relation"
            );
            assert!(
                transition["bpm_adjustment_pct"].is_number(),
                "each transition should include bpm_adjustment_pct"
            );
        }
    }

    let candidate_a_ids: Vec<String> = candidates[0]["tracks"]
        .as_array()
        .expect("candidate A tracks array")
        .iter()
        .map(|track| {
            track["track_id"]
                .as_str()
                .expect("candidate track should include track_id")
                .to_string()
        })
        .collect();
    let candidate_b_ids: Vec<String> = candidates[1]["tracks"]
        .as_array()
        .expect("candidate B tracks array")
        .iter()
        .map(|track| {
            track["track_id"]
                .as_str()
                .expect("candidate track should include track_id")
                .to_string()
        })
        .collect();
    assert_ne!(
        candidate_a_ids, candidate_b_ids,
        "candidate generation should include variation"
    );
}

#[tokio::test]
async fn build_set_adapts_energy_curve_to_single_track_pool() {
    let db_conn = create_single_track_test_db("single-set-1", "/tmp/single-set-1.flac");
    db_conn
        .execute(
            "UPDATE djmdContent SET Length = 0 WHERE ID = ?1",
            params!["single-set-1"],
        )
        .expect("single-track fixture should update");

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
        .build_set(Parameters(BuildSetParams {
            track_ids: vec!["single-set-1".to_string()],
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("energy".into())),
            energy_curve: Some(EnergyCurveInput::Custom(vec![
                EnergyPhase::Warmup,
                EnergyPhase::Build,
                EnergyPhase::Peak,
                EnergyPhase::Release,
            ])),
            opening_track_id: None,
            candidates: Some(2),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed for single-track pool");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 1);
    assert_eq!(payload["tracks_used"], 1);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert_eq!(candidates.len(), 1);
    let first = &candidates[0];
    assert_eq!(
        first["tracks"]
            .as_array()
            .expect("tracks should be array")
            .len(),
        1
    );
    assert_eq!(
        first["transitions"]
            .as_array()
            .expect("transitions should be array")
            .len(),
        0
    );
    assert_eq!(
        first["estimated_duration_minutes"]
            .as_i64()
            .expect("duration should be integer"),
        6
    );
}

#[tokio::test]
async fn build_set_produces_candidates_from_homogeneous_key_pool() {
    let db_conn = create_single_track_test_db("same-key-1", "/tmp/same-key-1.flac");
    insert_test_track(
        &db_conn,
        "same-key-2",
        "Same Key Two",
        "g1",
        "/tmp/same-key-2.flac",
    );
    insert_test_track(
        &db_conn,
        "same-key-3",
        "Same Key Three",
        "g1",
        "/tmp/same-key-3.flac",
    );

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
        .build_set(Parameters(BuildSetParams {
            track_ids: vec![
                "same-key-1".to_string(),
                "same-key-2".to_string(),
                "same-key-3".to_string(),
            ],
            target_tracks: 3,
            priority: Some(TransitionWeightSpec::Named("harmonic".into())),
            energy_curve: Some(EnergyCurveInput::Preset(EnergyCurvePreset::FlatEnergy)),
            opening_track_id: None,
            candidates: Some(2),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed when all tracks share the same key");
    let payload = extract_json(&result);

    assert_eq!(payload["pool_size"], 3);
    assert_eq!(payload["tracks_used"], 3);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    // With beam search (beam_width=2 from candidates), beam explores different
    // orderings of the same 3-track pool, yielding 1 or 2 candidates.
    assert!(
        !candidates.is_empty() && candidates.len() <= 2,
        "same-key pool with beam_width=2 should produce 1-2 candidates; got {}",
        candidates.len()
    );
    assert_eq!(
        candidates[0]["transitions"]
            .as_array()
            .expect("transitions should be an array")
            .len(),
        2
    );
}

#[tokio::test]
async fn build_set_recomputes_preset_curve_when_pool_is_smaller_than_target() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let selected: Vec<String> = track_ids.into_iter().take(3).collect();

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: selected,
            target_tracks: 6,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: Some(EnergyCurveInput::Preset(
                EnergyCurvePreset::WarmupBuildPeakRelease,
            )),
            opening_track_id: None,
            candidates: Some(1),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set should succeed when pool is smaller than target");
    let payload = extract_json(&result);

    assert_eq!(payload["tracks_used"], 3);
    let transitions = payload["candidates"][0]["transitions"]
        .as_array()
        .expect("candidate transitions should be an array");
    assert_eq!(transitions.len(), 2);
    let second_energy_label = transitions[1]["scores"]["energy"]["label"]
        .as_str()
        .expect("second transition should include energy label");
    assert!(
        second_energy_label.contains("peak phase"),
        "phase scaling should include a peak phase for the final transition when tracks_used=3; got: {second_energy_label}"
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_prefers_env_override_when_valid() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python");
    std::fs::write(&fake_python, "#!/bin/sh\necho '2.1b6.dev1389'\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let resolved = probe_essentia_python_from_sources(
        fake_python.to_str(),
        Some(dir.path().join("missing-default-python")),
    );

    assert_eq!(
        resolved.as_deref(),
        fake_python.to_str(),
        "valid env override should win over default candidate"
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_fails_when_no_version_string() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python-empty");
    std::fs::write(&fake_python, "#!/bin/sh\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let resolved =
        probe_essentia_python_from_sources(fake_python.to_str(), Some(dir.path().join("other")));
    assert!(
        resolved.is_none(),
        "probe should reject candidates that do not emit version output"
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_returns_error_on_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python-slow");
    std::fs::write(&fake_python, "#!/bin/sh\nexec sleep 2\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let start = std::time::Instant::now();
    let is_valid = validate_essentia_python_with_timeout(
        fake_python.to_str().unwrap(),
        Duration::from_millis(100),
    );
    assert!(
        !is_valid,
        "slow candidate should be rejected when probe timeout elapses"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "probe timeout should fail fast"
    );
}

#[test]
fn lookup_output_wraps_non_object_in_result_envelope() {
    let output = lookup_output_with_cache_metadata(serde_json::Value::Null, false, None);
    assert_eq!(output["result"], serde_json::Value::Null);
    assert_eq!(output["cache_hit"], false);
    assert!(
        output.get("cached_at").is_none(),
        "live payload should not include cached_at"
    );
}

#[test]
fn lookup_output_with_cache_metadata_keeps_object_payload_shape() {
    let output = lookup_output_with_cache_metadata(
        serde_json::json!({
            "genre": "Techno"
        }),
        true,
        Some("2026-02-20T10:00:00Z"),
    );
    assert_eq!(output["genre"], "Techno");
    assert_eq!(output["cache_hit"], true);
    assert_eq!(output["cached_at"], "2026-02-20T10:00:00Z");
    assert!(
        output.get("result").is_none(),
        "object payloads should not be wrapped in a result envelope"
    );
}

#[test]
fn auth_remediation_message_marks_discogs_auth_as_agent_actionable() {
    let remediation = crate::discogs::AuthRemediation {
        message: "Discogs auth required (not a lookup miss).".to_string(),
        auth_url: Some("https://discogs.example/auth/device".to_string()),
        poll_interval_seconds: Some(5),
        expires_at: Some(1_777_000_000),
    };

    let message = auth_remediation_message(&remediation);

    assert!(message.contains("not a lookup miss"));
    assert!(message.contains("Auth URL: https://discogs.example/auth/device"));
    assert!(message.contains("open 'https://discogs.example/auth/device'"));
    assert!(message.contains("Poll interval if polling instead of browser: 5s"));
    assert!(message.contains("Auth session expires_at (unix): 1777000000"));
}

#[tokio::test]
async fn analyze_track_audio_reports_essentia_unavailable_when_probe_is_none() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("cached-track.flac");
    std::fs::write(&audio_path, b"fake-audio-data").expect("temp audio file should be created");
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let metadata = std::fs::metadata(&audio_path).expect("temp audio metadata should load");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let db_conn = create_single_track_test_db("essentia-missing-1", &audio_path_str);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    store::set_audio_analysis(
        &store_conn,
        &audio_path_str,
        "stratum-dsp",
        file_size,
        file_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"stratum-dsp-1.0.0"}"#,
    )
    .expect("stratum cache should be seeded");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server
        .state
        .essentia_python
        .set(None)
        .expect("essentia probe state should be seeded once");

    let result = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: "essentia-missing-1".to_string(),
            skip_cached: Some(true),
        }))
        .await
        .expect("analyze_track_audio should succeed with cached stratum data");
    let payload = extract_json(&result);

    assert_eq!(payload["essentia_available"], false);
    assert!(
        payload["essentia"].is_null(),
        "essentia payload should be null when probe is unavailable"
    );
    assert_eq!(
        payload["stratum_cache_hit"], true,
        "stratum cache should still be used when Essentia is unavailable"
    );
    assert!(
        payload["stratum_dsp"].is_object(),
        "stratum_dsp should still be returned"
    );
    let hint = payload["essentia_setup_hint"]
        .as_str()
        .expect("essentia_setup_hint should be present when unavailable");
    assert!(
        hint.contains("setup_essentia"),
        "hint should mention setup_essentia tool"
    );
    assert!(
        hint.contains("CRATE_DIG_ESSENTIA_PYTHON"),
        "hint should mention the env var that was checked"
    );
}

#[test]
fn analyze_track_audio_cache_miss_when_file_unstatable() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let missing_path = audio_dir.path().join("missing-track.flac");
    let missing_path_str = missing_path.to_string_lossy().to_string();
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");

    store::set_audio_analysis(
        &store_conn,
        &missing_path_str,
        crate::audio::ANALYZER_STRATUM,
        123,
        456,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0}"#,
    )
    .expect("stale stratum cache should seed");

    let cached = check_analysis_cache(
        &store_conn,
        &missing_path_str,
        crate::audio::ANALYZER_STRATUM,
        crate::audio::STRATUM_SCHEMA_VERSION,
    )
    .expect("cache read should succeed");
    assert!(cached.is_none(), "unstatable files must be cache misses");
}

#[tokio::test]
#[ignore]
async fn analyze_track_audio_essentia_cache_round_trip_real_track() {
    let Some((server, _store_dir)) =
        create_real_server_with_temp_store(default_http_client_for_tests())
    else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    if server.essentia_python_path().is_none() {
        eprintln!("Skipping: Essentia Python not available");
        return;
    }

    let track = sample_real_tracks(&server, 40)
        .into_iter()
        .filter(|t| (120.0..=145.0).contains(&t.bpm))
        .find(|t| resolve_file_path(&t.file_path).is_ok())
        .expect("integration test needs at least one track with accessible audio file");

    let first = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: track.id.clone(),
            skip_cached: Some(false),
        }))
        .await
        .expect("initial analysis should succeed");
    let first_payload = extract_json(&first);
    assert_eq!(first_payload["essentia_available"], true);
    assert!(
        first_payload["essentia"].is_object(),
        "real Essentia run should produce feature JSON"
    );
    assert_eq!(first_payload["essentia_cache_hit"], false);
    let onset_rate = first_payload["essentia"]["onset_rate"]
        .as_f64()
        .expect("onset_rate should be present in Essentia output");
    let danceability = first_payload["essentia"]["danceability"]
        .as_f64()
        .expect("danceability should be present in Essentia output");
    let loudness_integrated = first_payload["essentia"]["loudness_integrated"]
        .as_f64()
        .expect("loudness_integrated should be present in Essentia output");
    assert!(
        onset_rate > 1.0,
        "onset_rate should be rate-like (Hz), got {onset_rate}"
    );
    assert!(
        (0.0..=3.5).contains(&danceability),
        "danceability should stay in plausible Essentia range [0, ~3], got {danceability}"
    );
    assert!(
        (-30.0..=0.0).contains(&loudness_integrated),
        "loudness_integrated should be in a plausible LUFS range, got {loudness_integrated}"
    );

    let second = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: track.id,
            skip_cached: Some(true),
        }))
        .await
        .expect("cached analysis should succeed");
    let second_payload = extract_json(&second);
    assert_eq!(second_payload["essentia_available"], true);
    assert_eq!(second_payload["stratum_cache_hit"], true);
    assert_eq!(second_payload["essentia_cache_hit"], true);
}

#[tokio::test]
#[cfg(unix)]
async fn setup_essentia_returns_already_installed_when_override_is_valid() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python");
    std::fs::write(&fake_python, "#!/bin/sh\necho '2.1b6.dev1389'\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python should be executable");
    let fake_path = fake_python.to_string_lossy().to_string();

    let server = ReklawdboxServer::new(None);
    {
        let mut guard = server.state.essentia_python_override.lock().unwrap();
        *guard = Some(fake_path.clone());
    }

    let result = server
        .setup_essentia()
        .await
        .expect("setup_essentia should succeed when already installed");
    let payload = extract_json(&result);

    assert_eq!(payload["status"], "already_installed");
    assert_eq!(payload["python_path"], fake_path.as_str());
}

#[tokio::test]
async fn essentia_python_override_takes_precedence() {
    let server = ReklawdboxServer::new(None);
    server
        .state
        .essentia_python
        .set(None)
        .expect("essentia probe should be seeded once");
    assert!(
        server.essentia_python_path().is_none(),
        "should be None before override"
    );

    {
        let mut guard = server.state.essentia_python_override.lock().unwrap();
        *guard = Some("/override/python".to_string());
    }
    assert_eq!(
        server.essentia_python_path().as_deref(),
        Some("/override/python"),
        "override should take precedence over OnceLock probe"
    );
}

#[tokio::test]
async fn write_xml_no_change_path_returns_message() {
    let server = ReklawdboxServer::new(None);

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("write_xml should succeed when no changes are staged");

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("message should be present"),
        "No changes to write."
    );
}

#[tokio::test]
async fn write_xml_no_change_path_via_router_returns_message() {
    let result = call_tool_via_router("write_xml", None).await;
    let payload = extract_json(&result);

    assert_eq!(
        payload
            .get("message")
            .and_then(serde_json::Value::as_str)
            .expect("message should be present"),
        "No changes to write."
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_with_playlists_exports_without_staged_changes() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
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

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("playlist-export.xml");
    let output_path_str = output_path.to_string_lossy().to_string();

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path_str.clone()),
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Set & Test".to_string(),
                track_ids: vec!["playlist-track-1".to_string()],
            }]),
        }))
        .await
        .expect("write_xml should export playlist-only requests");

    let payload = extract_json(&result);
    assert_eq!(payload["track_count"], 1);
    assert_eq!(payload["changes_applied"], 0);
    assert_eq!(payload["playlist_count"], 1);
    assert_eq!(
        payload["path"].as_str().expect("path should be present"),
        output_path_str
    );

    let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
    assert!(xml.contains("<PLAYLISTS>"));
    assert!(xml.contains("Name=\"Set &amp; Test\""));
    assert!(xml.contains("<TRACK Key=\"1\"/>"));
}

#[tokio::test]
async fn write_xml_with_playlists_reports_missing_track_ids() {
    let db_conn = create_single_track_test_db("playlist-track-1", "/tmp/playlist-track-1.flac");
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

    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Bad Set".to_string(),
                track_ids: vec!["does-not-exist".to_string()],
            }]),
        }))
        .await
        .expect_err("missing playlist track IDs should fail");

    let msg = format!("{err:?}");
    assert!(msg.contains("Track IDs not found in database"));
    assert!(msg.contains("does-not-exist"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_label_gate_blocks_when_set() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("gate-track-1", "/tmp/gate-track-1.flac");
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

    server.state.changes.stage(vec![TrackChange {
        track_id: "gate-track-1".to_string(),
        genre: None,
        comments: None,
        rating: None,
        color: None,
        label: Some("Test Label".to_string()),
        year: None,
        album: None,
    }]);

    server
        .state
        .label_research_gate
        .store(50, std::sync::atomic::Ordering::Relaxed);

    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: None,
            output_path: None,
            playlists: None,
        }))
        .await
        .expect_err("label gate should block write_xml");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("Label research gate"),
        "error should mention label research gate, got: {msg}"
    );
    assert!(
        msg.contains("50"),
        "error should mention the unlabeled count, got: {msg}"
    );

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("skip_label_gate=true should bypass the gate");

    let payload = extract_json(&result);
    assert!(payload.get("track_count").is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_label_gate_clears_when_zero() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("gate-clear-1", "/tmp/gate-clear-1.flac");
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

    server
        .state
        .label_research_gate
        .store(50, std::sync::atomic::Ordering::Relaxed);
    server
        .state
        .label_research_gate
        .store(0, std::sync::atomic::Ordering::Relaxed);

    server.state.changes.stage(vec![TrackChange {
        track_id: "gate-clear-1".to_string(),
        genre: None,
        comments: None,
        rating: None,
        color: None,
        label: Some("Test".to_string()),
        year: None,
        album: None,
    }]);

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: None,
            output_path: None,
            playlists: None,
        }))
        .await
        .expect("gate=0 should not block write_xml");

    let payload = extract_json(&result);
    assert!(payload.get("track_count").is_some());
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn write_xml_deduplicates_playlist_and_staged_tracks() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("staged-track-1", "/tmp/staged-track-1.flac");
    insert_test_track(
        &db_conn,
        "playlist-track-2",
        "Playlist Only",
        "g1",
        "/tmp/playlist-track-2.flac",
    );

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

    server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "staged-track-1".to_string(),
                genre: None,
                comments: Some("staged only comment".to_string()),
                rating: Some(5),
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("staging update should succeed");

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("mixed-export.xml");
    let output_path_str = output_path.to_string_lossy().to_string();

    let result = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path_str.clone()),
            playlists: Some(vec![WriteXmlPlaylistInput {
                name: "Mixed Export".to_string(),
                track_ids: vec!["playlist-track-2".to_string(), "staged-track-1".to_string()],
            }]),
        }))
        .await
        .expect("write_xml should succeed for mixed staged + playlist exports");

    let payload = extract_json(&result);
    assert_eq!(payload["track_count"], 2);
    assert_eq!(payload["changes_applied"], 1);
    assert_eq!(payload["playlist_count"], 1);
    assert_eq!(
        payload["path"].as_str().expect("path should be present"),
        output_path_str
    );

    let xml = std::fs::read_to_string(&output_path).expect("XML output should be readable");
    assert!(xml.contains("<COLLECTION Entries=\"2\">"));
    assert_eq!(xml.matches("TrackID=\"").count(), 2);
    assert_eq!(xml.matches("Name=\"Señorita\"").count(), 1);
    assert_eq!(xml.matches("Name=\"Playlist Only\"").count(), 1);

    let staged_line = xml
        .lines()
        .find(|line| line.contains("Name=\"Señorita\""))
        .expect("staged track line should exist");
    assert!(
        staged_line.contains("Comments=\"staged only comment\""),
        "staged comment should be applied to staged track"
    );
    assert!(
        staged_line.contains("Rating=\"255\""),
        "5-star staged rating should be encoded as 255"
    );

    let playlist_only_line = xml
        .lines()
        .find(|line| line.contains("Name=\"Playlist Only\""))
        .expect("playlist-only track line should exist");
    assert!(
        playlist_only_line.contains("Comments=\"cache coverage test\""),
        "playlist-only track should keep DB comments when no staged changes exist"
    );
    assert!(
        playlist_only_line.contains("Rating=\"102\""),
        "playlist-only track should keep DB-derived rating when not staged"
    );

    let playlist_line = xml
        .lines()
        .find(|line| {
            line.contains("<NODE")
                && line.contains("Type=\"1\"")
                && line.contains("Name=\"Mixed Export\"")
                && line.contains("Entries=\"2\"")
                && line.contains("KeyType=\"0\"")
        })
        .expect("playlist node should exist with expected attributes");
    let playlist_start = xml
        .find(playlist_line)
        .expect("playlist line should be findable in xml");
    let playlist_end = playlist_start
        + xml[playlist_start..]
            .find("</NODE>")
            .expect("playlist node should close");
    let playlist_block = &xml[playlist_start..playlist_end];
    let key2 = playlist_block
        .find("<TRACK Key=\"2\"/>")
        .expect("playlist should reference playlist-only track");
    let key1 = playlist_block
        .find("<TRACK Key=\"1\"/>")
        .expect("playlist should reference staged track");
    assert!(
        key2 < key1,
        "playlist key order should follow input track_ids order"
    );
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn write_xml_fails_closed_when_backup_script_fails_and_restores_changes() {
    use std::os::unix::fs::PermissionsExt;

    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let db_conn = create_single_track_test_db("staged-track-1", "/tmp/staged-track-1.flac");
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

    server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "staged-track-1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("staging update should succeed");

    let backup_dir = tempfile::tempdir().expect("temp backup dir should create");
    let backup_script = backup_dir.path().join("fail-backup.sh");
    std::fs::write(
        &backup_script,
        "#!/bin/sh\necho 'backup failed intentionally' >&2\nexit 23\n",
    )
    .expect("backup script should be written");
    let mut perms = std::fs::metadata(&backup_script)
        .expect("backup script metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&backup_script, perms).expect("backup script should be executable");

    let _backup_script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &backup_script);

    let output_dir = tempfile::tempdir().expect("temp output dir should create");
    let output_path = output_dir.path().join("should-not-exist.xml");
    let err = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(output_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect_err("write_xml should fail when backup script fails");

    let msg = format!("{err:?}");
    assert!(msg.contains("pre-op backup failed with exit status 23"));
    assert!(msg.contains("backup failed intentionally"));
    assert!(
        !output_path.exists(),
        "XML export should not be written after backup failure"
    );

    drop(_backup_script_env);

    let retry_path = output_dir.path().join("after-backup-failure.xml");
    let retry = server
        .write_xml(Parameters(WriteXmlParams {
            skip_label_gate: Some(true),
            output_path: Some(retry_path.to_string_lossy().to_string()),
            playlists: None,
        }))
        .await
        .expect("staged changes should be restored after backup failure");
    let payload = extract_json(&retry);
    assert_eq!(payload["changes_applied"], 1);

    let xml = std::fs::read_to_string(&retry_path).expect("retry XML output should be readable");
    assert!(
        xml.contains("Genre=\"Techno\""),
        "restored staged change should still be exported on retry"
    );
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_returns_custom_not_found_when_env_var_path_missing() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let _backup_env = EnvVarGuard::set(
        "REKLAWDBOX_BACKUP_SCRIPT",
        "/nonexistent/backup.sh".as_ref(),
    );

    let status = crate::backup::run_pre_op_backup()
        .await
        .expect("should return Ok, not Err");
    assert_eq!(status, crate::backup::BackupStatus::CustomNotFound);
}

#[tokio::test]
async fn update_tracks_stages_changes() {
    let server = ReklawdboxServer::new(None);
    let known_genre = genre::GENRES
        .first()
        .copied()
        .unwrap_or("House")
        .to_string();

    let result = server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "test-track-1".to_string(),
                genre: Some(known_genre),
                comments: Some("staged by test".to_string()),
                rating: Some(4),
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("update_tracks should succeed");

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    assert_eq!(
        payload
            .get("total_pending")
            .and_then(serde_json::Value::as_u64)
            .expect("total_pending should be present"),
        1
    );
    assert!(
        payload.get("changes").is_none(),
        "update_tracks should not echo changes back"
    );
}

#[tokio::test]
async fn update_tracks_via_router_warns_non_taxonomy_genre() {
    let result = call_tool_via_router(
        "update_tracks",
        serde_json::json!({
            "changes": [{
                "track_id": "router-test-track-1",
                "genre": "NotInTaxonomy"
            }]
        })
        .as_object()
        .cloned(),
    )
    .await;

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    let warnings = payload
        .get("warnings")
        .and_then(serde_json::Value::as_array)
        .expect("warnings should be present for non-taxonomy genre");
    assert!(
        !warnings.is_empty(),
        "warnings should include at least one non-taxonomy genre warning"
    );
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

#[test]
fn enrich_tracks_invalid_provider_rejected_by_serde() {
    let json = serde_json::json!({
        "providers": ["spotify"],
    });
    let result = serde_json::from_value::<EnrichTracksParams>(json);
    assert!(
        result.is_err(),
        "serde should reject unknown provider variant"
    );
}

#[tokio::test]
async fn lookup_discogs_no_match_payload_is_consistent_across_live_and_cache_paths() {
    let db_conn =
        create_single_track_test_db("discogs-no-match-track", "/tmp/discogs-no-match.flac");
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

    let artist = "Discogs NoMatch Artist";
    let title = "Discogs NoMatch Title";
    set_test_discogs_lookup_override(artist, title, None, Ok(None));

    let live_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(true),
        }))
        .await
        .expect("live discogs no-match should succeed");
    let live_payload = extract_json(&live_result);
    assert_eq!(live_payload["result"], serde_json::Value::Null);
    assert_eq!(live_payload["cache_hit"], false);
    assert!(
        live_payload.get("cached_at").is_none(),
        "live payload should omit cached_at"
    );

    let cache_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(false),
        }))
        .await
        .expect("cached discogs no-match should succeed");
    let cache_payload = extract_json(&cache_result);
    assert_eq!(cache_payload["result"], serde_json::Value::Null);
    assert_eq!(cache_payload["cache_hit"], true);

    let cache_hit_timestamp = cache_payload
        .get("cached_at")
        .and_then(serde_json::Value::as_str)
        .expect("cached no-match payload should include cached_at");
    let norm_artist = crate::normalize::normalize_for_matching(artist);
    let norm_title = crate::normalize::normalize_for_matching(title);
    let cache_entry = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "discogs", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("discogs no-match lookup should create cache entry")
    };
    assert!(
        cache_entry.response_json.is_none(),
        "discogs no-match cache entry should store null response as no payload"
    );
    assert_eq!(
        cache_hit_timestamp,
        cache_entry.created_at.as_str(),
        "cached_at should match persisted cache timestamp"
    );
}

#[tokio::test]
async fn lookup_beatport_no_match_payload_is_consistent_across_live_and_cache_paths() {
    let db_conn =
        create_single_track_test_db("beatport-no-match-track", "/tmp/beatport-no-match.flac");
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

    let artist = "Beatport NoMatch Artist";
    let title = "Beatport NoMatch Title";
    set_test_beatport_lookup_override(artist, title, Ok(None));

    let live_result = server
        .lookup_beatport(Parameters(LookupBeatportParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            force_refresh: Some(true),
        }))
        .await
        .expect("live beatport no-match should succeed");
    let live_payload = extract_json(&live_result);
    assert_eq!(live_payload["result"], serde_json::Value::Null);
    assert_eq!(live_payload["cache_hit"], false);
    assert!(
        live_payload.get("cached_at").is_none(),
        "live payload should omit cached_at"
    );

    let cache_result = server
        .lookup_beatport(Parameters(LookupBeatportParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            force_refresh: Some(false),
        }))
        .await
        .expect("cached beatport no-match should succeed");
    let cache_payload = extract_json(&cache_result);
    assert_eq!(cache_payload["result"], serde_json::Value::Null);
    assert_eq!(cache_payload["cache_hit"], true);

    let cache_hit_timestamp = cache_payload
        .get("cached_at")
        .and_then(serde_json::Value::as_str)
        .expect("cached no-match payload should include cached_at");
    let norm_artist = crate::normalize::normalize_for_matching(artist);
    let norm_title = crate::normalize::normalize_for_matching(title);
    let cache_entry = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "beatport", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("beatport no-match lookup should create cache entry")
    };
    assert!(
        cache_entry.response_json.is_none(),
        "beatport no-match cache entry should store null response as no payload"
    );
    assert_eq!(
        cache_hit_timestamp,
        cache_entry.created_at.as_str(),
        "cached_at should match persisted cache timestamp"
    );
}

#[tokio::test]
async fn enrich_tracks_discogs_skip_cached_reports_cached_counts() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                    12700, 153, 'cached batch test', 2025, 230, ?3, '0', 1411,
                    44100, 5, '2025-01-01', 0
                )",
            params![
                "cached-track-2",
                "Corazón Cached",
                "/tmp/cached-track-2.flac"
            ],
        )
        .expect("second test track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let artist = "Aníbal";
    let title_one = "Señorita";
    let title_two = "Corazón Cached";
    let norm_artist = crate::normalize::normalize_for_matching(artist);
    let norm_title_one = crate::normalize::normalize_for_matching(title_one);
    let norm_title_two = crate::normalize::normalize_for_matching(title_two);
    let norm_album = crate::normalize::normalize_for_matching("Encoded Paths");

    let cached_one = serde_json::json!({
        "title": "Anibal - Senorita",
        "genres": ["Electronic"],
        "styles": ["Deep House"],
        "fuzzy_match": false
    })
    .to_string();
    let cached_two = serde_json::json!({
        "title": "Anibal - Corazon Cached",
        "genres": ["Electronic"],
        "styles": ["House"],
        "fuzzy_match": false
    })
    .to_string();

    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_one),
    )
    .expect("first sentinel discogs cache entry should write");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_two),
    )
    .expect("second sentinel discogs cache entry should write");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );

    let params = EnrichTracksParams {
        filters: SearchFilterParams::default(),
        track_ids: Some(vec![
            "cached-track-1".to_string(),
            "cached-track-2".to_string(),
        ]),
        playlist_id: None,
        max_tracks: Some(10),
        offset: None,
        providers: Some(vec![crate::types::Provider::Discogs]),
        skip_cached: Some(true),
        force_refresh: Some(false),
        concurrency: None,
    };

    let first_result = server
        .enrich_tracks(Parameters(params))
        .await
        .expect("enrich_tracks should succeed when everything is cached");
    let first_payload = extract_json(&first_result);
    assert_eq!(first_payload["summary"]["total"], 2);
    assert_eq!(first_payload["summary"]["enriched"], 0);
    assert_eq!(first_payload["summary"]["cached"], 2);
    assert_eq!(first_payload["summary"]["skipped"], 0);
    assert_eq!(first_payload["summary"]["failed"], 0);
    assert_eq!(
        first_payload["failures"]
            .as_array()
            .expect("failures should be an array")
            .len(),
        0
    );

    let second_result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "cached-track-1".to_string(),
                "cached-track-2".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(10),
            offset: None,
            providers: Some(vec![crate::types::Provider::Discogs]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("second enrich_tracks run should also be fully cached");
    let second_payload = extract_json(&second_result);
    assert_eq!(second_payload["summary"]["total"], 2);
    assert_eq!(second_payload["summary"]["enriched"], 0);
    assert_eq!(second_payload["summary"]["cached"], 2);
    assert_eq!(second_payload["summary"]["skipped"], 0);
    assert_eq!(second_payload["summary"]["failed"], 0);

    let store = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry_one = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("first cache entry should still exist");
    let entry_two = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("second cache entry should still exist");
    assert_eq!(
        entry_one.response_json.as_deref(),
        Some(cached_one.as_str())
    );
    assert_eq!(
        entry_two.response_json.as_deref(),
        Some(cached_two.as_str())
    );
}

#[tokio::test]
async fn enrich_tracks_summary_uses_provider_attempt_totals() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let norm_artist = crate::normalize::normalize_for_matching("Aníbal");
    let norm_title = crate::normalize::normalize_for_matching("Señorita");
    let norm_album = crate::normalize::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .expect("discogs cache should seed");
    store::set_enrichment(
        &store_conn,
        "beatport",
        &norm_artist,
        &norm_title,
        None,
        Some("exact"),
        Some(r#"{"genre":"Deep House"}"#),
    )
    .expect("beatport cache should seed");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );
    let result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["cached-track-1".to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            offset: None,
            providers: Some(vec![
                crate::types::Provider::Discogs,
                crate::types::Provider::Beatport,
            ]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("enrich_tracks should resolve from cache for both providers");
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["tracks_total"], 1);
    assert_eq!(payload["summary"]["total"], 2);
    assert_eq!(payload["summary"]["cached"], 2);
    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 0);
}

#[tokio::test]
async fn resolve_track_data_uses_decoded_path_for_audio_cache_lookup() {
    let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let decoded_path = temp_audio_dir.path().join("Aníbal Track.flac");
    std::fs::write(&decoded_path, b"fake-audio-data")
        .expect("decoded path file should exist for resolve_file_path");
    let metadata = std::fs::metadata(&decoded_path).expect("decoded path metadata should load");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let decoded_path_str = decoded_path.to_string_lossy().to_string();
    let raw_path = decoded_path_str
        .replace("Aníbal", "An%C3%ADbal")
        .replace(' ', "%20");
    assert_ne!(
        raw_path, decoded_path_str,
        "raw path must differ from decoded path for this regression test"
    );
    assert!(
        std::fs::metadata(&decoded_path_str).is_ok(),
        "decoded file path should exist"
    );
    assert!(
        std::fs::metadata(&raw_path).is_err(),
        "raw encoded path should not exist"
    );

    let db_conn = create_single_track_test_db("encoded-track-1", &raw_path);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    store::set_audio_analysis(
        &store_conn,
        &decoded_path_str,
        "stratum-dsp",
        file_size,
        file_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"4"}"#,
    )
    .expect("audio cache entry should write with decoded cache key");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: "encoded-track-1".to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let payload = extract_json(&result);

    assert_eq!(
        payload["data_completeness"]["stratum_dsp"], true,
        "decoded path lookup should find stratum cache entry"
    );
    assert!(
        payload["audio_analysis"]["stratum_dsp"].is_object(),
        "audio_analysis.stratum_dsp should be populated from cache"
    );
    assert_eq!(payload["audio_analysis"]["stratum_dsp"]["key"], "Am");
}

#[tokio::test]
async fn cache_coverage_reports_provider_coverage_and_gap_counts() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let with_genre_path = audio_dir.path().join("coverage-1.flac");
    let fresh_path = audio_dir.path().join("coverage-2.flac");
    let stale_schema_path = audio_dir.path().join("coverage-3.flac");
    let stale_identity_path = audio_dir.path().join("coverage-4.flac");
    write_test_audio_file(&with_genre_path, 1000);
    let (fresh_size, fresh_mtime) = write_test_audio_file(&fresh_path, 1001);
    let (stale_schema_size, stale_schema_mtime) = write_test_audio_file(&stale_schema_path, 1002);
    let (stale_identity_size, stale_identity_mtime) =
        write_test_audio_file(&stale_identity_path, 1003);
    let with_genre_path_str = with_genre_path.to_string_lossy().to_string();
    let fresh_path_str = fresh_path.to_string_lossy().to_string();
    let stale_schema_path_str = stale_schema_path.to_string_lossy().to_string();
    let stale_identity_path_str = stale_identity_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("coverage-with-genre", &with_genre_path_str);
    insert_test_track(
        &db_conn,
        "coverage-no-genre-1",
        "No Genre One",
        "",
        &fresh_path_str,
    );
    insert_test_track(
        &db_conn,
        "coverage-no-genre-2",
        "No Genre Two",
        "",
        &stale_schema_path_str,
    );
    insert_test_track(
        &db_conn,
        "coverage-no-genre-3",
        "No Genre Three",
        "",
        &stale_identity_path_str,
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let norm_artist = crate::normalize::normalize_for_matching("Aníbal");
    let norm_title_one = crate::normalize::normalize_for_matching("No Genre One");
    let norm_title_two = crate::normalize::normalize_for_matching("No Genre Two");

    store::set_audio_analysis(
        &store_conn,
        &fresh_path_str,
        "stratum-dsp",
        fresh_size,
        fresh_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":127.1,"key":"Am"}"#,
    )
    .expect("stratum cache should be seeded");
    store::set_audio_analysis(
        &store_conn,
        &fresh_path_str,
        "essentia",
        fresh_size,
        fresh_mtime,
        crate::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":0.81}"#,
    )
    .expect("essentia cache should be seeded");
    store::set_audio_analysis(
        &store_conn,
        &stale_schema_path_str,
        "stratum-dsp",
        stale_schema_size,
        stale_schema_mtime,
        "outdated",
        r#"{"bpm":128.0,"key":"Am"}"#,
    )
    .expect("stale-schema stratum cache should be seeded");
    store::set_audio_analysis(
        &store_conn,
        &stale_schema_path_str,
        "essentia",
        stale_schema_size,
        stale_schema_mtime,
        "outdated",
        r#"{"danceability":0.70}"#,
    )
    .expect("stale-schema essentia cache should be seeded");
    store::set_audio_analysis(
        &store_conn,
        &stale_identity_path_str,
        "stratum-dsp",
        stale_identity_size + 1,
        stale_identity_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":129.0,"key":"Am"}"#,
    )
    .expect("stale-size stratum cache should be seeded");
    store::set_audio_analysis(
        &store_conn,
        &stale_identity_path_str,
        "essentia",
        stale_identity_size,
        stale_identity_mtime + 1,
        crate::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":0.75}"#,
    )
    .expect("stale-mtime essentia cache should be seeded");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some("encoded paths"),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .expect("discogs cache should be seeded for first ungenred track");
    store::set_enrichment(
        &store_conn,
        "beatport",
        &norm_artist,
        &norm_title_one,
        None,
        Some("exact"),
        Some(r#"{"genre":"Deep House"}"#),
    )
    .expect("beatport cache should be seeded for first ungenred track");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some("encoded paths"),
        Some("exact"),
        Some(r#"{"styles":["Tech House"]}"#),
    )
    .expect("discogs cache should be seeded for second ungenred track");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server
        .state
        .essentia_python
        .set(Some("/tmp/fake-essentia-python".to_string()))
        .expect("essentia probe cache should be set exactly once");

    let result = server
        .cache_coverage(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams {
                has_genre: Some(false),
                ..Default::default()
            },
            track_ids: None,
            playlist_id: None,
            max_tracks: None,
            format: None,
        }))
        .await
        .expect("cache_coverage should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["scope"]["total_tracks"], 4);
    assert_eq!(payload["scope"]["matched_tracks"], 3);
    assert_eq!(payload["scope"]["filter_description"], "has_genre = false");

    assert_eq!(payload["coverage"]["stratum_dsp"]["cached"], 1);
    assert_eq!(payload["coverage"]["stratum_dsp"]["percent"], 33.3);

    assert_eq!(payload["coverage"]["essentia"]["cached"], 1);
    assert_eq!(payload["coverage"]["essentia"]["percent"], 33.3);
    assert_eq!(payload["coverage"]["essentia"]["installed"], true);

    assert_eq!(payload["coverage"]["discogs"]["searched"], 2);
    assert_eq!(payload["coverage"]["discogs"]["searched_percent"], 66.7);
    assert_eq!(payload["coverage"]["discogs"]["has_result"], 2);
    assert_eq!(payload["coverage"]["discogs"]["has_result_percent"], 66.7);

    assert_eq!(payload["coverage"]["beatport"]["searched"], 1);
    assert_eq!(payload["coverage"]["beatport"]["searched_percent"], 33.3);
    assert_eq!(payload["coverage"]["beatport"]["has_result"], 1);
    assert_eq!(payload["coverage"]["beatport"]["has_result_percent"], 33.3);

    assert_eq!(payload["gaps"]["no_audio_analysis"], 2);
    assert_eq!(payload["gaps"]["no_enrichment"], 1);
    assert_eq!(payload["gaps"]["no_data_at_all"], 1);
}

#[tokio::test]
async fn cache_coverage_excludes_sampler_tracks_for_id_and_playlist_scopes() {
    let db_conn = create_single_track_test_db("coverage-base", "/music/coverage-base.flac");
    insert_test_track(
        &db_conn,
        "coverage-nonsample",
        "Coverage Non Sample",
        "",
        "/music/coverage-nonsample.flac",
    );
    let sampler_path = format!("/music{}CoverageSampler.wav", db::SAMPLER_PATH_FRAGMENT);
    insert_test_track(
        &db_conn,
        "coverage-sampler",
        "Coverage Sampler",
        "",
        &sampler_path,
    );

    db_conn
        .execute_batch(
            "CREATE TABLE djmdSongPlaylist (
                    PlaylistID VARCHAR(255),
                    ContentID VARCHAR(255),
                    TrackNo INTEGER
                );",
        )
        .expect("playlist table should be created for test");
    db_conn
        .execute(
            "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
            params!["pl-cache", "coverage-nonsample", 1],
        )
        .expect("non-sampler playlist entry should insert");
    db_conn
        .execute(
            "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
            params!["pl-cache", "coverage-sampler", 2],
        )
        .expect("sampler playlist entry should insert");

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

    let id_scope = server
        .cache_coverage(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "coverage-nonsample".to_string(),
                "coverage-sampler".to_string(),
            ]),
            playlist_id: None,
            max_tracks: None,
            format: None,
        }))
        .await
        .expect("cache_coverage track_ids scope should succeed");
    let id_payload = extract_json(&id_scope);
    assert!(id_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
    assert_eq!(id_payload["scope"]["matched_tracks"], 1);
    assert_eq!(id_payload["gaps"]["no_data_at_all"], 1);

    let playlist_scope = server
        .cache_coverage(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: None,
            playlist_id: Some("pl-cache".to_string()),
            max_tracks: None,
            format: None,
        }))
        .await
        .expect("cache_coverage playlist scope should succeed");
    let playlist_payload = extract_json(&playlist_scope);
    assert!(playlist_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
    assert_eq!(playlist_payload["scope"]["matched_tracks"], 1);
    assert_eq!(playlist_payload["gaps"]["no_data_at_all"], 1);
}

#[tokio::test]
async fn calibration_coverage_reports_verified_playlist_readiness() {
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
        store::set_audio_analysis(
            &store_conn,
            path,
            crate::audio::ANALYZER_STRATUM,
            *file_size,
            *file_mtime,
            crate::audio::STRATUM_SCHEMA_VERSION,
            r#"{"bpm":127.0,"decay_mid_tau":0.21,"key_clarity":0.72}"#,
        )
        .expect("stratum analysis should be seeded");
    }
    store::set_audio_analysis(
        &store_conn,
        &tech_path_str,
        crate::audio::ANALYZER_STRATUM,
        tech_size,
        tech_mtime,
        "stale-stratum-schema",
        r#"{"bpm":132.0,"decay_mid_tau":240.0,"key_clarity":0.40}"#,
    )
    .expect("stale stratum analysis should be seeded");

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
    assert_eq!(payload["tracks_with_stratum_features"], 5);
    assert_eq!(payload["missing_stratum_features"], 1);
    assert_eq!(payload["tracks_with_essentia_features"], 0);
    assert_eq!(payload["missing_essentia_features"], 6);
    assert_eq!(payload["skipped_no_genre"], 1);
    assert_eq!(payload["skipped_unknown_genre"], 1);
    assert_eq!(
        payload["min_tracks_per_genre"],
        crate::audio_profile::MIN_TRACKS
    );
    assert_eq!(payload["genres_ready_to_calibrate"], 1);
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
    assert_eq!(deep_house["tracks_with_stratum_features"], 5);
    assert_eq!(deep_house["tracks_with_essentia_features"], 0);
    assert_eq!(deep_house["prototype_ready"], true);
    assert_eq!(deep_house["status"], "ready_to_calibrate");

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
}

#[tokio::test]
async fn calibration_coverage_reads_verified_playlist_without_ordinary_limit() {
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

#[tokio::test]
#[ignore]
async fn force_refresh_bypasses_enrichment_cache() {
    let offline_http = reqwest::Client::builder()
        .user_agent("Reklawdbox/0.1")
        .proxy(reqwest::Proxy::all("http://127.0.0.1:9").expect("offline proxy URL should parse"))
        .build()
        .expect("offline HTTP client should build");

    let Some((server, _store_dir)) = create_real_server_with_temp_store(offline_http) else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let track = sample_real_tracks(&server, 1)
        .into_iter()
        .next()
        .expect("integration test needs at least one real track");
    let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
    let norm_title = crate::normalize::normalize_for_matching(&track.title);
    let cached_json = serde_json::json!({"genre":"Sentinel Genre","key":"Am","bpm":128});
    let cached_json_str = cached_json.to_string();

    {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::set_enrichment(
            &store,
            "beatport",
            &norm_artist,
            &norm_title,
            None,
            Some("exact"),
            Some(&cached_json_str),
        )
        .expect("sentinel cache entry should write");
    }

    let cache_hit = server
        .lookup_beatport(Parameters(LookupBeatportParams {
            track_id: None,
            artist: Some(track.artist.clone()),
            title: Some(track.title.clone()),
            force_refresh: Some(false),
        }))
        .await
        .expect("lookup_beatport(force_refresh=false) should return cache");
    let cache_hit_json = extract_json(&cache_hit);
    assert_eq!(cache_hit_json["cache_hit"], true);
    assert_eq!(cache_hit_json["genre"], "Sentinel Genre");

    let refresh_err = server
        .lookup_beatport(Parameters(LookupBeatportParams {
            track_id: None,
            artist: Some(track.artist.clone()),
            title: Some(track.title.clone()),
            force_refresh: Some(true),
        }))
        .await
        .expect_err("force_refresh=true should bypass cache and attempt HTTP call");
    assert!(
        format!("{refresh_err}").contains("Beatport error"),
        "force refresh should fail via offline HTTP path, got: {refresh_err}"
    );
}

#[tokio::test]
#[ignore]
async fn enrich_tracks_beatport_schema_matches_individual_lookup() {
    let Some((server, _store_dir)) =
        create_real_server_with_temp_store(default_http_client_for_tests())
    else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let candidates = sample_real_tracks(&server, 30);
    if candidates.is_empty() {
        eprintln!("Skipping: integration test needs candidate tracks from real DB");
        return;
    }

    let mut selected_track: Option<crate::types::Track> = None;
    for track in candidates.into_iter().take(10) {
        let lookup = server
            .lookup_beatport(Parameters(LookupBeatportParams {
                track_id: None,
                artist: Some(track.artist.clone()),
                title: Some(track.title.clone()),
                force_refresh: Some(true),
            }))
            .await;

        let Ok(result) = lookup else {
            continue;
        };
        let payload = extract_json(&result);
        if payload
            .get("genre")
            .and_then(serde_json::Value::as_str)
            .is_some()
        {
            selected_track = Some(track);
            break;
        }
    }

    let Some(track) = selected_track else {
        eprintln!(
            "Skipping: could not find a track with a successful Beatport match; \
                 rerun when network/providers are available"
        );
        return;
    };
    let norm_artist = crate::normalize::normalize_for_matching(&track.artist);
    let norm_title = crate::normalize::normalize_for_matching(&track.title);

    let individual_cache = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "beatport", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("individual lookup should have created cache entry")
    };
    let individual_json: serde_json::Value = serde_json::from_str(
        individual_cache
            .response_json
            .as_deref()
            .expect("individual beatport cache should contain JSON"),
    )
    .expect("individual beatport cache JSON should parse");
    assert!(
        individual_json
            .get("genre")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "individual beatport cache should have string 'genre'"
    );
    assert!(
        individual_json.get("label").is_some(),
        "individual beatport cache should include 'label' field"
    );
    let individual_fields: HashSet<String> = individual_json
        .as_object()
        .expect("individual beatport cache should be object")
        .keys()
        .cloned()
        .collect();

    {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store
            .execute(
                "DELETE FROM enrichment_cache
                     WHERE provider = ?1 AND query_artist = ?2 AND query_title = ?3",
                params!["beatport", &norm_artist, &norm_title],
            )
            .expect("cache clear should succeed");
    }

    let enrich_result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![track.id.clone()]),
            playlist_id: None,
            max_tracks: Some(1),
            offset: None,
            providers: Some(vec![crate::types::Provider::Beatport]),
            skip_cached: Some(false),
            force_refresh: Some(true),
            concurrency: None,
        }))
        .await
        .expect("enrich_tracks should succeed for beatport provider");
    let enrich_payload = extract_json(&enrich_result);
    assert_eq!(enrich_payload["summary"]["total"], 1);

    let batch_cache = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "beatport", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("batch enrich should have created beatport cache entry")
    };
    let batch_json: serde_json::Value = serde_json::from_str(
        batch_cache
            .response_json
            .as_deref()
            .expect("batch beatport cache should contain JSON"),
    )
    .expect("batch beatport cache JSON should parse");
    assert!(
        batch_json
            .get("genre")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "batch beatport cache should have string 'genre'"
    );
    assert!(
        batch_json.get("genres").is_none(),
        "beatport cache should not be transformed into discogs-style 'genres' schema"
    );

    let batch_fields: HashSet<String> = batch_json
        .as_object()
        .expect("batch beatport cache should be object")
        .keys()
        .cloned()
        .collect();
    assert_eq!(
        batch_fields, individual_fields,
        "batch and individual beatport cache JSON should share the same schema"
    );
}

#[tokio::test]
#[ignore]
async fn resolve_tracks_data_batch_consistency() {
    let Some((server, _store_dir)) =
        create_real_server_with_temp_store(default_http_client_for_tests())
    else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let tracks = sample_real_tracks(&server, 5);
    assert!(
        !tracks.is_empty(),
        "integration test needs tracks from real DB"
    );
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();

    let batch_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(track_ids.clone()),
            playlist_id: None,
            max_tracks: Some(track_ids.len() as u32),
            format: None,
        }))
        .await
        .expect("batch resolve should succeed");
    let batch_payload = extract_json(&batch_result);
    let batch_items = batch_payload
        .as_array()
        .expect("batch resolve payload should be an array");
    assert_eq!(
        batch_items.len(),
        track_ids.len(),
        "batch resolve should return one entry per requested track"
    );

    let mut by_track_id: HashMap<String, serde_json::Value> = HashMap::new();
    for item in batch_items {
        let track_id = item
            .get("track_id")
            .and_then(serde_json::Value::as_str)
            .expect("resolved track item should include track_id");
        by_track_id.insert(track_id.to_string(), item.clone());
    }

    for track_id in &track_ids {
        let single_result = server
            .resolve_track_data(Parameters(ResolveTrackDataParams {
                track_id: track_id.clone(),
            }))
            .await
            .expect("single resolve should succeed");
        let single_payload = extract_json(&single_result);
        assert_eq!(
            by_track_id
                .get(track_id)
                .expect("batch output should include every requested track"),
            &single_payload,
            "batch resolve output should match single-track resolve output"
        );
    }
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
fn golden_dataset_genre_accuracy() {
    let entries = load_golden_genres_fixture();
    let Some(conn) = db::open_real_db() else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let mut compared = 0usize;
    let mut correct = 0usize;
    let mut missing_tracks = Vec::new();
    let mut no_genre = Vec::new();
    let mut mismatches = Vec::new();

    for entry in &entries {
        let Some(track) = find_track_by_artist_and_title(&conn, &entry.artist, &entry.title) else {
            missing_tracks.push(format!("{} - {}", entry.artist, entry.title));
            continue;
        };

        if track.genre.trim().is_empty() {
            no_genre.push(format!("{} - {}", entry.artist, entry.title));
            continue;
        }

        compared += 1;
        let actual = canonical_genre_name(&track.genre);
        if actual.eq_ignore_ascii_case(&entry.expected_genre) {
            correct += 1;
        } else {
            mismatches.push(format!(
                "{} - {}: expected '{}', actual '{}' ({})",
                entry.artist, entry.title, entry.expected_genre, actual, entry.notes
            ));
        }
    }

    let accuracy = if compared == 0 {
        0.0
    } else {
        (correct as f64 / compared as f64) * 100.0
    };
    eprintln!(
        "[integration] golden dataset: total={} compared={} correct={} accuracy={:.1}%",
        entries.len(),
        compared,
        correct,
        accuracy
    );
    if !missing_tracks.is_empty() {
        eprintln!("[integration] missing tracks ({}):", missing_tracks.len());
        for item in &missing_tracks {
            eprintln!("  - {item}");
        }
    }
    if !no_genre.is_empty() {
        eprintln!(
            "[integration] tracks with empty genre ({}):",
            no_genre.len()
        );
        for item in &no_genre {
            eprintln!("  - {item}");
        }
    }
    if !mismatches.is_empty() {
        eprintln!("[integration] mismatches ({}):", mismatches.len());
        for item in &mismatches {
            eprintln!("  - {item}");
        }
    }

    assert!(
        !missing_tracks.is_empty() || compared > 0,
        "fixture should either report missing tracks or compare at least one track"
    );
}

fn make_test_track(id: &str, genre: &str, bpm: f64, key: &str) -> crate::types::Track {
    crate::types::Track {
        id: id.to_string(),
        title: format!("Track {id}"),
        artist: "Test Artist".to_string(),
        album: "Test Album".to_string(),
        genre: genre.to_string(),
        bpm,
        key: key.to_string(),
        rating: 3,
        comments: "test comment".to_string(),
        color: "Rose".to_string(),
        color_code: 1,
        label: "Test Label".to_string(),
        remixer: "".to_string(),
        year: 2023,
        length: 300,
        file_path: "/music/test.flac".to_string(),
        play_count: 5,
        bit_rate: 1411,
        sample_rate: 44100,
        file_kind: crate::types::FileKind::Flac,
        date_added: "2023-01-15".to_string(),
        position: None,
        played_at: None,
    }
}

#[test]
fn resolve_single_track_rekordbox_only() {
    let track = make_test_track("t1", "Deep House", 126.0, "Am");
    let result = resolve_single_track(&track, None, None, None, None, false, None);

    let rb = result
        .get("rekordbox")
        .expect("rekordbox section should exist");
    assert_eq!(rb["title"], "Track t1");
    assert_eq!(rb["artist"], "Test Artist");
    assert_eq!(rb["genre"], "Deep House");
    assert_eq!(rb["bpm"], 126.0);
    assert_eq!(rb["key"], "Am");
    assert_eq!(rb["duration_s"], 300);
    assert_eq!(rb["year"], 2023);
    assert_eq!(rb["rating"], 3);
    assert_eq!(rb["label"], "Test Label");

    assert!(
        result["audio_analysis"].is_null(),
        "audio_analysis should be null without cache"
    );
    assert!(
        result["discogs"].is_null(),
        "discogs should be null without cache"
    );
    assert!(
        result["beatport"].is_null(),
        "beatport should be null without cache"
    );
    assert!(
        result["staged_changes"].is_null(),
        "staged_changes should be null without staged"
    );

    let dc = result
        .get("data_completeness")
        .expect("data_completeness should exist");
    assert_eq!(dc["rekordbox"], true);
    assert_eq!(dc["stratum_dsp"], false);
    assert_eq!(dc["essentia"], false);
    assert_eq!(dc["essentia_installed"], false);
    assert_eq!(dc["discogs"], false);
    assert_eq!(dc["beatport"], false);

    let gt = result
        .get("genre_taxonomy")
        .expect("genre_taxonomy should exist");
    assert_eq!(gt["current_genre_canonical"], "Deep House");
}

#[test]
fn resolve_single_track_with_staged_changes() {
    let track = make_test_track("t2", "House", 128.0, "Cm");
    let staged = crate::types::TrackChange {
        track_id: "t2".to_string(),
        genre: Some("Deep House".to_string()),
        comments: None,
        rating: Some(5),
        color: None,
        label: None,
        year: None,
        album: None,
    };
    let result = resolve_single_track(&track, None, None, None, None, false, Some(&staged));

    let sc = result
        .get("staged_changes")
        .expect("staged_changes should exist");
    assert!(
        !sc.is_null(),
        "staged_changes should not be null when changes are staged"
    );
    assert_eq!(sc["genre"], "Deep House");
    assert!(sc["comments"].is_null(), "unstaged field should be null");
    assert_eq!(sc["rating"], 5);
    assert!(sc["color"].is_null(), "unstaged field should be null");
}

#[test]
fn resolve_single_track_taxonomy_mappings() {
    let track = make_test_track("t3", "Hip-Hop", 130.0, "Fm");

    let discogs_json = serde_json::json!({
        "title": "Some Release",
        "year": "2020",
        "label": "Some Label",
        "genres": ["Electronic"],
        "styles": ["Deep House", "Garage House", "Some Unknown Style"],
        "fuzzy_match": false,
    });
    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t3".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("exact".to_string()),
        response_json: Some(serde_json::to_string(&discogs_json).unwrap()),
        created_at: "2024-01-01".to_string(),
    };

    let beatport_json = serde_json::json!({
        "genre": "Techno",
        "bpm": 130,
        "key": "Fm",
        "track_name": "Track t3",
        "artists": ["Test Artist"],
    });
    let beatport_cache = store::EnrichmentCacheEntry {
        provider: "beatport".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t3".to_string(),
        query_album: String::new(),
        match_quality: Some("exact".to_string()),
        response_json: Some(serde_json::to_string(&beatport_json).unwrap()),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(
        &track,
        Some(&discogs_cache),
        Some(&beatport_cache),
        None,
        None,
        false,
        None,
    );

    let dc = &result["data_completeness"];
    assert_eq!(dc["discogs"], true);
    assert_eq!(dc["beatport"], true);
    assert_eq!(dc["stratum_dsp"], false);

    let gt = &result["genre_taxonomy"];
    assert_eq!(gt["current_genre_canonical"], "Hip Hop");

    let dsm = gt["discogs_style_mappings"]
        .as_array()
        .expect("should be array");
    assert_eq!(dsm.len(), 3);

    let dh = dsm
        .iter()
        .find(|m| m["style"] == "Deep House")
        .expect("Deep House mapping");
    assert_eq!(dh["mapping_type"], "exact");
    assert_eq!(dh["maps_to"], "Deep House");

    let gh = dsm
        .iter()
        .find(|m| m["style"] == "Garage House")
        .expect("Garage House mapping");
    assert_eq!(gh["mapping_type"], "alias");
    assert_eq!(gh["maps_to"], "House");

    let unknown = dsm
        .iter()
        .find(|m| m["style"] == "Some Unknown Style")
        .expect("unknown mapping");
    assert_eq!(unknown["mapping_type"], "unknown");
    assert!(unknown["maps_to"].is_null());

    let bgm = &gt["beatport_genre_mapping"];
    assert_eq!(bgm["genre"], "Techno");
    assert_eq!(bgm["mapping_type"], "exact");
    assert_eq!(bgm["maps_to"], "Techno");

    assert!(
        result["discogs"].is_object(),
        "discogs should be parsed object"
    );
    assert!(
        result["beatport"].is_object(),
        "beatport should be parsed object"
    );
}

#[test]
fn resolve_single_track_empty_genre_is_null() {
    let track = make_test_track("t4", "", 0.0, "");
    let result = resolve_single_track(&track, None, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "empty genre should map to null canonical"
    );
}

#[test]
fn resolve_single_track_unknown_genre_maps_to_null() {
    let track = make_test_track("t5", "Polka", 120.0, "C");
    let result = resolve_single_track(&track, None, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "unknown genre 'Polka' should map to null"
    );
}

#[test]
fn resolve_single_track_with_stratum_agreement() {
    let track = make_test_track("t6", "Techno", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 128.5,
        "key": "Am",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, None, Some(&stratum_cache), None, false, None);

    let aa = result
        .get("audio_analysis")
        .expect("audio_analysis should exist");
    assert!(
        !aa.is_null(),
        "audio_analysis should not be null with stratum cache"
    );
    assert_eq!(
        aa["bpm_agreement"], true,
        "BPM 128.0 vs 128.5 should agree (within 2.0)"
    );
    assert_eq!(aa["key_agreement"], true, "Key Am vs Am should agree");
    assert!(
        aa["stratum_dsp"].is_object(),
        "stratum_dsp should be the parsed features"
    );
    assert!(
        aa["essentia"].is_null(),
        "essentia should be null when not cached"
    );

    let dc = &result["data_completeness"];
    assert_eq!(dc["stratum_dsp"], true);
}

#[test]
fn resolve_single_track_with_essentia_cache() {
    let track = make_test_track("t6b", "Techno", 128.0, "Am");
    let essentia_json = serde_json::json!({
        "danceability": 0.82,
        "loudness_integrated": -8.4,
        "rhythm_regularity": 0.91,
        "analyzer_version": "2.1b6.dev1389"
    });
    let essentia_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "essentia".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "2.1b6.dev1389".to_string(),
        features_json: serde_json::to_string(&essentia_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, None, None, Some(&essentia_cache), true, None);

    let aa = &result["audio_analysis"];
    assert!(
        aa.is_object(),
        "audio_analysis should be populated when essentia cache exists"
    );
    assert!(
        aa["stratum_dsp"].is_null(),
        "stratum_dsp should remain null when not cached"
    );
    assert!(
        aa["essentia"].is_object(),
        "essentia should expose cached analysis JSON"
    );
    assert_eq!(aa["essentia"]["danceability"], 0.82);

    let dc = &result["data_completeness"];
    assert_eq!(dc["essentia"], true);
    assert_eq!(dc["essentia_installed"], true);
}

#[test]
fn resolve_single_track_stratum_disagreement() {
    let track = make_test_track("t7", "House", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 64.0,
        "key": "Cm",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, None, Some(&stratum_cache), None, false, None);

    let aa = &result["audio_analysis"];
    assert_eq!(
        aa["bpm_agreement"], false,
        "BPM 128.0 vs 64.0 should disagree"
    );
    assert_eq!(aa["key_agreement"], false, "Key Am vs Cm should disagree");
}

#[test]
fn resolve_single_track_enrichment_no_match_returns_null() {
    let track = make_test_track("t8", "House", 126.0, "Am");

    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t8".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("none".to_string()),
        response_json: None,
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, Some(&discogs_cache), None, None, None, false, None);

    assert!(
        result["discogs"].is_null(),
        "discogs with no response_json should be null"
    );
    assert_eq!(
        result["data_completeness"]["discogs"], true,
        "cache entry exists so completeness is true"
    );
}

#[test]
fn musical_key_to_camelot_converts_major_minor_and_flats() {
    assert_eq!(
        musical_key_to_camelot("Am").map(format_camelot).as_deref(),
        Some("8A")
    );
    assert_eq!(
        musical_key_to_camelot("C").map(format_camelot).as_deref(),
        Some("8B")
    );
    assert_eq!(
        musical_key_to_camelot("F#m").map(format_camelot).as_deref(),
        Some("11A")
    );
    assert_eq!(
        musical_key_to_camelot("Bb").map(format_camelot).as_deref(),
        Some("6B")
    );
    assert_eq!(
        musical_key_to_camelot("Dbm").map(format_camelot).as_deref(),
        Some("12A")
    );
    assert_eq!(
        key_to_camelot("8a").map(format_camelot).as_deref(),
        Some("8A")
    );
    assert_eq!(musical_key_to_camelot("not-a-key"), None);
}

#[test]
fn camelot_distance_scoring_handles_wrap_and_mode_shift() {
    let wrap_up = score_key_axis(parse_camelot_key("12A"), parse_camelot_key("1A"));
    assert_eq!(wrap_up.value, 0.9);
    assert!(
        wrap_up.label.contains("Camelot adjacent"),
        "wrap-around up should be treated as +1"
    );

    let wrap_down = score_key_axis(parse_camelot_key("1A"), parse_camelot_key("12A"));
    assert_eq!(wrap_down.value, 0.9);
    assert!(
        wrap_down.label.contains("Camelot adjacent"),
        "wrap-around down should be treated as -1"
    );

    let mood_shift = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("6B"));
    assert_eq!(mood_shift.value, 0.8);

    let diagonal = score_key_axis(parse_camelot_key("6A"), parse_camelot_key("7B"));
    assert_eq!(diagonal.value, 0.55);
    assert!(
        diagonal.label.contains("Energy diagonal"),
        "cross-letter ±1 should be Energy diagonal"
    );
}

#[test]
fn key_axis_covers_all_relation_types() {
    let perfect = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("8A"));
    assert_eq!(perfect.value, 1.0);
    assert_eq!(perfect.label, "Perfect");

    let adjacent = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("9A"));
    assert_eq!(adjacent.value, 0.9);
    assert!(adjacent.label.contains("Camelot adjacent"));

    let mood = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("8B"));
    assert_eq!(mood.value, 0.8);
    assert!(mood.label.contains("Mood shift"));

    let diagonal = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("9B"));
    assert_eq!(diagonal.value, 0.55);
    assert!(diagonal.label.contains("Energy diagonal"));

    let extended = score_key_axis(parse_camelot_key("8A"), parse_camelot_key("10A"));
    assert_eq!(extended.value, 0.45);
    assert!(extended.label.contains("Extended"));

    let clash = score_key_axis(parse_camelot_key("1A"), parse_camelot_key("6A"));
    assert_eq!(clash.value, 0.1);
    assert_eq!(clash.label, "Clash");
}

#[test]
fn bpm_exponential_scoring_curve() {
    // exp(-0.019 * pct²): 0% → 1.0, monotonically decreasing
    let seamless = score_bpm_axis(128.0, 129.5); // 1.17%
    assert!(
        seamless.value > 0.97,
        "1.17% should score near 1.0, got {}",
        seamless.value
    );
    assert!(seamless.label.contains("Seamless"));

    let comfortable = score_bpm_axis(130.0, 126.5); // 2.69%
    assert!(
        comfortable.value > 0.85 && comfortable.value < 0.95,
        "2.69% should be ~0.87, got {}",
        comfortable.value
    );
    assert!(comfortable.label.contains("Comfortable"));

    let noticeable = score_bpm_axis(120.0, 125.5); // 4.58%
    assert!(
        noticeable.value > 0.55 && noticeable.value < 0.75,
        "4.58% should be ~0.65, got {}",
        noticeable.value
    );
    assert!(noticeable.label.contains("Noticeable"));

    let creative = score_bpm_axis(128.0, 138.0); // 7.81%
    assert!(
        creative.value > 0.25 && creative.value < 0.45,
        "7.81% should be ~0.33, got {}",
        creative.value
    );
    assert!(creative.label.contains("Creative transition needed"));

    let jarring = score_bpm_axis(120.0, 132.0); // 10.0%
    assert!(
        jarring.value < 0.20,
        "10% should be near 0, got {}",
        jarring.value
    );
    assert!(jarring.label.contains("Jarring"));

    let unknown = score_bpm_axis(0.0, 128.0);
    assert_eq!(unknown.value, 0.5);
    assert_eq!(unknown.label, "Unknown BPM");

    let at_0 = score_bpm_axis(128.0, 128.0);
    let at_1 = score_bpm_axis(128.0, 129.28); // ~1%
    let at_3 = score_bpm_axis(128.0, 131.84); // ~3%
    let at_5 = score_bpm_axis(128.0, 134.4); // ~5%
    let at_8 = score_bpm_axis(128.0, 138.24); // ~8%
    assert!(at_0.value > at_1.value);
    assert!(at_1.value > at_3.value);
    assert!(at_3.value > at_5.value);
    assert!(at_5.value > at_8.value);
}

#[test]
fn transpose_camelot_key_circle_of_fifths() {
    // +1 semitone = +7 Camelot positions: 8A → 3A
    let k8a = parse_camelot_key("8A").unwrap();
    let up1 = transpose_camelot_key(k8a, 1);
    assert_eq!(format_camelot(up1), "3A");

    let full = transpose_camelot_key(k8a, 12);
    assert_eq!(format_camelot(full), "8A");

    let down1 = transpose_camelot_key(k8a, -1);
    assert_eq!(format_camelot(down1), "1A");

    let round_trip = transpose_camelot_key(up1, -1);
    assert_eq!(format_camelot(round_trip), "8A");

    let k5b = parse_camelot_key("5B").unwrap();
    let up2 = transpose_camelot_key(k5b, 2);
    assert!(
        format_camelot(up2).ends_with('B'),
        "letter should be preserved through transposition"
    );
}

#[test]
fn master_tempo_off_changes_key_scoring() {
    let from = TrackProfile {
        track: crate::types::Track {
            id: "mt-from".to_string(),
            title: "MT From".to_string(),
            artist: "Test".to_string(),
            album: String::new(),
            genre: "House".to_string(),
            key: "Am".to_string(),
            bpm: 128.0,
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 0,
            length: 300,
            file_path: "/tmp/mt-from.flac".to_string(),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::types::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        },
        camelot_key: parse_camelot_key("8A"),
        key_display: "8A".to_string(),
        bpm: 128.0,
        energy: 0.6,
        brightness: None,
        rhythm_regularity: None,
        loudness_range: None,
        canonical_genre: Some("House".to_string()),
        genre_family: GenreFamily::House,
        timbral: None,
    };

    // 128/135 BPM → -1 semitone pitch shift
    let mut to = from.clone();
    to.track.id = "mt-to".to_string();
    to.bpm = 135.0;
    to.camelot_key = parse_camelot_key("8A"); // same key naturally

    let scores_mt_on = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );
    assert_eq!(
        scores_mt_on.key.value, 1.0,
        "master_tempo on: same key should be perfect"
    );
    assert_eq!(scores_mt_on.pitch_shift_semitones, 0);
    assert!(scores_mt_on.effective_to_key.is_none());

    let scores_mt_off = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ScoringContext::default(),
        None,
    );
    assert_eq!(
        scores_mt_off.pitch_shift_semitones, -1,
        "128→135 BPM should yield -1 semitone shift"
    );
    assert_eq!(
        scores_mt_off.effective_to_key,
        Some("1A".to_string()),
        "8A shifted -1 semitone = 1A"
    );
    // Continuous detuning blends Perfect (1.0) and Clash (0.1) weighted by
    // fractional semitones (-0.909), so score is slightly above 0.1.
    assert!(
        scores_mt_off.key.value > 0.1 && scores_mt_off.key.value < 0.25,
        "128→135: key score should be slightly above 0.1 due to detuning blend, got {}",
        scores_mt_off.key.value,
    );
    assert_eq!(
        scores_mt_on.key.value, 1.0,
        "master_tempo on: same key is perfect (1.0)"
    );
}

#[test]
fn detuning_eliminates_cliff_at_rounding_boundary() {
    // Regression: old rounding caused a 10x score cliff at 0.5 semitones.
    // Continuous detuning should produce similar scores on either side.
    let from = make_test_profile("cliff-from", "8A", 128.0, 0.5, "House");
    let to_under = make_test_profile("cliff-under", "8A", 131.5, 0.5, "House");
    let to_over = make_test_profile("cliff-over", "8A", 132.0, 0.5, "House");

    let ctx = ScoringContext::default();

    let scores_under = score_transition_profiles(
        &from,
        &to_under,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        None,
    );
    let scores_over = score_transition_profiles(
        &from,
        &to_over,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        None,
    );

    let diff = (scores_under.key.value - scores_over.key.value).abs();
    assert!(
        diff < 0.15,
        "Key scores across the rounding boundary should be similar: \
         under={:.3} (shift {:.3}), over={:.3} (shift {:.3}), diff={:.3}",
        scores_under.key.value,
        12.0 * (128.0_f64 / 131.5).log2(),
        scores_over.key.value,
        12.0 * (128.0_f64 / 132.0).log2(),
        diff,
    );

    assert!(
        scores_under.key.value < 0.85,
        "~0.46 semitone detuning should noticeably reduce key score, got {}",
        scores_under.key.value,
    );
}

#[test]
fn detuning_smooth_degradation_with_increasing_shift() {
    let from = make_test_profile("smooth-from", "8A", 128.0, 0.5, "House");
    let ctx = ScoringContext::default();

    let bpms = [128.0, 129.0, 130.0, 131.0, 132.0, 133.0, 134.0, 135.0];
    let mut prev_score = 1.1_f64;

    for &bpm in &bpms {
        let to = make_test_profile("smooth-to", "8A", bpm, 0.5, "House");
        let scores = score_transition_profiles(
            &from,
            &to,
            None,
            None,
            &priority_weights(SequencingPriority::Balanced),
            false,
            None,
            &ctx,
            None,
        );

        assert!(
            scores.key.value <= prev_score + 0.01,
            "Key score should not increase: at {bpm} BPM got {:.3}, prev was {:.3}",
            scores.key.value,
            prev_score,
        );
        prev_score = scores.key.value;
    }

    let same = make_test_profile("smooth-same", "8A", 128.0, 0.5, "House");
    let scores_same = score_transition_profiles(
        &from,
        &same,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        None,
    );
    assert_eq!(scores_same.key.value, 1.0, "Same BPM should be perfect");
}

#[test]
fn detuning_master_tempo_on_unchanged() {
    let from = make_test_profile("mt-on-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("mt-on-to", "8A", 135.0, 0.5, "House");
    let ctx = ScoringContext::default();

    let scores = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ctx,
        None,
    );

    assert_eq!(
        scores.key.value, 1.0,
        "Master tempo ON: same key should always be perfect regardless of BPM"
    );
}

#[test]
fn detuning_label_shows_cents_when_audible() {
    let from = make_test_profile("label-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("label-to", "8A", 130.5, 0.5, "House");
    let ctx = ScoringContext::default();

    let scores = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        None,
    );

    assert!(
        scores.key.label.contains("detuned"),
        "Label should mention detuning for ~34 cents shift, got: {}",
        scores.key.label,
    );
}

#[test]
fn detuning_play_bpms_bilinear_interpolation() {
    // Both tracks have fractional pitch shifts, exercising all 4 bilinear blend combinations.
    let from = make_test_profile("pb-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("pb-to", "8A", 132.0, 0.5, "House");
    let ctx = ScoringContext::default();

    let scores = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        Some((130.0, 130.0)), // both pitched to 130
    );

    assert!(
        scores.key.value > 0.5 && scores.key.value < 0.85,
        "Bilinear blend with ~0.53 total shift should score moderately, got {}",
        scores.key.value,
    );
}

#[test]
fn detuning_play_bpms_master_tempo_on_ignores_shifts() {
    let from = make_test_profile("pb-mt-from", "8A", 128.0, 0.5, "House");
    let to = make_test_profile("pb-mt-to", "8A", 135.0, 0.5, "House");
    let ctx = ScoringContext::default();

    let scores = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        true, // master tempo ON
        None,
        &ctx,
        Some((130.0, 130.0)),
    );

    assert_eq!(
        scores.key.value, 1.0,
        "Master tempo ON with play_bpms: same key should be perfect, got {}",
        scores.key.value,
    );
}

#[test]
fn detuning_play_bpms_asymmetric_one_zero_shift() {
    let from = make_test_profile("pb-asym-from", "8A", 130.0, 0.5, "House");
    let to = make_test_profile("pb-asym-to", "8A", 132.0, 0.5, "House");
    let ctx = ScoringContext::default();

    let scores = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ctx,
        Some((130.0, 130.0)), // from at native, to pitched down 2 BPM
    );

    assert!(
        scores.key.value > 0.7 && scores.key.value < 0.9,
        "One-sided ~0.26 semitone shift should score ~0.76, got {}",
        scores.key.value,
    );
}

fn make_test_profile(id: &str, key: &str, bpm: f64, energy: f64, genre: &str) -> TrackProfile {
    TrackProfile {
        track: crate::types::Track {
            id: id.to_string(),
            title: id.to_string(),
            artist: "Test".to_string(),
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
            year: 0,
            length: 300,
            file_path: format!("/tmp/{id}.flac"),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: crate::types::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        },
        camelot_key: parse_camelot_key(key),
        key_display: key.to_string(),
        bpm,
        energy,
        brightness: None,
        rhythm_regularity: None,
        loudness_range: None,
        canonical_genre: Some(genre.to_string()),
        genre_family: genre_family_for(genre),
        timbral: None,
    }
}

#[test]
fn harmonic_style_conservative_penalizes_poor_transitions() {
    let from = make_test_profile("hs-from", "8A", 128.0, 0.7, "House");
    let to = make_test_profile("hs-to", "9B", 128.0, 0.7, "House");

    let conservative = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Conservative),
        &ScoringContext::default(),
        None,
    );

    let baseline = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );

    assert!(
        conservative.composite < baseline.composite,
        "conservative should penalize key=0.55 at peak phase"
    );
    let expected = baseline.composite * 0.1;
    assert!(
        (conservative.composite - expected).abs() < 1e-9,
        "conservative penalty should be 0.1x; got {} vs expected {}",
        conservative.composite,
        expected
    );

    let adventurous = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Adventurous),
        &ScoringContext::default(),
        None,
    );
    assert_eq!(
        adventurous.composite, baseline.composite,
        "adventurous should not penalize key=0.55 at peak phase"
    );

    // key=0.45 at exactly the Balanced build threshold (0.45) should not trigger penalty
    let from2 = make_test_profile("hs-from2", "8A", 128.0, 0.5, "House");
    let to2 = make_test_profile("hs-to2", "10A", 128.0, 0.6, "House");
    let balanced_build = score_transition_profiles(
        &from2,
        &to2,
        Some(EnergyPhase::Build),
        Some(EnergyPhase::Build),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Balanced),
        &ScoringContext::default(),
        None,
    );
    let baseline_build = score_transition_profiles(
        &from2,
        &to2,
        Some(EnergyPhase::Build),
        Some(EnergyPhase::Build),
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );
    assert_eq!(
        balanced_build.composite, baseline_build.composite,
        "balanced should not penalize key=0.45 at build phase (exactly at threshold)"
    );
}

#[test]
fn harmonic_style_adventurous_is_phase_dependent() {
    let from = make_test_profile("adv-from", "8A", 128.0, 0.7, "House");
    let to = make_test_profile("adv-to", "2A", 128.0, 0.7, "House");

    let adv_peak = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Adventurous),
        &ScoringContext::default(),
        None,
    );
    let baseline_peak = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );
    assert_eq!(
        adv_peak.composite, baseline_peak.composite,
        "adventurous at peak should not penalize key=0.1 (threshold is 0.1)"
    );

    let adv_warmup = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Warmup),
        Some(EnergyPhase::Warmup),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Adventurous),
        &ScoringContext::default(),
        None,
    );
    let baseline_warmup = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Warmup),
        Some(EnergyPhase::Warmup),
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );
    assert!(
        adv_warmup.composite < baseline_warmup.composite,
        "adventurous at warmup should penalize key=0.1 (threshold is 0.45)"
    );
    let expected = baseline_warmup.composite * 0.5;
    assert!(
        (adv_warmup.composite - expected).abs() < 1e-9,
        "adventurous penalty should be 0.5x; got {} vs expected {}",
        adv_warmup.composite,
        expected
    );

    let cons_peak = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Conservative),
        &ScoringContext::default(),
        None,
    );
    let cons_warmup = score_transition_profiles(
        &from,
        &to,
        Some(EnergyPhase::Warmup),
        Some(EnergyPhase::Warmup),
        &priority_weights(SequencingPriority::Balanced),
        true,
        Some(HarmonicMixingStyle::Conservative),
        &ScoringContext::default(),
        None,
    );
    assert!(cons_peak.composite < baseline_peak.composite);
    assert!(cons_warmup.composite < baseline_warmup.composite);
}

#[test]
fn composite_scoring_changes_by_priority_axis() {
    let approx = |left: f64, right: f64| (left - right).abs() < 1e-9;

    assert!(approx(
        composite_score(
            1.0,
            0.0,
            0.0,
            0.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Balanced)
        ),
        0.30
    ));
    assert!(approx(
        composite_score(
            1.0,
            0.0,
            0.0,
            0.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Harmonic)
        ),
        0.48
    ));
    assert!(approx(
        composite_score(
            1.0,
            0.0,
            0.0,
            0.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Energy)
        ),
        0.12
    ));
    assert!(approx(
        composite_score(
            1.0,
            0.0,
            0.0,
            0.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Genre)
        ),
        0.18
    ));

    assert!(approx(
        composite_score(
            0.0,
            0.0,
            0.0,
            1.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Balanced)
        ),
        0.17
    ));
    assert!(approx(
        composite_score(
            0.0,
            0.0,
            0.0,
            1.0,
            Some(0.0),
            Some(0.0),
            &priority_weights(SequencingPriority::Genre)
        ),
        0.38
    ));

    assert!(approx(
        composite_score(
            1.0,
            0.0,
            0.0,
            0.0,
            None,
            None,
            &priority_weights(SequencingPriority::Balanced)
        ),
        0.30 / 0.85
    ));
}

#[test]
fn score_genre_axis_treats_missing_genre_as_neutral() {
    let unknown_source = score_genre_axis(
        None,
        Some("House"),
        GenreFamily::Other,
        GenreFamily::House,
        0,
    );
    assert_eq!(unknown_source.value, 0.5);
    assert_eq!(unknown_source.label, "Unknown genre");

    let unknown_destination = score_genre_axis(
        Some("House"),
        None,
        GenreFamily::House,
        GenreFamily::Other,
        0,
    );
    assert_eq!(unknown_destination.value, 0.5);
    assert_eq!(unknown_destination.label, "Unknown genre");
}

#[test]
fn genre_stickiness_bonus_and_penalty() {
    let approx = |a: f64, b: f64| (a - b).abs() < 1e-9;

    let bonus = score_genre_axis(
        Some("Deep House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        3,
    );
    assert!(
        approx(bonus.value, 0.8),
        "0.7 + 0.1 streak bonus; got {}",
        bonus.value
    );
    assert!(bonus.label.contains("streak bonus"));

    let no_bonus = score_genre_axis(
        Some("Deep House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        5,
    );
    assert_eq!(no_bonus.value, 0.7);
    assert!(!no_bonus.label.contains("streak bonus"));

    let penalty = score_genre_axis(
        Some("House"),
        Some("Drum & Bass"),
        GenreFamily::House,
        GenreFamily::Bass,
        1,
    );
    assert!(
        approx(penalty.value, 0.2),
        "0.3 - 0.1 early switch penalty; got {}",
        penalty.value
    );
    assert!(penalty.label.contains("early switch penalty"));

    let no_penalty = score_genre_axis(
        Some("House"),
        Some("Drum & Bass"),
        GenreFamily::House,
        GenreFamily::Bass,
        2,
    );
    assert_eq!(no_penalty.value, 0.3);
    assert!(!no_penalty.label.contains("early switch penalty"));

    let first = score_genre_axis(
        Some("House"),
        Some("Tech House"),
        GenreFamily::House,
        GenreFamily::House,
        0,
    );
    assert_eq!(first.value, 0.7);
    assert!(!first.label.contains("streak bonus"));
}

#[test]
fn bpm_trajectory_drift_penalty() {
    use std::collections::HashMap;

    let start = make_test_profile("bpm-start", "8A", 128.0, 0.7, "House");
    let close = make_test_profile("bpm-close", "8A", 130.0, 0.7, "House");
    let far = make_test_profile("bpm-far", "8A", 145.0, 0.7, "House");

    let mut profiles: HashMap<String, TrackProfile> = HashMap::new();
    profiles.insert("bpm-start".to_string(), start);
    profiles.insert("bpm-close".to_string(), close);
    profiles.insert("bpm-far".to_string(), far);

    let tight = build_candidate_plan(
        &profiles,
        "bpm-start",
        3,
        &[EnergyPhase::Build, EnergyPhase::Build, EnergyPhase::Build],
        &priority_weights(SequencingPriority::Harmonic),
        0,
        true,
        None,
        3.0,
        None,
    );
    assert_eq!(tight.ordered_ids[1], "bpm-close");

    let moderate = build_candidate_plan(
        &profiles,
        "bpm-start",
        3,
        &[EnergyPhase::Build, EnergyPhase::Build, EnergyPhase::Build],
        &priority_weights(SequencingPriority::Harmonic),
        0,
        true,
        None,
        6.0,
        None,
    );
    assert_eq!(moderate.ordered_ids[1], "bpm-close");
    assert!(moderate.ordered_ids.contains(&"bpm-far".to_string()));

    let generous = build_candidate_plan(
        &profiles,
        "bpm-start",
        3,
        &[EnergyPhase::Build, EnergyPhase::Build, EnergyPhase::Build],
        &priority_weights(SequencingPriority::Harmonic),
        0,
        true,
        None,
        50.0,
        None,
    );
    assert_eq!(generous.ordered_ids[1], "bpm-close");
    assert!(generous.ordered_ids.contains(&"bpm-far".to_string()));
}

#[test]
fn bpm_proxy_energy_keeps_peak_phase_reachable_without_essentia() {
    let from_energy = compute_track_energy(None, 126.0);
    let to_energy = compute_track_energy(None, 130.0);
    let peak = score_energy_axis(
        from_energy,
        to_energy,
        Some(EnergyPhase::Peak),
        Some(EnergyPhase::Peak),
        None,
    );

    assert!(
        to_energy >= 0.65,
        "fallback energy should allow peak thresholds"
    );
    assert_eq!(peak.value, 1.0);
    assert_eq!(peak.label, "High and stable (peak phase)");
}

#[tokio::test]
async fn score_transition_returns_expected_axis_scores() {
    let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let from_path = temp_audio_dir.path().join("from-track.flac");
    let to_path = temp_audio_dir.path().join("to-track.flac");
    std::fs::write(&from_path, b"from-track-audio").expect("source track fixture should write");
    std::fs::write(&to_path, b"to-track-audio").expect("target track fixture should write");
    let from_path_str = from_path.to_string_lossy().to_string();
    let to_path_str = to_path.to_string_lossy().to_string();
    let from_metadata = std::fs::metadata(&from_path).expect("source track metadata should load");
    let to_metadata = std::fs::metadata(&to_path).expect("target track metadata should load");
    let from_file_size = from_metadata.len() as i64;
    let to_file_size = to_metadata.len() as i64;
    let from_file_mtime = from_metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let to_file_mtime = to_metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let db_conn = create_single_track_test_db("from-track", &from_path_str);
    db_conn
        .execute(
            "INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Em')",
            [],
        )
        .expect("second key should insert");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, 'Second Track', 'a1', 'al1', 'g1', 'k2', 'c1', 'l1', '',
                    12350, 153, 'score transition test', 2025, 260, ?2, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params!["to-track", &to_path_str],
        )
        .expect("second track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    store::set_audio_analysis(
        &store_conn,
        &from_path_str,
        "stratum-dsp",
        from_file_size,
        from_file_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":122.0,"key":"Am","key_camelot":"8A"}"#,
    )
    .expect("source stratum cache should seed");
    store::set_audio_analysis(
        &store_conn,
        &to_path_str,
        "stratum-dsp",
        to_file_size,
        to_file_mtime,
        crate::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":123.5,"key":"Em","key_camelot":"9A"}"#,
    )
    .expect("destination stratum cache should seed");

    store::set_audio_analysis(
        &store_conn,
        &from_path_str,
        "essentia",
        from_file_size,
        from_file_mtime,
        crate::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":0.90,"loudness_integrated":-12.0,"onset_rate":3.0}"#,
    )
    .expect("source essentia cache should seed");
    store::set_audio_analysis(
        &store_conn,
        &to_path_str,
        "essentia",
        to_file_size,
        to_file_mtime,
        crate::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":1.80,"loudness_integrated":-8.0,"onset_rate":5.0}"#,
    )
    .expect("destination essentia cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "from-track".to_string(),
            target_track_id: "to-track".to_string(),
            energy_phase: Some(EnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
        }))
        .await
        .expect("score_transition should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["from"]["track_id"], "from-track");
    assert_eq!(payload["from"]["key"], "8A");
    assert_eq!(payload["to"]["track_id"], "to-track");
    assert_eq!(payload["to"]["key"], "9A");

    assert_eq!(payload["scores"]["key"]["value"], 0.9);
    assert_eq!(payload["scores"]["bpm"]["value"], 0.791);
    assert_eq!(payload["scores"]["energy"]["value"], 1.0);
    assert_eq!(payload["scores"]["genre"]["value"], 1.0);
    assert_eq!(payload["scores"]["brightness"]["value"], 0.5);
    assert_eq!(payload["scores"]["rhythm"]["value"], 0.5);
    assert_eq!(payload["scores"]["composite"], 0.915);

    assert!(
        payload["key_relation"].is_string(),
        "key_relation should be present"
    );
    assert!(
        payload["key_relation"]
            .as_str()
            .unwrap()
            .contains("Camelot adjacent")
    );
    assert!(
        payload["bpm_adjustment_pct"].is_number(),
        "bpm_adjustment_pct should be present"
    );
    let bpm_pct = payload["bpm_adjustment_pct"].as_f64().unwrap();
    assert!(
        bpm_pct > 3.0 && bpm_pct < 4.0,
        "128→123.5 is ~3.52%; got {bpm_pct}"
    );
}

#[tokio::test]
async fn score_transition_balanced_default_penalizes_clash() {
    // harmonic_style: None defaults to Balanced, which applies 0.5x penalty on Clash
    let db_conn = create_single_track_test_db("clash-from", "/tmp/clash-from.flac");
    db_conn
        .execute(
            "INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Bbm')",
            [],
        )
        .expect("second key should insert");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, 'Clash Track', 'a1', 'al1', 'g1', 'k2', 'c1', 'l1', '',
                    12200, 153, 'clash test', 2025, 260, ?2, '0', 1411,
                    44100, 5, '2025-01-03', 0
                )",
            params!["clash-to", "/tmp/clash-to.flac"],
        )
        .expect("second track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    store::set_audio_analysis(
        &store_conn,
        "/tmp/clash-from.flac",
        "stratum-dsp",
        1,
        1,
        "stratum-dsp-1.0.0",
        r#"{"bpm":122.0,"key":"Am","key_camelot":"8A"}"#,
    )
    .expect("from stratum should seed");
    store::set_audio_analysis(
        &store_conn,
        "/tmp/clash-to.flac",
        "stratum-dsp",
        1,
        1,
        "stratum-dsp-1.0.0",
        r#"{"bpm":122.0,"key":"Bbm","key_camelot":"2A"}"#,
    )
    .expect("to stratum should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let penalized = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "clash-from".to_string(),
            target_track_id: "clash-to".to_string(),
            energy_phase: Some(EnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
        }))
        .await
        .expect("score_transition should succeed");
    let penalized_payload = extract_json(&penalized);

    let unpenalized = server
        .score_transition(Parameters(ScoreTransitionParams {
            source_track_id: "clash-from".to_string(),
            target_track_id: "clash-to".to_string(),
            energy_phase: Some(EnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: Some(HarmonicMixingStyle::Adventurous),
        }))
        .await
        .expect("score_transition should succeed");
    let unpenalized_payload = extract_json(&unpenalized);

    assert_eq!(penalized_payload["scores"]["key"]["value"], 0.1);
    assert_eq!(unpenalized_payload["scores"]["key"]["value"], 0.1);

    let penalized_composite = penalized_payload["scores"]["composite"].as_f64().unwrap();
    let unpenalized_composite = unpenalized_payload["scores"]["composite"].as_f64().unwrap();
    let expected = unpenalized_composite * 0.5;
    assert!(
        (penalized_composite - expected).abs() < 0.01,
        "Balanced default should halve composite for Clash; got {penalized_composite} vs expected {expected}"
    );
}

#[test]
fn flatten_json_round_trip_search_tracks_params() {
    let json = serde_json::json!({
        "query": "burial",
        "artist": "Burial",
        "genre": "Dubstep",
        "rating_min": 3,
        "bpm_min": 130.0,
        "bpm_max": 145.0,
        "key": "Am",
        "has_genre": true,
        "label": "Hyperdub",
        "path": "/Music",
        "added_after": "2026-01-01",
        "added_before": "2026-12-31",
        "playlist": "p1",
        "include_samples": true,
        "limit": 50,
        "offset": 10,
    });
    let p: SearchTracksParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.query.as_deref(), Some("burial"));
    assert_eq!(p.filters.artist.as_deref(), Some("Burial"));
    assert_eq!(p.filters.genre.as_deref(), Some("Dubstep"));
    assert_eq!(p.filters.rating_min, Some(3));
    assert_eq!(p.filters.bpm_min, Some(130.0));
    assert_eq!(p.filters.bpm_max, Some(145.0));
    assert_eq!(p.filters.key.as_deref(), Some("Am"));
    assert_eq!(p.filters.has_genre, Some(true));
    assert_eq!(p.filters.label.as_deref(), Some("Hyperdub"));
    assert_eq!(p.filters.path.as_deref(), Some("/Music"));
    assert_eq!(p.filters.added_after.as_deref(), Some("2026-01-01"));
    assert_eq!(p.filters.added_before.as_deref(), Some("2026-12-31"));
    assert_eq!(p.playlist.as_deref(), Some("p1"));
    assert_eq!(p.include_samples, Some(true));
    assert_eq!(p.limit, Some(50));
    assert_eq!(p.offset, Some(10));
}

#[test]
fn flatten_json_round_trip_enrich_tracks_params() {
    let json = serde_json::json!({
        "genre": "Techno",
        "bpm_min": 125.0,
        "track_ids": ["t1", "t2"],
        "playlist_id": "p1",
        "max_tracks": 20,
        "providers": ["discogs", "beatport"],
        "skip_cached": false,
        "force_refresh": true,
    });
    let p: EnrichTracksParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.genre.as_deref(), Some("Techno"));
    assert_eq!(p.filters.bpm_min, Some(125.0));
    assert_eq!(p.filters.query, None);
    assert_eq!(p.track_ids.as_ref().unwrap().len(), 2);
    assert_eq!(p.playlist_id.as_deref(), Some("p1"));
    assert_eq!(p.max_tracks, Some(20));
    assert_eq!(p.skip_cached, Some(false));
    assert_eq!(p.force_refresh, Some(true));
}

#[test]
fn flatten_json_round_trip_analyze_audio_batch_params() {
    let json = serde_json::json!({
        "artist": "Aphex Twin",
        "rating_min": 4,
        "track_ids": ["t1"],
        "max_tracks": 10,
        "skip_cached": true,
    });
    let p: AnalyzeAudioBatchParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.artist.as_deref(), Some("Aphex Twin"));
    assert_eq!(p.filters.rating_min, Some(4));
    assert_eq!(p.track_ids.as_ref().unwrap(), &["t1"]);
    assert_eq!(p.max_tracks, Some(10));
    assert_eq!(p.skip_cached, Some(true));
}

#[test]
fn flatten_json_round_trip_resolve_tracks_data_params() {
    let json = serde_json::json!({
        "key": "Cm",
        "has_genre": false,
        "added_after": "2025-06-01",
        "playlist_id": "p2",
        "max_tracks": 100,
    });
    let p: ResolveTracksDataParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.key.as_deref(), Some("Cm"));
    assert_eq!(p.filters.has_genre, Some(false));
    assert_eq!(p.filters.added_after.as_deref(), Some("2025-06-01"));
    assert_eq!(p.playlist_id.as_deref(), Some("p2"));
    assert_eq!(p.max_tracks, Some(100));
}

#[test]
fn flatten_json_empty_payload_deserializes_to_all_none() {
    let json = serde_json::json!({});
    let p: SearchTracksParams = serde_json::from_value(json.clone()).expect("SearchTracksParams");
    assert!(p.filters.query.is_none());
    assert!(p.playlist.is_none());
    assert!(p.limit.is_none());

    let p: EnrichTracksParams = serde_json::from_value(json.clone()).expect("EnrichTracksParams");
    assert!(p.filters.genre.is_none());
    assert!(p.track_ids.is_none());

    let p: AnalyzeAudioBatchParams =
        serde_json::from_value(json.clone()).expect("AnalyzeAudioBatchParams");
    assert!(p.filters.artist.is_none());
    assert!(p.track_ids.is_none());

    let p: ResolveTracksDataParams = serde_json::from_value(json).expect("ResolveTracksDataParams");
    assert!(p.filters.key.is_none());
    assert!(p.track_ids.is_none());
}

#[test]
fn build_set_params_bpm_range_deserializes_from_json_array() {
    let json = serde_json::json!({
        "track_ids": ["a", "b"],
        "target_tracks": 4,
        "beam_width": 3,
        "bpm_range": [124.0, 131.0],
    });
    let p: BuildSetParams =
        serde_json::from_value(json).expect("bpm_range should deserialize from JSON array");
    assert_eq!(p.bpm_range, Some((124.0, 131.0)));
    assert_eq!(p.beam_width, Some(3));
    assert!(p.candidates.is_none());
}

#[test]
fn build_set_params_without_new_fields_deserializes() {
    let json = serde_json::json!({
        "track_ids": ["a"],
        "target_tracks": 2,
        "candidates": 2,
    });
    let p: BuildSetParams = serde_json::from_value(json).expect("legacy fields should still work");
    assert_eq!(p.candidates, Some(2));
    assert!(p.beam_width.is_none());
    assert!(p.bpm_range.is_none());
}

#[test]
fn query_transition_candidates_params_deserializes_from_json() {
    let json = serde_json::json!({
        "from_track_id": "t1",
        "pool_track_ids": ["t2", "t3"],
        "target_bpm": 130.0,
        "limit": 5,
    });
    let p: QueryTransitionCandidatesParams =
        serde_json::from_value(json).expect("QueryTransitionCandidatesParams should deserialize");
    assert_eq!(p.source_track_id, "t1");
    assert_eq!(p.candidate_track_ids.as_ref().unwrap().len(), 2);
    assert_eq!(p.target_bpm, Some(130.0));
    assert_eq!(p.limit, Some(5));
    assert!(p.playlist_id.is_none());
}

/// MCP clients expect filter fields at schema top level, not nested under `filters`.
#[test]
fn flatten_schema_has_top_level_filter_properties() {
    let filter_fields = [
        "query",
        "artist",
        "genre",
        "rating_min",
        "bpm_min",
        "bpm_max",
        "key",
        "has_genre",
        "has_label",
        "label",
        "path",
        "added_after",
        "added_before",
    ];

    fn assert_schema_properties<T: JsonSchema>(
        type_name: &str,
        expected: &[&str],
        forbidden: &[&str],
    ) {
        let schema = schemars::schema_for!(T);
        let root = schema.as_value();
        let props = root
            .get("properties")
            .unwrap_or_else(|| panic!("{type_name} schema should have properties"));
        for field in expected {
            assert!(
                props.get(*field).is_some(),
                "{type_name} schema missing top-level property '{field}'"
            );
        }
        for field in forbidden {
            assert!(
                props.get(*field).is_none(),
                "{type_name} schema should NOT have property '{field}'"
            );
        }
    }

    assert_schema_properties::<SearchTracksParams>(
        "SearchTracksParams",
        &[
            &filter_fields[..],
            &["playlist", "include_samples", "limit", "offset"],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<EnrichTracksParams>(
        "EnrichTracksParams",
        &[
            &filter_fields[..],
            &[
                "track_ids",
                "playlist_id",
                "max_tracks",
                "providers",
                "skip_cached",
                "force_refresh",
            ],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<AnalyzeAudioBatchParams>(
        "AnalyzeAudioBatchParams",
        &[
            &filter_fields[..],
            &["track_ids", "playlist_id", "max_tracks", "skip_cached"],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<ResolveTracksDataParams>(
        "ResolveTracksDataParams",
        &[
            &filter_fields[..],
            &["track_ids", "playlist_id", "max_tracks"],
        ]
        .concat(),
        &["filters"],
    );
}

#[test]
fn bpm_trajectory_warmup_build_peak_release() {
    let phases = vec![
        EnergyPhase::Warmup,
        EnergyPhase::Build,
        EnergyPhase::Build,
        EnergyPhase::Build,
        EnergyPhase::Peak,
        EnergyPhase::Peak,
        EnergyPhase::Release,
        EnergyPhase::Release,
    ];
    let trajectory = compute_bpm_trajectory(&phases, 124.0, 132.0);
    assert_eq!(trajectory.len(), 8);
    assert_eq!(trajectory[0], 124.0);
    assert_eq!(trajectory[1], 124.0);
    assert_eq!(trajectory[2], 128.0);
    assert_eq!(trajectory[3], 132.0);
    assert_eq!(trajectory[4], 132.0);
    assert_eq!(trajectory[5], 132.0);
    assert_eq!(trajectory[6], 132.0);
    assert_eq!(trajectory[7], 124.0);
}

#[test]
fn bpm_trajectory_flat_curve() {
    let phases = vec![EnergyPhase::Peak; 5];
    let trajectory = compute_bpm_trajectory(&phases, 126.0, 133.0);
    assert_eq!(trajectory.len(), 5);
    for bpm in &trajectory {
        assert_eq!(*bpm, 133.0);
    }
}

#[test]
fn bpm_trajectory_single_position() {
    let trajectory = compute_bpm_trajectory(&[EnergyPhase::Peak], 128.0, 132.0);
    assert_eq!(trajectory.len(), 1);
    assert_eq!(trajectory[0], 132.0);
}

#[test]
fn bpm_trajectory_empty() {
    let trajectory = compute_bpm_trajectory(&[], 128.0, 132.0);
    assert!(trajectory.is_empty());
}

#[test]
fn bpm_trajectory_single_build_single_release() {
    let phases = vec![EnergyPhase::Build, EnergyPhase::Peak, EnergyPhase::Release];
    let trajectory = compute_bpm_trajectory(&phases, 120.0, 130.0);
    assert_eq!(trajectory[0], 125.0); // midpoint for single build
    assert_eq!(trajectory[1], 130.0); // peak
    assert_eq!(trajectory[2], 125.0); // midpoint for single release
}

#[test]
fn play_bpms_none_preserves_existing_behavior() {
    let from = make_test_profile("pb-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pb-to", "9A", 130.0, 0.7, "House");

    let without = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        None,
    );
    assert!(without.composite > 0.0);
    assert!(without.effective_to_key.is_none());
    assert_eq!(without.pitch_shift_semitones, 0);
}

#[test]
fn play_bpms_affects_bpm_adjustment_pct() {
    let from = make_test_profile("pbadj-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pbadj-to", "9A", 126.0, 0.7, "House");

    let with_play = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        true,
        None,
        &ScoringContext::default(),
        Some((128.0, 130.0)),
    );
    assert!(
        (with_play.bpm_adjustment_pct - 3.174).abs() < 0.1,
        "bpm_adjustment_pct should reflect target vs native; got {}",
        with_play.bpm_adjustment_pct
    );
}

#[test]
fn play_bpms_affects_key_transposition() {
    let from = make_test_profile("pbkey-from", "8A", 128.0, 0.6, "House");
    let to = make_test_profile("pbkey-to", "8A", 128.0, 0.7, "House");

    let no_shift = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ScoringContext::default(),
        Some((128.0, 128.0)),
    );
    assert_eq!(
        no_shift.key.value, 1.0,
        "same play BPM, same native key = perfect"
    );

    let big_shift = score_transition_profiles(
        &from,
        &to,
        None,
        None,
        &priority_weights(SequencingPriority::Balanced),
        false,
        None,
        &ScoringContext::default(),
        Some((128.0, 136.0)),
    );
    assert_ne!(
        big_shift.pitch_shift_semitones, 0,
        "large BPM shift should transpose key"
    );
}

fn make_beam_test_profiles() -> HashMap<String, TrackProfile> {
    let tracks = vec![
        make_test_profile("b1", "8A", 126.0, 0.4, "Deep House"),
        make_test_profile("b2", "9A", 127.0, 0.5, "Deep House"),
        make_test_profile("b3", "10A", 128.0, 0.6, "House"),
        make_test_profile("b4", "11A", 129.0, 0.7, "House"),
        make_test_profile("b5", "12A", 130.0, 0.8, "Tech House"),
    ];
    tracks
        .into_iter()
        .map(|p| (p.track.id.clone(), p))
        .collect()
}

#[test]
fn beam_search_width_1_matches_greedy() {
    let profiles = make_beam_test_profiles();
    let phases = resolve_energy_curve(None, 4).unwrap();

    let greedy = build_candidate_plan(
        &profiles,
        "b1",
        4,
        &phases,
        &priority_weights(SequencingPriority::Balanced),
        0,
        true,
        Some(HarmonicMixingStyle::Balanced),
        6.0,
        None,
    );
    let beam_plans = build_candidate_plan_beam(
        &profiles,
        "b1",
        4,
        &phases,
        &priority_weights(SequencingPriority::Balanced),
        1,
        true,
        Some(HarmonicMixingStyle::Balanced),
        6.0,
        None,
    );

    assert_eq!(
        beam_plans.len(),
        1,
        "beam width 1 should produce exactly 1 plan"
    );
    assert_eq!(
        greedy.ordered_ids, beam_plans[0].ordered_ids,
        "beam width 1 should match greedy ordering"
    );
}

#[test]
fn beam_search_wider_produces_multiple_plans() {
    let profiles = make_beam_test_profiles();
    let phases = resolve_energy_curve(None, 4).unwrap();

    let plans = build_candidate_plan_beam(
        &profiles,
        "b1",
        4,
        &phases,
        &priority_weights(SequencingPriority::Balanced),
        4,
        true,
        Some(HarmonicMixingStyle::Balanced),
        6.0,
        None,
    );

    assert!(
        plans.len() > 1,
        "beam width 4 with 5-track pool should produce multiple plans; got {}",
        plans.len()
    );

    for plan in &plans {
        assert_eq!(plan.ordered_ids.len(), 4);
        assert_eq!(plan.transitions.len(), 3);
        assert_eq!(plan.ordered_ids[0], "b1", "all plans should start with b1");
    }

    let unique: HashSet<&Vec<String>> = plans.iter().map(|p| &p.ordered_ids).collect();
    assert_eq!(unique.len(), plans.len(), "all plans should be distinct");
}

#[test]
fn beam_search_empty_pool() {
    let profiles: HashMap<String, TrackProfile> = HashMap::new();
    let plans = build_candidate_plan_beam(
        &profiles,
        "missing",
        4,
        &[EnergyPhase::Peak; 4],
        &priority_weights(SequencingPriority::Balanced),
        3,
        true,
        None,
        6.0,
        None,
    );
    assert_eq!(plans.len(), 1, "empty pool should still produce one plan");
    assert_eq!(plans[0].ordered_ids, vec!["missing"]);
    assert!(plans[0].transitions.is_empty());
}

#[test]
fn beam_search_width_exceeding_pool_size() {
    let mut profiles = HashMap::new();
    profiles.insert(
        "only1".to_string(),
        make_test_profile("only1", "8A", 128.0, 0.5, "House"),
    );
    profiles.insert(
        "only2".to_string(),
        make_test_profile("only2", "9A", 128.5, 0.6, "House"),
    );

    let plans = build_candidate_plan_beam(
        &profiles,
        "only1",
        2,
        &[EnergyPhase::Peak; 2],
        &priority_weights(SequencingPriority::Balanced),
        10,
        true,
        None,
        6.0,
        None,
    );

    assert_eq!(plans.len(), 1, "only one possible plan with 2-track pool");
    assert_eq!(plans[0].ordered_ids.len(), 2);
}

#[test]
fn beam_search_with_bpm_trajectory() {
    let profiles = make_beam_test_profiles();
    let phases = vec![
        EnergyPhase::Warmup,
        EnergyPhase::Build,
        EnergyPhase::Peak,
        EnergyPhase::Peak,
    ];
    let target_bpms = compute_bpm_trajectory(&phases, 126.0, 130.0);

    let plans = build_candidate_plan_beam(
        &profiles,
        "b1",
        4,
        &phases,
        &priority_weights(SequencingPriority::Balanced),
        3,
        true,
        Some(HarmonicMixingStyle::Balanced),
        6.0,
        Some(&target_bpms),
    );

    assert!(
        !plans.is_empty(),
        "beam search with trajectory should produce plans"
    );
    for plan in &plans {
        assert_eq!(plan.ordered_ids.len(), 4);
        assert_eq!(plan.ordered_ids[0], "b1");
    }
}

#[tokio::test]
async fn query_transition_candidates_ranks_pool() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let from_id = track_ids[0].clone();
    let pool_ids: Vec<String> = track_ids[1..].to_vec();

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: from_id.clone(),
            candidate_track_ids: Some(pool_ids),
            playlist_id: None,
            target_bpm: None,
            energy_phase: Some(EnergyPhase::Build),
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            use_master_tempo: None,
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect("query_transition_candidates should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["from"]["track_id"], from_id);
    assert!(payload["master_tempo"].as_bool().unwrap());

    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert!(
        !candidates.is_empty(),
        "should return at least one candidate"
    );

    let composites: Vec<f64> = candidates
        .iter()
        .map(|c| c["scores"]["composite"].as_f64().unwrap())
        .collect();
    for window in composites.windows(2) {
        assert!(
            window[0] >= window[1],
            "candidates should be sorted by composite descending"
        );
    }

    for c in candidates {
        assert!(c["track_id"].is_string());
        assert!(c["native_bpm"].is_number());
        assert!(c["native_key"].is_string());
        assert!(c["bpm_difference_pct"].is_number());
        assert!(c["key_relation"].is_string());
        assert!(c["scores"]["composite"].is_number());
        assert!(
            c.get("play_at_bpm").is_none() || c["play_at_bpm"].is_null(),
            "play_at_bpm should not be present without target_bpm"
        );
        assert!(
            c.get("pitch_adjustment_pct").is_none() || c["pitch_adjustment_pct"].is_null(),
            "pitch_adjustment_pct should not be present without target_bpm"
        );
    }
}

#[tokio::test]
async fn query_transition_candidates_with_target_bpm() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: track_ids[0].clone(),
            candidate_track_ids: Some(track_ids[1..].to_vec()),
            playlist_id: None,
            target_bpm: Some(130.0),
            energy_phase: None,
            priority: None,
            use_master_tempo: None,
            harmonic_style: None,
            limit: Some(3),
        }))
        .await
        .expect("query_transition_candidates with target_bpm should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["reference_bpm"], 130.0);

    let candidates = payload["candidates"].as_array().unwrap();
    assert!(candidates.len() <= 3, "limit should be respected");

    for c in candidates {
        assert_eq!(
            c["play_at_bpm"].as_f64().unwrap(),
            130.0,
            "play_at_bpm should equal target_bpm for all candidates"
        );
        assert!(
            c["pitch_adjustment_pct"].as_f64().unwrap() >= 0.0,
            "pitch_adjustment_pct should be non-negative"
        );
    }
}

#[tokio::test]
async fn query_transition_candidates_master_tempo_off() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: track_ids[0].clone(),
            candidate_track_ids: Some(track_ids[1..].to_vec()),
            playlist_id: None,
            target_bpm: Some(135.0), // significant BPM shift to trigger key transposition
            energy_phase: None,
            priority: None,
            use_master_tempo: Some(false),
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect("query_transition_candidates with master_tempo off should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["master_tempo"], false);
    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());
    let has_shift = candidates
        .iter()
        .any(|c| c.get("pitch_shift_semitones").is_some());
    assert!(
        has_shift,
        "with master_tempo off and large BPM shift, some candidates should have pitch_shift_semitones"
    );
}

#[tokio::test]
async fn query_transition_candidates_rejects_missing_pool() {
    let db_conn = create_single_track_test_db("orphan-track", "/tmp/orphan.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let err = server
        .query_transition_candidates(Parameters(QueryTransitionCandidatesParams {
            source_track_id: "orphan-track".to_string(),
            candidate_track_ids: None,
            playlist_id: None,
            target_bpm: None,
            energy_phase: None,
            priority: None,
            use_master_tempo: None,
            harmonic_style: None,
            limit: None,
        }))
        .await
        .expect_err("should reject when neither pool_track_ids nor playlist_id is set");

    let msg = format!("{err:?}");
    assert!(
        msg.contains("pool_track_ids") || msg.contains("playlist_id"),
        "error should mention required pool source; got: {msg}"
    );
}

#[tokio::test]
async fn build_set_beam_search_produces_multiple_candidates() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: None,
            beam_width: Some(5),
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set with beam_width=5 should succeed");

    let payload = extract_json(&result);
    assert_eq!(payload["beam_width"], 5);
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert!(
        candidates.len() > 1,
        "beam_width=5 should produce multiple candidates; got {}",
        candidates.len()
    );

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 4);
        assert!(candidate["set_score"].is_number());
    }
}

#[tokio::test]
async fn build_set_with_bpm_range_includes_trajectory_fields() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids,
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: None,
            beam_width: Some(3),
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: Some((124.0, 131.0)),
        }))
        .await
        .expect("build_set with bpm_range should succeed");

    let payload = extract_json(&result);

    let trajectory = payload["bpm_trajectory"]
        .as_array()
        .expect("bpm_trajectory should be present at set level");
    assert_eq!(trajectory.len(), 4, "trajectory should match target_tracks");

    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        for track in tracks {
            assert!(
                track["play_at_bpm"].is_number(),
                "tracks should include play_at_bpm when bpm_range is set"
            );
            assert!(
                track["pitch_adjustment_pct"].is_number(),
                "tracks should include pitch_adjustment_pct when bpm_range is set"
            );
        }

        let candidate_trajectory = candidate["bpm_trajectory"]
            .as_array()
            .expect("candidate should include bpm_trajectory");
        assert_eq!(candidate_trajectory.len(), 4);
    }
}

#[tokio::test]
async fn build_set_beam_width_1_backward_compatible() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let result = server
        .build_set(Parameters(BuildSetParams {
            track_ids: track_ids.clone(),
            target_tracks: 4,
            priority: Some(TransitionWeightSpec::Named("balanced".into())),
            energy_curve: None,
            opening_track_id: None,
            candidates: Some(1),
            beam_width: None,
            use_master_tempo: None,
            harmonic_style: None,
            bpm_drift_pct: None,
            bpm_range: None,
        }))
        .await
        .expect("build_set with candidates=1 should succeed");

    let payload = extract_json(&result);
    assert_eq!(
        payload["beam_width"], 1,
        "candidates=1 should route to greedy"
    );
    let candidates = payload["candidates"].as_array().unwrap();
    assert!(!candidates.is_empty());

    for candidate in candidates {
        let tracks = candidate["tracks"].as_array().unwrap();
        assert_eq!(tracks.len(), 4);
    }
}

/// Tool schemas must be free of $ref/$defs and top-level oneOf/anyOf for Claude API compatibility.
#[test]
fn tool_schemas_are_claude_api_compatible() {
    fn check<T: JsonSchema>(name: &str) {
        let schema = schemars::schema_for!(T);
        let json = serde_json::to_string(&schema).unwrap();

        assert!(!json.contains(r#""$ref""#), "{name} schema contains $ref");
        assert!(!json.contains(r#""$defs""#), "{name} schema contains $defs");

        let root = schema.as_value();
        assert!(
            root.get("oneOf").is_none(),
            "{name} schema has top-level oneOf"
        );
        assert!(
            root.get("anyOf").is_none(),
            "{name} schema has top-level anyOf"
        );
    }

    check::<AuditOperation>("AuditOperation");
    check::<BuildSetParams>("BuildSetParams");
    check::<ScoreTransitionParams>("ScoreTransitionParams");
    check::<QueryTransitionCandidatesParams>("QueryTransitionCandidatesParams");
    check::<EnrichTracksParams>("EnrichTracksParams");
    check::<WriteFileTagsParams>("WriteFileTagsParams");
    check::<WriteXmlParams>("WriteXmlParams");
    check::<UpdateTracksParams>("UpdateTracksParams");

    check::<SearchTracksParams>("SearchTracksParams");
    check::<GetTrackParams>("GetTrackParams");
    check::<GetPlaylistTracksParams>("GetPlaylistTracksParams");
    check::<PreviewChangesParams>("PreviewChangesParams");
    check::<ClearChangesParams>("ClearChangesParams");
    check::<SuggestNormalizationsParams>("SuggestNormalizationsParams");
    check::<LookupDiscogsParams>("LookupDiscogsParams");
    check::<LookupBeatportParams>("LookupBeatportParams");
    check::<AnalyzeTrackAudioParams>("AnalyzeTrackAudioParams");
    check::<AnalyzeAudioBatchParams>("AnalyzeAudioBatchParams");
    check::<ResolveTrackDataParams>("ResolveTrackDataParams");
    check::<ResolveTracksDataParams>("ResolveTracksDataParams");
    check::<ReadFileTagsParams>("ReadFileTagsParams");
    check::<ExtractCoverArtParams>("ExtractCoverArtParams");
    check::<EmbedCoverArtParams>("EmbedCoverArtParams");
}

// --- build_genre_distribution tests ---

fn make_result(
    genre: Option<&'static str>,
    confidence: crate::classify::ClassificationConfidence,
    action: crate::classify::ClassificationAction,
    artist: &str,
) -> crate::classify::ClassificationResult {
    crate::classify::ClassificationResult {
        track_id: String::new(),
        artist: artist.to_string(),
        title: String::new(),
        current_genre: String::new(),
        genre,
        confidence,
        action,
        evidence: vec![],
        candidates: vec![],
        flags: vec![],
        review_hint: None,
    }
}

#[test]
fn genre_distribution_empty_input() {
    let dist = classify_handler::build_genre_distribution(&[]);
    assert_eq!(dist, serde_json::json!([]));
}

#[test]
fn genre_distribution_excludes_confirm_and_genreless() {
    use crate::classify::{ClassificationAction as A, ClassificationConfidence as C};

    let results = vec![
        make_result(Some("Techno"), C::High, A::Confirm, "Artist A"),
        make_result(None, C::Insufficient, A::Suggest, "Artist B"),
        make_result(Some("House"), C::Medium, A::Suggest, "Artist C"),
    ];
    let dist = classify_handler::build_genre_distribution(&results);
    let arr = dist.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only House should appear");
    assert_eq!(arr[0]["genre"], "House");
    assert_eq!(arr[0]["count"], 1);
}

#[test]
fn genre_distribution_groups_and_sorts_by_count() {
    use crate::classify::{ClassificationAction as A, ClassificationConfidence as C};

    let results = vec![
        make_result(Some("Techno"), C::High, A::Suggest, "A"),
        make_result(Some("Techno"), C::High, A::Suggest, "B"),
        make_result(Some("Techno"), C::Medium, A::Suggest, "A"),
        make_result(Some("House"), C::High, A::Suggest, "C"),
    ];
    let dist = classify_handler::build_genre_distribution(&results);
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
    use crate::classify::{ClassificationAction as A, ClassificationConfidence as C};

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

    let dist = classify_handler::build_genre_distribution(&results);
    let arr = dist.as_array().unwrap();
    let top = arr[0]["top_artists"].as_array().unwrap();
    assert_eq!(top.len(), 5, "top_artists capped at 5");
    // First entry should be "Artist 0 (2)" since it has the highest count
    assert_eq!(top[0].as_str().unwrap(), "Artist 0 (2)");
}
