use rusqlite::{Connection, OpenFlags, ffi, params};
use std::path::PathBuf;

pub fn default_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reklawdbox")
        .join("internal.sqlite3")
}

pub fn open(path: &str) -> Result<Connection, rusqlite::Error> {
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            rusqlite::Error::SqliteFailure(
                ffi::Error::new(ffi::SQLITE_CANTOPEN),
                Some(format!(
                    "failed to create parent directory {} for {}: {}",
                    parent.display(),
                    p.display(),
                    err
                )),
            )
        })?;
    }
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )?;
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA synchronous = NORMAL;",
    )?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    let version: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS enrichment_cache (
            provider TEXT NOT NULL,
            query_artist TEXT NOT NULL,
            query_title TEXT NOT NULL,
            match_quality TEXT,
            response_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (provider, query_artist, query_title)
        );
        CREATE TABLE IF NOT EXISTS audio_analysis_cache (
            file_path TEXT NOT NULL,
            analyzer TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            file_mtime INTEGER NOT NULL,
            analysis_version TEXT NOT NULL,
            features_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (file_path, analyzer)
        );
        CREATE TABLE IF NOT EXISTS broker_discogs_session (
            broker_url TEXT PRIMARY KEY,
            session_token TEXT NOT NULL,
            expires_at INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    if version < 2 {
        conn.execute_batch("PRAGMA user_version = 2;")?;
    }
    Ok(())
}

pub struct CachedEnrichment {
    pub provider: String,
    pub query_artist: String,
    pub query_title: String,
    pub match_quality: Option<String>,
    pub response_json: Option<String>,
    pub created_at: String,
}

fn touch_cached_enrichment(entry: &CachedEnrichment) {
    let _ = (
        entry.provider.as_str(),
        entry.query_artist.as_str(),
        entry.query_title.as_str(),
    );
}

pub fn get_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
) -> Result<Option<CachedEnrichment>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT provider, query_artist, query_title, match_quality, response_json, created_at
         FROM enrichment_cache
         WHERE provider = ?1 AND query_artist = ?2 AND query_title = ?3",
    )?;
    let mut rows = stmt.query_map(params![provider, artist, title], |row| {
        Ok(CachedEnrichment {
            provider: row.get(0)?,
            query_artist: row.get(1)?,
            query_title: row.get(2)?,
            match_quality: row.get(3)?,
            response_json: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => {
            touch_cached_enrichment(&entry);
            Ok(Some(entry))
        }
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn set_enrichment(
    conn: &Connection,
    provider: &str,
    artist: &str,
    title: &str,
    match_quality: Option<&str>,
    response_json: Option<&str>,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO enrichment_cache (provider, query_artist, query_title, match_quality, response_json)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(provider, query_artist, query_title)
         DO UPDATE SET match_quality = ?4, response_json = ?5, created_at = datetime('now')",
        params![provider, artist, title, match_quality, response_json],
    )?;
    Ok(())
}

pub struct CachedAudioAnalysis {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analysis_version: String,
    pub features_json: String,
    pub created_at: String,
}

fn touch_cached_audio_analysis(entry: &CachedAudioAnalysis) {
    let _ = (
        entry.file_path.as_str(),
        entry.analyzer.as_str(),
        entry.analysis_version.as_str(),
        entry.created_at.as_str(),
    );
}

pub fn get_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
) -> Result<Option<CachedAudioAnalysis>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT file_path, analyzer, file_size, file_mtime, analysis_version, features_json, created_at
         FROM audio_analysis_cache
         WHERE file_path = ?1 AND analyzer = ?2",
    )?;
    let mut rows = stmt.query_map(params![file_path, analyzer], |row| {
        Ok(CachedAudioAnalysis {
            file_path: row.get(0)?,
            analyzer: row.get(1)?,
            file_size: row.get(2)?,
            file_mtime: row.get(3)?,
            analysis_version: row.get(4)?,
            features_json: row.get(5)?,
            created_at: row.get(6)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => {
            touch_cached_audio_analysis(&entry);
            Ok(Some(entry))
        }
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn set_audio_analysis(
    conn: &Connection,
    file_path: &str,
    analyzer: &str,
    file_size: i64,
    file_mtime: i64,
    analysis_version: &str,
    features_json: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO audio_analysis_cache (file_path, analyzer, file_size, file_mtime, analysis_version, features_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(file_path, analyzer)
         DO UPDATE SET file_size = ?3, file_mtime = ?4, analysis_version = ?5, features_json = ?6, created_at = datetime('now')",
        params![file_path, analyzer, file_size, file_mtime, analysis_version, features_json],
    )?;
    Ok(())
}

pub struct BrokerDiscogsSession {
    pub broker_url: String,
    pub session_token: String,
    pub expires_at: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn touch_broker_discogs_session(entry: &BrokerDiscogsSession) {
    let _ = (
        entry.broker_url.as_str(),
        entry.created_at.as_str(),
        entry.updated_at.as_str(),
    );
}

pub fn get_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<Option<BrokerDiscogsSession>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT broker_url, session_token, expires_at, created_at, updated_at
         FROM broker_discogs_session
         WHERE broker_url = ?1",
    )?;
    let mut rows = stmt.query_map(params![broker_url], |row| {
        Ok(BrokerDiscogsSession {
            broker_url: row.get(0)?,
            session_token: row.get(1)?,
            expires_at: row.get(2)?,
            created_at: row.get(3)?,
            updated_at: row.get(4)?,
        })
    })?;
    match rows.next() {
        Some(Ok(entry)) => {
            touch_broker_discogs_session(&entry);
            Ok(Some(entry))
        }
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn set_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
    session_token: &str,
    expires_at: i64,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "INSERT INTO broker_discogs_session (broker_url, session_token, expires_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(broker_url)
         DO UPDATE SET
            session_token = ?2,
            expires_at = ?3,
            updated_at = datetime('now')",
        params![broker_url, session_token, expires_at],
    )?;
    Ok(())
}

pub fn clear_broker_discogs_session(
    conn: &Connection,
    broker_url: &str,
) -> Result<(), rusqlite::Error> {
    conn.execute(
        "DELETE FROM broker_discogs_session WHERE broker_url = ?1",
        params![broker_url],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_temp_store() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let conn = open(path.to_str().unwrap()).unwrap();
        (dir, conn)
    }

    #[test]
    fn test_open_creates_schema() {
        let (_dir, conn) = open_temp_store();
        let version: i32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);

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
    }

    #[test]
    fn test_open_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite3");
        let path_str = path.to_str().unwrap();

        // Open twice — second open should not fail
        let conn1 = open(path_str).unwrap();
        drop(conn1);
        let conn2 = open(path_str).unwrap();
        let version: i32 = conn2
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
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
        conn.execute_batch("PRAGMA user_version = 2;").unwrap();
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
    }

    #[test]
    fn test_enrichment_cache_round_trip() {
        let (_dir, conn) = open_temp_store();

        // Write
        set_enrichment(
            &conn,
            "discogs",
            "burial",
            "archangel",
            Some("exact"),
            Some(r#"{"title":"Burial - Untrue","genres":["Electronic"]}"#),
        )
        .unwrap();

        // Read
        let entry = get_enrichment(&conn, "discogs", "burial", "archangel")
            .unwrap()
            .expect("should find cached entry");
        assert_eq!(entry.provider, "discogs");
        assert_eq!(entry.query_artist, "burial");
        assert_eq!(entry.query_title, "archangel");
        assert_eq!(entry.match_quality.as_deref(), Some("exact"));
        assert!(entry.response_json.unwrap().contains("Burial"));
        assert!(!entry.created_at.is_empty());
    }

    #[test]
    fn test_enrichment_cache_miss() {
        let (_dir, conn) = open_temp_store();
        let entry = get_enrichment(&conn, "discogs", "nobody", "nothing").unwrap();
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
            Some("fuzzy"),
            Some("old"),
        )
        .unwrap();
        set_enrichment(
            &conn,
            "discogs",
            "burial",
            "archangel",
            Some("exact"),
            Some("new"),
        )
        .unwrap();

        let entry = get_enrichment(&conn, "discogs", "burial", "archangel")
            .unwrap()
            .unwrap();
        assert_eq!(entry.match_quality.as_deref(), Some("exact"));
        assert_eq!(entry.response_json.as_deref(), Some("new"));
    }

    #[test]
    fn test_enrichment_cache_no_match() {
        let (_dir, conn) = open_temp_store();

        // Cache a "no match" result
        set_enrichment(&conn, "discogs", "nobody", "nothing", Some("none"), None).unwrap();

        let entry = get_enrichment(&conn, "discogs", "nobody", "nothing")
            .unwrap()
            .unwrap();
        assert_eq!(entry.match_quality.as_deref(), Some("none"));
        assert!(entry.response_json.is_none());
    }

    #[test]
    fn test_audio_analysis_cache_round_trip() {
        let (_dir, conn) = open_temp_store();

        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            12345678,
            1700000000,
            "1.0.0",
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

        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            100,
            200,
            "1.0.0",
            "old",
        )
        .unwrap();
        set_audio_analysis(
            &conn,
            "/music/track.flac",
            "stratum-dsp",
            100,
            300,
            "1.1.0",
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
    fn test_broker_discogs_session_round_trip() {
        let (_dir, conn) = open_temp_store();

        set_broker_discogs_session(
            &conn,
            "https://broker.example.com",
            "session-token-1",
            1_800_000_000,
        )
        .unwrap();

        let row = get_broker_discogs_session(&conn, "https://broker.example.com")
            .unwrap()
            .expect("broker session should exist");
        assert_eq!(row.broker_url, "https://broker.example.com");
        assert_eq!(row.session_token, "session-token-1");
        assert_eq!(row.expires_at, 1_800_000_000);
        assert!(!row.created_at.is_empty());
        assert!(!row.updated_at.is_empty());

        set_broker_discogs_session(
            &conn,
            "https://broker.example.com",
            "session-token-2",
            1_900_000_000,
        )
        .unwrap();
        let row = get_broker_discogs_session(&conn, "https://broker.example.com")
            .unwrap()
            .expect("broker session should still exist");
        assert_eq!(row.session_token, "session-token-2");
        assert_eq!(row.expires_at, 1_900_000_000);

        clear_broker_discogs_session(&conn, "https://broker.example.com").unwrap();
        let missing = get_broker_discogs_session(&conn, "https://broker.example.com").unwrap();
        assert!(missing.is_none());
    }
}
