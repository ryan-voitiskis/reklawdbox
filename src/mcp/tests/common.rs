use crate::mcp::context::ServerContext;
use crate::mcp::server::ReklawdboxServer;
use std::sync::{Arc, Mutex};

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult};
use rusqlite::{Connection, params};
use tempfile::TempDir;

use crate::adapters::rekordbox::test_support::{PrivateFixtureError, PrivateRekordboxFixture};
use crate::adapters::{rekordbox as db, state as store};

pub(super) fn extract_json(result: &CallToolResult) -> serde_json::Value {
    let text = result
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.as_str())
        .expect("tool result should include text content");

    serde_json::from_str(text).expect("tool text content should be valid JSON")
}

pub(super) fn set_test_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
    file_size: i64,
    file_mtime: i64,
    analysis_version: &str,
    features_json: &str,
) -> Result<(), rusqlite::Error> {
    let input_fingerprint = if analyzer == crate::adapters::audio::ANALYZER_STRATUM {
        crate::adapters::audio::STRATUM_HMM_INPUT_FINGERPRINT
    } else {
        ""
    };
    store::set_audio_analysis_with_fingerprint(
        conn,
        file_path,
        analyzer,
        file_size,
        file_mtime,
        analysis_version,
        input_fingerprint,
        features_json,
    )
}

pub(super) fn valid_test_essentia_payload(extra: serde_json::Value) -> String {
    let mut payload = serde_json::json!({
        "analyzer_version": crate::adapters::audio::SUPPORTED_ESSENTIA_VERSION,
        "runtime_manifest": {
            "python_version": "3.14.6",
            "python_implementation": "cpython",
            "essentia_version": crate::adapters::audio::SUPPORTED_ESSENTIA_VERSION,
            "essentia_module_version": crate::adapters::audio::SUPPORTED_ESSENTIA_MODULE_VERSION,
            "numpy_version": crate::adapters::audio::SUPPORTED_NUMPY_VERSION,
            "pyyaml_version": crate::adapters::audio::SUPPORTED_PYYAML_VERSION,
            "six_version": crate::adapters::audio::SUPPORTED_SIX_VERSION,
            "analyzer_contract": crate::adapters::audio::ESSENTIA_CONTRACT_ID,
        },
    });
    if let (Some(payload), Some(extra)) = (payload.as_object_mut(), extra.as_object()) {
        payload.extend(extra.clone());
    }
    serde_json::to_string(&payload).expect("test Essentia payload should serialize")
}

pub(super) async fn call_tool_via_router(
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
    let mut params = CallToolRequestParams::new(tool_name.to_owned());
    if let Some(arguments) = arguments {
        params = params.with_arguments(arguments);
    }

    let result = client
        .call_tool(params)
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

pub(super) fn create_server_with_connections(
    db_conn: Connection,
    store_conn: Connection,
    http: reqwest::Client,
) -> ReklawdboxServer {
    create_server_with_store_path(db_conn, store_conn, http, None)
}

pub(super) fn create_server_with_store_path(
    db_conn: Connection,
    store_conn: Connection,
    http: reqwest::Client,
    store_path: Option<String>,
) -> ReklawdboxServer {
    create_server_with_paths(
        db_conn,
        store_conn,
        http,
        store_path,
        std::path::PathBuf::from("synthetic-master.db"),
    )
}

fn create_server_with_paths(
    db_conn: Connection,
    store_conn: Connection,
    http: reqwest::Client,
    store_path: Option<String>,
    effective_db_path: std::path::PathBuf,
) -> ReklawdboxServer {
    let mut context = ServerContext::new(None, http);
    context.database.store_path = store_path;
    context
        .database
        .db
        .set(Mutex::new(db_conn))
        .expect("test DB should initialize exactly once");
    context
        .database
        .effective_db_path
        .set(effective_db_path)
        .expect("test effective DB path should initialize exactly once");
    context
        .database
        .internal_db
        .set(Mutex::new(store_conn))
        .expect("test internal store should initialize exactly once");

    ReklawdboxServer {
        context: Arc::new(context),
        tool_router: ReklawdboxServer::build_tool_router(),
    }
}

pub(super) struct PrivateServerFixture {
    server: Option<ReklawdboxServer>,
    _store_dir: TempDir,
    _rekordbox_fixture: PrivateRekordboxFixture,
    drop_log: Option<Arc<Mutex<Vec<&'static str>>>>,
}

impl PrivateServerFixture {
    pub(super) fn server(&self) -> &ReklawdboxServer {
        self.server
            .as_ref()
            .expect("private server should remain owned until fixture drop")
    }

    fn with_drop_log(mut self, drop_log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        self.drop_log = Some(drop_log);
        self
    }
}

impl Drop for PrivateServerFixture {
    fn drop(&mut self) {
        // Close both SQLite connections before either temporary root is
        // eligible to disappear. Keeping the server private prevents callers
        // from destructuring the ownership relationship.
        drop(self.server.take());
        if let Some(drop_log) = &self.drop_log
            && let Ok(mut drop_log) = drop_log.lock()
        {
            drop_log.push("server");
        }
    }
}

fn create_private_server_with_temp_store(
    fixture: PrivateRekordboxFixture,
    http: reqwest::Client,
) -> Result<PrivateServerFixture, PrivateFixtureError> {
    let db_conn = fixture.open()?;
    let store_dir = tempfile::tempdir().map_err(PrivateFixtureError::TempRoot)?;
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn =
        store::open(&store_path_str).expect("internal store should open for integration test");

    let server = create_server_with_paths(
        db_conn,
        store_conn,
        http,
        Some(store_path_str),
        fixture.database_path().to_path_buf(),
    );
    Ok(PrivateServerFixture {
        server: Some(server),
        _store_dir: store_dir,
        _rekordbox_fixture: fixture,
        drop_log: None,
    })
}

pub(super) fn create_real_server_with_temp_store(
    http: reqwest::Client,
) -> Result<PrivateServerFixture, PrivateFixtureError> {
    create_private_server_with_temp_store(PrivateRekordboxFixture::from_env()?, http)
}

#[test]
fn private_server_fixture_drops_server_before_database_fixture() {
    let source = tempfile::tempdir().unwrap();
    let source_database = source.path().join("synthetic-master.db");
    {
        let conn = Connection::open(&source_database).unwrap();
        conn.execute_batch(&format!(
            "PRAGMA key = '{}'; CREATE TABLE fixture_identity (value TEXT NOT NULL);",
            db::REKORDBOX_SQLCIPHER_KEY
        ))
        .unwrap();
        conn.execute(
            "INSERT INTO fixture_identity (value) VALUES ('synthetic')",
            [],
        )
        .unwrap();
    }
    let drop_log = Arc::new(Mutex::new(Vec::new()));
    let fixture =
        PrivateRekordboxFixture::from_archive_with(&source_database, |archive, destination| {
            std::fs::copy(archive, destination.join("master.db"))
                .map(|_| ())
                .map_err(|error| PrivateFixtureError::Extraction {
                    diagnostic: error.kind().to_string(),
                })
        })
        .unwrap()
        .with_drop_log(Arc::clone(&drop_log));
    let fixture_root = fixture.root().to_path_buf();
    let server_fixture =
        create_private_server_with_temp_store(fixture, default_http_client_for_tests())
            .unwrap()
            .with_drop_log(Arc::clone(&drop_log));

    let conn = server_fixture.server().rekordbox_conn().unwrap();
    let identity: String = conn
        .query_row("SELECT value FROM fixture_identity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(identity, "synthetic");
    drop(conn);
    drop(server_fixture);

    assert!(!fixture_root.exists());
    assert_eq!(*drop_log.lock().unwrap(), ["server", "fixture"]);
}

pub(super) fn sample_real_tracks(
    server: &ReklawdboxServer,
    limit: u32,
) -> Vec<crate::domain::library::Track> {
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

pub(super) fn write_test_audio_file(path: &std::path::Path, size: usize) -> (i64, i64) {
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

pub(super) fn create_single_track_test_db(track_id: &str, raw_file_path: &str) -> Connection {
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

pub(super) fn insert_test_track(
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

pub(super) fn create_selector_pagination_test_db() -> Connection {
    let conn = create_single_track_test_db("t1", "/music/01-known.flac");
    conn.execute_batch(
        "
            CREATE TABLE djmdSongPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                PlaylistID VARCHAR(255),
                ContentID VARCHAR(255),
                TrackNo INTEGER
            );

            INSERT INTO djmdGenre (ID, Name) VALUES ('g-known-2', 'Techno');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g-unknown-1', 'Wonky Bass');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g-unknown-2', 'Alien Rhythms');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g-unknown-3', 'Future Tribal');

            UPDATE djmdContent
            SET Title = '01 Known', GenreID = 'g1', FolderPath = '/music/01-known.flac'
            WHERE ID = 't1';
        ",
    )
    .expect("selector pagination fixture schema should initialize");

    insert_test_track(
        &conn,
        "t2",
        "02 Unknown First",
        "g-unknown-1",
        "/music/02-unknown.flac",
    );
    insert_test_track(&conn, "t3", "03 Known", "g-known-2", "/music/03-known.flac");
    insert_test_track(
        &conn,
        "t4",
        "04 Unknown Second",
        "g-unknown-2",
        "/music/04-unknown.flac",
    );
    insert_test_track(&conn, "t5", "05 Empty", "", "/music/05-empty.flac");
    insert_test_track(
        &conn,
        "t6",
        "06 Unknown Sample",
        "g-unknown-3",
        &format!("/music{}06-unknown-sample.wav", db::SAMPLER_PATH_FRAGMENT),
    );

    for (row_id, track_id, position) in [
        ("sp1", "t1", 1),
        ("sp2", "t6", 2),
        ("sp3", "t2", 3),
        ("sp4", "t3", 4),
        ("sp5", "t4", 5),
        ("sp6", "t5", 6),
    ] {
        conn.execute(
            "INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo)
             VALUES (?1, 'selector-playlist', ?2, ?3)",
            params![row_id, track_id, position],
        )
        .expect("selector pagination playlist row should insert");
    }

    conn
}

// --- build_genre_distribution tests ---

pub(super) fn default_http_client_for_tests() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent("Reklawdbox/0.1")
        .build()
        .expect("default test HTTP client should build")
}

pub(super) fn create_enrich_cache_writer_test_server(
    db_conn: Connection,
) -> (ReklawdboxServer, TempDir, String) {
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");
    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str.clone()),
    );
    (server, store_dir, store_path_str)
}

pub(super) fn make_test_track(
    id: &str,
    genre: &str,
    bpm: f64,
    key: &str,
) -> crate::domain::library::Track {
    crate::domain::library::Track {
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
        file_kind: crate::domain::library::FileKind::Flac,
        date_added: "2023-01-15".to_string(),
        position: None,
        played_at: None,
    }
}

pub(super) fn track_ids(tracks: &[crate::domain::library::Track]) -> Vec<&str> {
    tracks.iter().map(|track| track.id.as_str()).collect()
}
