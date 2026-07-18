use std::fs;
use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;

use crate::adapters::{rekordbox, state};
use crate::mcp::context::ServerContext;
use crate::mcp::server::ReklawdboxServer;

use super::common::default_http_client_for_tests;

fn server_with_paths(db_path: Option<&Path>, store_path: Option<&Path>) -> ReklawdboxServer {
    let mut context = ServerContext::new(
        db_path.map(|path| path.to_string_lossy().into_owned()),
        default_http_client_for_tests(),
    );
    context.database.store_path = store_path.map(|path| path.to_string_lossy().into_owned());
    ReklawdboxServer {
        context: Arc::new(context),
        tool_router: ReklawdboxServer::build_tool_router(),
    }
}

#[test]
fn retryable_mcp_database_init_effective_path() {
    let dir = tempfile::tempdir().expect("temporary directory should create");
    let master_path = dir.path().join("master.db");
    let server = server_with_paths(Some(&master_path), None);

    let first_error = server
        .effective_db_path()
        .expect_err("missing database path should fail");
    assert!(
        first_error.message.contains("Rekordbox database path")
            && first_error.message.contains("is unavailable"),
        "missing-path error should retain the public diagnostic: {}",
        first_error.message
    );
    assert!(server.context.database.effective_db_path.get().is_none());

    fs::write(&master_path, b"path-only fixture").expect("database path fixture should be created");
    let canonical = master_path
        .canonicalize()
        .expect("database path fixture should canonicalize");
    assert_eq!(
        server
            .effective_db_path()
            .expect("path resolution should retry successfully"),
        canonical
    );

    fs::remove_file(&master_path).expect("path fixture should be removable after caching");
    assert_eq!(
        server
            .effective_db_path()
            .expect("successful path resolution should remain cached"),
        canonical
    );
}

#[test]
fn retryable_mcp_database_init_rekordbox_connection() {
    let dir = tempfile::tempdir().expect("temporary directory should create");
    let master_path = dir.path().join("master.db");
    let moved_path = dir.path().join("opened-master.db");
    let server = server_with_paths(Some(&master_path), None);

    let first_error = server
        .rekordbox_conn()
        .expect_err("missing Rekordbox database should fail");
    assert!(
        first_error.message.contains("is unavailable"),
        "missing database should retain the public diagnostic: {}",
        first_error.message
    );
    assert!(server.context.database.db.get().is_none());

    {
        let conn = Connection::open(&master_path).expect("encrypted fixture should create");
        conn.execute_batch(&format!(
            "PRAGMA key = '{}';
             CREATE TABLE retry_fixture (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
             INSERT INTO retry_fixture (id, value) VALUES (1, 'retained');",
            rekordbox::REKORDBOX_SQLCIPHER_KEY
        ))
        .expect("encrypted fixture should initialize");
    }

    {
        let conn = server
            .rekordbox_conn()
            .expect("Rekordbox connection should retry successfully");
        let value: String = conn
            .query_row("SELECT value FROM retry_fixture WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("production connection should read encrypted fixture");
        assert_eq!(value, "retained");
        assert!(
            conn.execute(
                "INSERT INTO retry_fixture (id, value) VALUES (2, 'forbidden')",
                []
            )
            .is_err(),
            "production Rekordbox connection must remain read-only"
        );
    }

    let retained_mutex = server
        .context
        .database
        .db
        .get()
        .expect("successful connection should be cached");
    let retained_mutex_address = std::ptr::from_ref(retained_mutex);
    fs::rename(&master_path, &moved_path)
        .expect("opened database fixture should move after successful initialization");

    let conn = server
        .rekordbox_conn()
        .expect("later access should reuse the retained connection without reopening");
    assert_eq!(
        std::ptr::from_ref(
            server
                .context
                .database
                .db
                .get()
                .expect("cached connection should remain present")
        ),
        retained_mutex_address
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM retry_fixture", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("retained connection should remain readable"),
        1
    );
}

#[test]
fn retryable_mcp_database_init_internal_store() {
    let dir = tempfile::tempdir().expect("temporary directory should create");
    let master_path = dir.path().join("master.db");
    let master_marker = b"master database must remain untouched";
    fs::write(&master_path, master_marker).expect("master marker should create");

    let blocked_parent = dir.path().join("blocked-parent");
    fs::write(&blocked_parent, b"not a directory").expect("blocking file should create");
    let store_path = blocked_parent.join("internal.sqlite3");
    let server = server_with_paths(Some(&master_path), Some(&store_path));

    let first_error = server
        .cache_store_conn()
        .expect_err("store below a regular file should fail");
    assert!(
        first_error
            .message
            .contains("Failed to open internal store"),
        "store-open error should retain the public diagnostic: {}",
        first_error.message
    );
    assert!(server.context.database.internal_db.get().is_none());
    assert!(server.context.database.db.get().is_none());
    assert!(server.context.database.effective_db_path.get().is_none());

    fs::remove_file(&blocked_parent).expect("blocking file should be removable");
    fs::create_dir(&blocked_parent).expect("store parent should become a directory");
    {
        let conn = server
            .cache_store_conn()
            .expect("internal store initialization should retry successfully");
        let table_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master
                 WHERE type = 'table' AND name IN ('enrichment_cache', 'audio_analysis_cache')",
                [],
                |row| row.get(0),
            )
            .expect("migrated store schema should be queryable");
        assert_eq!(table_count, 2);
        state::set_enrichment(
            &conn,
            "retry-test",
            "Test Artist",
            "Test Title",
            None,
            Some("exact"),
            Some(r#"{"result":"retained"}"#),
        )
        .expect("Reklawdbox-owned store should remain writable");
    }

    let retained_mutex = server
        .context
        .database
        .internal_db
        .get()
        .expect("successful store connection should be cached");
    let retained_mutex_address = std::ptr::from_ref(retained_mutex);
    let conn = server
        .cache_store_conn()
        .expect("later store access should reuse the retained connection");
    assert_eq!(
        std::ptr::from_ref(
            server
                .context
                .database
                .internal_db
                .get()
                .expect("cached store connection should remain present")
        ),
        retained_mutex_address
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM enrichment_cache WHERE provider = 'retry-test'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .expect("retained store connection should contain the allowed write"),
        1
    );
    assert_eq!(
        fs::read(&master_path).expect("master marker should remain readable"),
        master_marker
    );
    assert!(server.context.database.db.get().is_none());
    assert!(server.context.database.effective_db_path.get().is_none());
}
