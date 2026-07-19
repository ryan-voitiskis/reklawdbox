use super::super::test_support::{PrivateFixtureError, PrivateRekordboxFixture};
use super::super::*;
use super::support::seed_test_db;
use rusqlite::{Connection, OpenFlags};

#[test]
fn rekordbox_connection_configured_missing_db_fails_closed_without_using_default() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db");
    let fallback_called = std::cell::Cell::new(false);

    let resolved = resolve_db_path_from(Some(missing.into_os_string()), || {
        fallback_called.set(true);
        Some("/real/library/master.db".to_string())
    });

    assert!(resolved.is_none());
    assert!(!fallback_called.get());
}

#[test]
fn rekordbox_connection_configured_existing_db_wins_over_default() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let configured = file.path().to_path_buf();

    let resolved = resolve_db_path_from(Some(configured.clone().into_os_string()), || {
        Some("/real/library/master.db".to_string())
    });

    assert_eq!(resolved.as_deref(), configured.to_str());
}

fn copy_archive_database(
    archive: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), PrivateFixtureError> {
    std::fs::copy(archive, destination.join("master.db"))
        .map(|_| ())
        .map_err(|error| PrivateFixtureError::Extraction {
            diagnostic: error.kind().to_string(),
        })
}

fn write_encrypted_identity_database(path: &std::path::Path, identity: &str) {
    write_encrypted_identity_database_with_key(path, REKORDBOX_SQLCIPHER_KEY, identity);
}

fn write_encrypted_identity_database_with_key(path: &std::path::Path, key: &str, identity: &str) {
    let conn = Connection::open(path).unwrap();
    conn.execute_batch(&format!(
        "PRAGMA key = '{key}'; \
         CREATE TABLE fixture_identity (value TEXT NOT NULL);"
    ))
    .unwrap();
    conn.execute(
        "INSERT INTO fixture_identity (value) VALUES (?1)",
        [identity],
    )
    .unwrap();
}

#[test]
fn rekordbox_connection_private_fixture_uses_unique_owned_roots() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive = source_dir.path().join("fixture-a.db");
    write_encrypted_identity_database(&archive, "A");

    let first =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();
    let second =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();

    assert_ne!(first.root(), second.root());
    assert_ne!(first.database_path(), second.database_path());
}

#[test]
fn rekordbox_connection_private_fixture_open_is_production_read_only() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive = source_dir.path().join("fixture.db");
    write_encrypted_identity_database(&archive, "read-only");
    let fixture =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();

    let conn = fixture.open().unwrap();
    assert!(
        conn.execute(
            "INSERT INTO fixture_identity (value) VALUES ('write blocked')",
            [],
        )
        .is_err(),
        "private fixture connections must use the production read-only adapter"
    );
}

#[test]
fn rekordbox_connection_private_fixture_reports_corrupt_database() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive = source_dir.path().join("corrupt.db");
    std::fs::write(&archive, b"not a SQLCipher database").unwrap();
    let fixture =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();

    assert!(matches!(fixture.open(), Err(PrivateFixtureError::Open(_))));
}

#[test]
fn rekordbox_connection_private_fixture_reports_wrong_key_database() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive = source_dir.path().join("wrong-key.db");
    write_encrypted_identity_database_with_key(&archive, "synthetic-wrong-key", "wrong-key");
    let fixture =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();

    assert!(matches!(fixture.open(), Err(PrivateFixtureError::Open(_))));
}

#[test]
fn rekordbox_connection_private_fixture_rejects_missing_database_with_sidecars() {
    let archive = tempfile::NamedTempFile::new().unwrap();
    let result = PrivateRekordboxFixture::from_archive_with(archive.path(), |_archive, root| {
        std::fs::write(root.join("master.db-wal"), b"synthetic sidecar").unwrap();
        std::fs::write(root.join("master.db-shm"), b"synthetic sidecar").unwrap();
        Ok(())
    });

    assert!(matches!(result, Err(PrivateFixtureError::MissingDatabase)));
}

#[test]
fn rekordbox_connection_private_fixture_cleans_root_after_connection_and_guard_drop() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive = source_dir.path().join("fixture.db");
    write_encrypted_identity_database(&archive, "cleanup");
    let fixture =
        PrivateRekordboxFixture::from_archive_with(&archive, copy_archive_database).unwrap();
    let root = fixture.root().to_path_buf();
    let conn = fixture.open().unwrap();

    assert!(root.is_dir());
    drop(conn);
    drop(fixture);
    assert!(!root.exists());
}

#[test]
fn rekordbox_connection_private_fixture_never_reuses_another_archive_extraction() {
    let source_dir = tempfile::tempdir().unwrap();
    let archive_a = source_dir.path().join("fixture-a.db");
    let archive_b = source_dir.path().join("fixture-b.db");
    write_encrypted_identity_database(&archive_a, "A");
    write_encrypted_identity_database(&archive_b, "B");

    let fixture_a =
        PrivateRekordboxFixture::from_archive_with(&archive_a, copy_archive_database).unwrap();
    let fixture_b =
        PrivateRekordboxFixture::from_archive_with(&archive_b, copy_archive_database).unwrap();
    let identity_a: String = fixture_a
        .open()
        .unwrap()
        .query_row("SELECT value FROM fixture_identity", [], |row| row.get(0))
        .unwrap();
    let identity_b: String = fixture_b
        .open()
        .unwrap()
        .query_row("SELECT value FROM fixture_identity", [], |row| row.get(0))
        .unwrap();

    assert_eq!(identity_a, "A");
    assert_eq!(identity_b, "B");
    assert_ne!(fixture_a.archive_identity(), fixture_b.archive_identity());
    assert_ne!(fixture_a.database_path(), fixture_b.database_path());
}

#[test]
fn rekordbox_connection_sanitized_sqlcipher_fixture_opens_read_only_through_production_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sanitized-master.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA key = '{REKORDBOX_SQLCIPHER_KEY}'"))
            .unwrap();
        seed_test_db(&conn);
    }

    let plain = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(
        plain
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row
                .get::<_, i64>(0))
            .is_err(),
        "fixture must be encrypted rather than plain SQLite"
    );

    let fixture = PrivateRekordboxFixture::from_archive_with(&path, copy_archive_database).unwrap();
    let conn = fixture
        .open()
        .expect("production SQLCipher open should succeed");
    let tracks = search_tracks(&conn, &SearchParams::default()).unwrap();
    assert_eq!(tracks.len(), 7);
    assert!(
        tracks
            .iter()
            .any(|track| track.id == "t1" && track.title == "Archangel")
    );
    assert!(
        conn.execute(
            "INSERT INTO djmdArtist (ID, Name) VALUES ('write', 'blocked')",
            [],
        )
        .is_err(),
        "production fixture connection must remain read-only"
    );
}
