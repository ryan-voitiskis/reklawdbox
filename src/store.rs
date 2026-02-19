use rusqlite::{Connection, OpenFlags, params};
use std::path::PathBuf;

/// Default path for the internal SQLite store.
pub fn default_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crate-dig")
        .join("internal.sqlite3")
}

/// Open (or create) the internal store at the given path.
pub fn open(path: &str) -> Result<Connection, rusqlite::Error> {
    let p = std::path::Path::new(path);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).ok();
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
    if version < 1 {
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
            PRAGMA user_version = 1;",
        )?;
    }
    Ok(())
}

/// Cached enrichment result.
pub struct CachedEnrichment {
    pub provider: String,
    pub query_artist: String,
    pub query_title: String,
    pub match_quality: Option<String>,
    pub response_json: Option<String>,
    pub created_at: String,
}

/// Read an enrichment cache entry.
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
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// Write an enrichment cache entry (upsert).
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

/// Cached audio analysis result.
pub struct CachedAudioAnalysis {
    pub file_path: String,
    pub analyzer: String,
    pub file_size: i64,
    pub file_mtime: i64,
    pub analysis_version: String,
    pub features_json: String,
    pub created_at: String,
}

/// Read an audio analysis cache entry.
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
        Some(Ok(entry)) => Ok(Some(entry)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

/// Write an audio analysis cache entry (upsert).
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
        assert_eq!(version, 1);

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(tables.contains(&"enrichment_cache".to_string()));
        assert!(tables.contains(&"audio_analysis_cache".to_string()));
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
        assert_eq!(version, 1);
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
}
