use rusqlite::Connection;

pub(crate) const STORE_SCHEMA_VERSION: i32 = 9;

pub(crate) fn migrate(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS enrichment_cache (
            provider TEXT NOT NULL,
            query_artist TEXT NOT NULL,
            query_title TEXT NOT NULL,
            query_album TEXT NOT NULL DEFAULT '',
            match_quality TEXT,
            response_json TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (provider, query_artist, query_title, query_album)
        );
        CREATE TABLE IF NOT EXISTS audio_analysis_cache (
            file_path TEXT NOT NULL,
            analyzer TEXT NOT NULL,
            file_size INTEGER NOT NULL,
            file_mtime INTEGER NOT NULL,
            analysis_version TEXT NOT NULL,
            input_fingerprint TEXT NOT NULL DEFAULT '',
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
        );
        CREATE TABLE IF NOT EXISTS audit_files (
            path         TEXT PRIMARY KEY,
            last_audited TEXT NOT NULL,
            file_mtime   TEXT NOT NULL,
            file_size    INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS audit_issues (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            path        TEXT NOT NULL REFERENCES audit_files(path) ON DELETE CASCADE ON UPDATE CASCADE,
            issue_type  TEXT NOT NULL,
            detail      TEXT,
            status      TEXT NOT NULL DEFAULT 'open',
            resolution  TEXT,
            note        TEXT,
            created_at  TEXT NOT NULL,
            resolved_at TEXT,
            UNIQUE(path, issue_type)
        );
        CREATE INDEX IF NOT EXISTS idx_audit_issues_status ON audit_issues(status);
        CREATE INDEX IF NOT EXISTS idx_audit_issues_path ON audit_issues(path);
        CREATE TABLE IF NOT EXISTS timbral_norm_stats (
            dimension_index INTEGER PRIMARY KEY,
            mean REAL NOT NULL,
            stddev REAL NOT NULL,
            sample_count INTEGER NOT NULL,
            source_fingerprint TEXT NOT NULL DEFAULT '',
            analysis_version TEXT NOT NULL DEFAULT '',
            vector_schema_version TEXT NOT NULL DEFAULT '',
            computed_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS weight_presets (
            name TEXT NOT NULL,
            scorer_type TEXT NOT NULL,
            weights_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (name, scorer_type)
        );
        CREATE TABLE IF NOT EXISTS genre_audio_profiles (
            genre         TEXT NOT NULL,
            feature       TEXT NOT NULL,
            mean          REAL NOT NULL,
            stddev        REAL NOT NULL,
            fisher_weight REAL NOT NULL,
            n_verified    INTEGER NOT NULL DEFAULT 0,
            updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (genre, feature)
        );
        CREATE TABLE IF NOT EXISTS genre_timbral_centroids (
            genre         TEXT NOT NULL,
            centroid_type TEXT NOT NULL,
            values_json   TEXT NOT NULL,
            mean_dist     REAL,
            n_verified    INTEGER NOT NULL DEFAULT 0,
            updated_at    TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (genre, centroid_type)
        );
        CREATE TABLE IF NOT EXISTS genre_global_stats (
            feature       TEXT PRIMARY KEY,
            mean          REAL NOT NULL,
            stddev        REAL NOT NULL,
            updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
        );
        ",
    )?;
    migrate_enrichment_cache(conn)?;
    migrate_audio_analysis_cache(conn)?;
    migrate_timbral_norm_stats(conn)?;
    conn.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;
    Ok(())
}

fn migrate_audio_analysis_cache(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !table_has_column(conn, "audio_analysis_cache", "input_fingerprint")? {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "ALTER TABLE audio_analysis_cache
             ADD COLUMN input_fingerprint TEXT NOT NULL DEFAULT '';",
        )?;
        tx.commit()?;
    }
    Ok(())
}

fn migrate_enrichment_cache(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !table_has_column(conn, "enrichment_cache", "query_album")? {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "DROP TABLE IF EXISTS enrichment_cache_new;
             CREATE TABLE enrichment_cache_new (
                 provider TEXT NOT NULL,
                 query_artist TEXT NOT NULL,
                 query_title TEXT NOT NULL,
                 query_album TEXT NOT NULL DEFAULT '',
                 match_quality TEXT,
                 response_json TEXT,
                 created_at TEXT NOT NULL DEFAULT (datetime('now')),
                 PRIMARY KEY (provider, query_artist, query_title, query_album)
             );",
        )?;
        tx.execute(
            "INSERT INTO enrichment_cache_new (
                 provider,
                 query_artist,
                 query_title,
                 query_album,
                 match_quality,
                 response_json,
                 created_at
             )
             SELECT
                 provider,
                 query_artist,
                 query_title,
                 '',
                 match_quality,
                 response_json,
                 created_at
             FROM enrichment_cache",
            [],
        )?;
        tx.execute_batch(
            "DROP TABLE enrichment_cache;
             ALTER TABLE enrichment_cache_new RENAME TO enrichment_cache;",
        )?;
        tx.commit()?;
    }

    Ok(())
}

fn migrate_timbral_norm_stats(conn: &Connection) -> Result<(), rusqlite::Error> {
    let missing_source_fingerprint =
        !table_has_column(conn, "timbral_norm_stats", "source_fingerprint")?;
    let missing_analysis_version =
        !table_has_column(conn, "timbral_norm_stats", "analysis_version")?;
    let missing_vector_schema_version =
        !table_has_column(conn, "timbral_norm_stats", "vector_schema_version")?;

    if missing_source_fingerprint || missing_analysis_version || missing_vector_schema_version {
        let tx = conn.unchecked_transaction()?;
        if missing_source_fingerprint {
            tx.execute_batch(
                "ALTER TABLE timbral_norm_stats
                 ADD COLUMN source_fingerprint TEXT NOT NULL DEFAULT '';",
            )?;
        }
        if missing_analysis_version {
            tx.execute_batch(
                "ALTER TABLE timbral_norm_stats
                 ADD COLUMN analysis_version TEXT NOT NULL DEFAULT '';",
            )?;
        }
        if missing_vector_schema_version {
            tx.execute_batch(
                "ALTER TABLE timbral_norm_stats
                 ADD COLUMN vector_schema_version TEXT NOT NULL DEFAULT '';",
            )?;
        }
        tx.commit()?;
    }

    Ok(())
}

pub(crate) fn table_has_column(
    conn: &Connection,
    table_name: &str,
    column_name: &str,
) -> Result<bool, rusqlite::Error> {
    let pragma = format!("PRAGMA table_info({table_name})");
    let mut stmt = conn.prepare(&pragma)?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column_name {
            return Ok(true);
        }
    }
    Ok(false)
}
