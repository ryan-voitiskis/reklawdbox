use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OpenFlags, params};

use crate::types::{
    FileKind, GenreCount, KeyCount, LibraryStats, Playlist, Track, rating_to_stars,
};

/// The universal Rekordbox 6/7 SQLCipher key (publicly known, same for all installations).
const REKORDBOX_SQLCIPHER_KEY: &str = "402fd482c38817c35ffa8ffb8c7d93143b749e7d315df7a81732a1ff43608497";

pub fn open(path: &str) -> Result<Connection, rusqlite::Error> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // Key is passed as a passphrase — SQLCipher derives the encryption key via PBKDF2.
    conn.execute_batch(&format!("PRAGMA key = '{REKORDBOX_SQLCIPHER_KEY}'"))?;
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_test() -> Connection {
    Connection::open_in_memory().unwrap()
}

/// Base SELECT for track queries — joins all lookup tables.
pub(crate) const TRACK_SELECT: &str = "
SELECT
    c.ID,
    COALESCE(c.Title, '') AS Title,
    COALESCE(a.Name, '') AS ArtistName,
    COALESCE(al.Name, '') AS AlbumName,
    COALESCE(g.Name, '') AS GenreName,
    COALESCE(c.BPM, 0) AS BPM,
    COALESCE(k.ScaleName, '') AS KeyName,
    COALESCE(c.Rating, 0) AS Rating,
    COALESCE(c.Commnt, '') AS Comments,
    COALESCE(col.Commnt, '') AS ColorName,
    COALESCE(col.ColorCode, 0) AS ColorCode,
    COALESCE(l.Name, '') AS LabelName,
    COALESCE(ra.Name, '') AS RemixerName,
    COALESCE(c.ReleaseYear, 0) AS ReleaseYear,
    COALESCE(c.Length, 0) AS Length,
    COALESCE(c.FolderPath, '') AS FolderPath,
    COALESCE(c.DJPlayCount, '0') AS DJPlayCount,
    COALESCE(c.BitRate, 0) AS BitRate,
    COALESCE(c.SampleRate, 0) AS SampleRate,
    COALESCE(c.FileType, 0) AS FileType,
    COALESCE(c.created_at, '') AS DateAdded
FROM djmdContent c
LEFT JOIN djmdArtist a ON c.ArtistID = a.ID
LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID
LEFT JOIN djmdGenre g ON c.GenreID = g.ID
LEFT JOIN djmdKey k ON c.KeyID = k.ID
LEFT JOIN djmdLabel l ON c.LabelID = l.ID
LEFT JOIN djmdColor col ON c.ColorID = col.ID
LEFT JOIN djmdArtist ra ON c.RemixerID = ra.ID
";

pub(crate) fn row_to_track(row: &rusqlite::Row) -> Result<Track, rusqlite::Error> {
    let bpm_raw: i32 = row.get("BPM")?;
    let rating_raw: i32 = row.get("Rating")?;
    // DJPlayCount is stored as integer in real DB but as text in some versions.
    let play_count: i32 = match row.get::<_, i32>("DJPlayCount") {
        Ok(n) => n,
        Err(_) => {
            let raw = row.get::<_, String>("DJPlayCount").unwrap_or_default();
            match raw.parse() {
                Ok(n) => n,
                Err(_) => {
                    if !raw.is_empty() {
                        tracing::debug!(
                            "DJPlayCount parse failed for value {raw:?}, defaulting to 0"
                        );
                    }
                    0
                }
            }
        }
    };

    let file_type_raw: i32 = row.get("FileType")?;

    Ok(Track {
        id: row.get("ID")?,
        title: row.get::<_, String>("Title")?.trim().to_string(),
        artist: row.get::<_, String>("ArtistName")?.trim().to_string(),
        album: row.get::<_, String>("AlbumName")?.trim().to_string(),
        genre: row.get::<_, String>("GenreName")?.trim().to_string(),
        bpm: bpm_raw as f64 / 100.0,
        key: row.get::<_, String>("KeyName")?.trim().to_string(),
        rating: decode_rating_stars(rating_raw),
        comments: row.get::<_, String>("Comments")?.trim().to_string(),
        color: row.get::<_, String>("ColorName")?.trim().to_string(),
        color_code: row.get("ColorCode")?,
        label: row.get::<_, String>("LabelName")?.trim().to_string(),
        remixer: row.get::<_, String>("RemixerName")?.trim().to_string(),
        year: row.get("ReleaseYear")?,
        length: row.get("Length")?,
        file_path: row.get("FolderPath")?,
        play_count,
        bit_rate: row.get("BitRate")?,
        sample_rate: row.get("SampleRate")?,
        file_kind: FileKind::from_raw(file_type_raw),
        date_added: row.get::<_, String>("DateAdded")?.trim().to_string(),
        position: None,
    })
}

fn decode_rating_stars(rating_raw: i32) -> u8 {
    match rating_raw {
        i32::MIN..=-1 => 0,
        0..=5 => rating_raw as u8,
        _ => rating_to_stars(rating_raw as u16),
    }
}

/// Rekordbox sampler files live under this path fragment across installations.
pub const SAMPLER_PATH_FRAGMENT: &str = "/rekordbox/Sampler/";

fn sampler_path_like_pattern() -> String {
    format!("%{}%", escape_like(SAMPLER_PATH_FRAGMENT))
}

#[cfg(test)]
fn is_sampler_path(path: &str) -> bool {
    path.contains(SAMPLER_PATH_FRAGMENT)
}

#[derive(Default)]
pub struct SearchParams {
    pub query: Option<String>,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub rating_min: Option<u8>,
    pub bpm_min: Option<f64>,
    pub bpm_max: Option<f64>,
    pub key: Option<String>,
    pub playlist: Option<String>,
    pub has_genre: Option<bool>,
    pub label: Option<String>,
    pub path: Option<String>,
    pub path_prefix: Option<String>,
    pub added_after: Option<String>,
    pub added_before: Option<String>,
    pub exclude_samples: bool,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

fn apply_search_filters(
    sql: &mut String,
    params: &SearchParams,
    bind_values: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    if let Some(ref query_text) = params.query {
        let bind_index = bind_values.len() + 1;
        sql.push_str(&format!(
            " AND (c.Title LIKE ?{bind_index} ESCAPE '\\' OR a.Name LIKE ?{bind_index} ESCAPE '\\')"
        ));
        bind_values.push(Box::new(format!("%{}%", escape_like(query_text))));
    }

    if let Some(ref artist) = params.artist {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND a.Name LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(format!("%{}%", escape_like(artist))));
    }

    if let Some(ref genre) = params.genre {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND g.Name LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(format!("%{}%", escape_like(genre))));
    }

    if let Some(rating_min) = params.rating_min {
        let idx_encoded = bind_values.len() + 1;
        let idx_star_scale = idx_encoded + 1;
        sql.push_str(&format!(
            " AND (c.Rating >= ?{idx_encoded} OR (c.Rating BETWEEN 0 AND 5 AND c.Rating >= ?{idx_star_scale}))"
        ));
        let min_rating = crate::types::stars_to_rating(rating_min) as i32;
        bind_values.push(Box::new(min_rating));
        bind_values.push(Box::new(rating_min as i32));
    }

    if let Some(bpm_min) = params.bpm_min {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.BPM >= ?{idx}"));
        bind_values.push(Box::new((bpm_min * 100.0) as i32));
    }

    if let Some(bpm_max) = params.bpm_max {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.BPM <= ?{idx}"));
        bind_values.push(Box::new((bpm_max * 100.0) as i32));
    }

    if let Some(ref key) = params.key {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND k.ScaleName = ?{idx}"));
        bind_values.push(Box::new(key.clone()));
    }

    if let Some(has_genre) = params.has_genre {
        if has_genre {
            sql.push_str(" AND g.Name IS NOT NULL AND g.Name != ''");
        } else {
            sql.push_str(" AND (g.Name IS NULL OR g.Name = '')");
        }
    }

    if let Some(ref label) = params.label {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND l.Name LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(format!("%{}%", escape_like(label))));
    }

    if let Some(ref path) = params.path {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.FolderPath LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(format!("%{}%", escape_like(path))));
    }

    if let Some(ref prefix) = params.path_prefix {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.FolderPath LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(format!("{}%", escape_like(prefix))));
    }

    if let Some(ref added_after) = params.added_after {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.created_at >= ?{idx}"));
        bind_values.push(Box::new(added_after.clone()));
    }

    if let Some(ref added_before) = params.added_before {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.created_at <= ?{idx}"));
        bind_values.push(Box::new(added_before.clone()));
    }

    if params.exclude_samples {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(" AND c.FolderPath NOT LIKE ?{idx} ESCAPE '\\'"));
        bind_values.push(Box::new(sampler_path_like_pattern()));
    }

    // Playlist filter: join through djmdSongPlaylist
    if let Some(ref playlist_id) = params.playlist {
        let idx = bind_values.len() + 1;
        sql.push_str(&format!(
            " AND c.ID IN (SELECT sp.ContentID FROM djmdSongPlaylist sp WHERE sp.PlaylistID = ?{idx})"
        ));
        bind_values.push(Box::new(playlist_id.clone()));
    }
}

fn search_tracks_with_limit_policy(
    conn: &Connection,
    params: &SearchParams,
    default_limit: Option<u32>,
    max_limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    let mut sql = format!("{TRACK_SELECT} WHERE c.rb_local_deleted = 0");
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
    apply_search_filters(&mut sql, params, &mut bind_values);

    sql.push_str(" ORDER BY c.Title");
    if let Some(mut limit) = params.limit.or(default_limit) {
        if let Some(max_limit) = max_limit {
            limit = limit.min(max_limit);
        }
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    if let Some(offset) = params.offset {
        // SQLite requires LIMIT before OFFSET — use LIMIT -1 (unlimited) if needed
        if !sql.contains("LIMIT") {
            sql.push_str(" LIMIT -1");
        }
        sql.push_str(&format!(" OFFSET {offset}"));
    }

    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(bind_params.as_slice(), row_to_track)?;
    rows.collect()
}

pub(crate) fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

pub fn search_tracks(
    conn: &Connection,
    params: &SearchParams,
) -> Result<Vec<Track>, rusqlite::Error> {
    search_tracks_with_limit_policy(conn, params, Some(50), Some(200))
}

/// Unbounded variant of `search_tracks` with no safety limit. Intended for `cache_coverage` only.
pub fn search_tracks_unbounded(
    conn: &Connection,
    params: &SearchParams,
) -> Result<Vec<Track>, rusqlite::Error> {
    search_tracks_with_limit_policy(conn, params, None, None)
}

fn get_playlist_tracks_with_limit_policy(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
    default_limit: Option<u32>,
    max_limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    let resolved_limit = limit.or(default_limit).map(|value| {
        if let Some(max_limit) = max_limit {
            value.min(max_limit)
        } else {
            value
        }
    });

    // Insert sp.TrackNo column before the FROM clause in TRACK_SELECT
    let base_sql = TRACK_SELECT.replace(
        "\nFROM djmdContent c",
        ",\n    sp.TrackNo AS Position\nFROM djmdContent c",
    );
    debug_assert_ne!(base_sql, TRACK_SELECT, "TRACK_SELECT Position injection failed");
    let mut sql = format!(
        "{base_sql}
         INNER JOIN djmdSongPlaylist sp ON sp.ContentID = c.ID
         WHERE sp.PlaylistID = ?1 AND c.rb_local_deleted = 0
         ORDER BY sp.TrackNo"
    );
    if let Some(limit) = resolved_limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![playlist_id], row_to_playlist_track)?;
    rows.collect()
}

pub fn get_track(conn: &Connection, track_id: &str) -> Result<Option<Track>, rusqlite::Error> {
    let sql = format!("{TRACK_SELECT} WHERE c.ID = ?1 AND c.rb_local_deleted = 0");
    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query_map(params![track_id], row_to_track)?;
    match rows.next() {
        Some(Ok(track)) => Ok(Some(track)),
        Some(Err(e)) => Err(e),
        None => Ok(None),
    }
}

pub fn get_playlists(conn: &Connection) -> Result<Vec<Playlist>, rusqlite::Error> {
    let sql = "
        SELECT
            p.ID,
            COALESCE(p.Name, '') AS Name,
            COALESCE(p.ParentID, '') AS ParentID,
            COALESCE(p.Attribute, 0) AS Attribute,
            (
                SELECT COUNT(*)
                FROM djmdSongPlaylist sp
                INNER JOIN djmdContent c ON c.ID = sp.ContentID
                WHERE sp.PlaylistID = p.ID AND c.rb_local_deleted = 0
            ) AS TrackCount
        FROM djmdPlaylist p
        WHERE p.rb_local_deleted = 0 AND p.ID != '200000'
        ORDER BY p.Seq
    ";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        let playlist_attribute: i32 = row.get("Attribute")?;
        Ok(Playlist {
            id: row.get("ID")?,
            name: row.get::<_, String>("Name")?.trim().to_string(),
            track_count: row.get("TrackCount")?,
            parent_id: row.get("ParentID")?,
            is_folder: playlist_attribute == 1,
            is_smart: playlist_attribute == 4,
        })
    })?;
    rows.collect()
}

fn row_to_playlist_track(row: &rusqlite::Row) -> Result<Track, rusqlite::Error> {
    let mut track = row_to_track(row)?;
    track.position = Some(row.get::<_, u32>("Position")?);
    Ok(track)
}

pub fn get_playlist_tracks(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, Some(200), Some(200))
}

/// Unbounded variant of `get_playlist_tracks` with no safety limit. Intended for `cache_coverage` only.
pub fn get_playlist_tracks_unbounded(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, None, None)
}

pub fn get_library_stats(conn: &Connection) -> Result<LibraryStats, rusqlite::Error> {
    get_library_stats_filtered(conn, true)
}

pub fn get_library_stats_filtered(
    conn: &Connection,
    exclude_samples: bool,
) -> Result<LibraryStats, rusqlite::Error> {
    let sampler_pattern = sampler_path_like_pattern();
    let sample_filter = if exclude_samples {
        format!(" AND FolderPath NOT LIKE '{sampler_pattern}' ESCAPE '\\'")
    } else {
        String::new()
    };
    let content_sample_filter = if exclude_samples {
        format!(" AND c.FolderPath NOT LIKE '{sampler_pattern}' ESCAPE '\\'")
    } else {
        String::new()
    };

    let total_tracks: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM djmdContent WHERE rb_local_deleted = 0{sample_filter}"),
        [],
        |row| row.get(0),
    )?;

    let avg_bpm: f64 = conn
        .query_row(
            &format!("SELECT COALESCE(AVG(BPM), 0) FROM djmdContent WHERE rb_local_deleted = 0 AND BPM > 0{sample_filter}"),
            [],
            |row| row.get(0),
        )
        .map(|v: f64| v / 100.0)?;

    let rated_count: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM djmdContent WHERE rb_local_deleted = 0 AND Rating > 0{sample_filter}"),
        [],
        |row| row.get(0),
    )?;

    let playlist_count: i32 = conn.query_row(
        "SELECT COUNT(*) FROM djmdPlaylist WHERE rb_local_deleted = 0 AND Attribute != 1",
        [],
        |row| row.get(0),
    )?;

    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(g.Name, '(none)') AS GenreName, COUNT(*) AS cnt
         FROM djmdContent c
         LEFT JOIN djmdGenre g ON c.GenreID = g.ID
         WHERE c.rb_local_deleted = 0{content_sample_filter}
         GROUP BY g.Name
         ORDER BY cnt DESC"
    ))?;
    let genres: Vec<GenreCount> = stmt
        .query_map([], |row| {
            Ok(GenreCount {
                name: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    let mut stmt = conn.prepare(&format!(
        "SELECT COALESCE(k.ScaleName, '(none)') AS KeyName, COUNT(*) AS cnt
         FROM djmdContent c
         LEFT JOIN djmdKey k ON c.KeyID = k.ID
         WHERE c.rb_local_deleted = 0{content_sample_filter}
         GROUP BY k.ScaleName
         ORDER BY cnt DESC"
    ))?;
    let key_distribution: Vec<KeyCount> = stmt
        .query_map([], |row| {
            Ok(KeyCount {
                name: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    Ok(LibraryStats {
        total_tracks,
        genres,
        playlist_count,
        rated_count,
        unrated_count: total_tracks - rated_count,
        avg_bpm,
        key_distribution,
    })
}

pub fn get_tracks_by_exact_genre(
    conn: &Connection,
    genre_name: &str,
    exclude_samples: bool,
) -> Result<Vec<Track>, rusqlite::Error> {
    let mut sql = format!("{TRACK_SELECT} WHERE c.rb_local_deleted = 0 AND g.Name = ?1");
    if exclude_samples {
        sql.push_str(" AND c.FolderPath NOT LIKE ?2 ESCAPE '\\'");
    }
    sql.push_str(" ORDER BY c.Title");
    let mut stmt = conn.prepare(&sql)?;
    let rows = if exclude_samples {
        stmt.query_map(
            params![genre_name, sampler_path_like_pattern()],
            row_to_track,
        )?
    } else {
        stmt.query_map(params![genre_name], row_to_track)?
    };
    rows.collect()
}

pub fn get_tracks_by_ids(conn: &Connection, ids: &[String]) -> Result<Vec<Track>, rusqlite::Error> {
    if ids.is_empty() {
        return Ok(vec![]);
    }
    // Keep well below common SQLite variable limits (e.g. 999) to avoid prepare failures.
    const MAX_BIND_VARS_PER_QUERY: usize = 900;

    let mut tracks_by_id: HashMap<String, Track> = HashMap::new();
    for chunk in ids.chunks(MAX_BIND_VARS_PER_QUERY) {
        let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "{TRACK_SELECT} WHERE c.ID IN ({}) AND c.rb_local_deleted = 0",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(refs.as_slice(), row_to_track)?;
        for track in rows.collect::<Result<Vec<_>, _>>()? {
            tracks_by_id.entry(track.id.clone()).or_insert(track);
        }
    }

    // Preserve caller order and deduplicate.
    let mut seen = HashSet::new();
    let result = ids
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .filter_map(|id| tracks_by_id.remove(id))
        .collect();

    Ok(result)
}

pub fn default_db_path() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = format!("{home}/Library/Pioneer/rekordbox/master.db");
    if std::path::Path::new(&path).exists() {
        Some(path)
    } else {
        None
    }
}

pub fn resolve_db_path() -> Option<String> {
    if let Ok(path) = std::env::var("REKORDBOX_DB_PATH")
        && std::path::Path::new(&path).exists()
    {
        return Some(path);
    }
    default_db_path()
}

/// Open a real Rekordbox DB from the backup tarball for integration tests.
/// Returns None if the tarball is missing (allows graceful skip on CI).
#[cfg(test)]
pub(crate) fn open_real_db() -> Option<Connection> {
    use std::path::Path;
    use std::process::Command;
    use std::sync::OnceLock;

    static EXTRACTED: OnceLock<bool> = OnceLock::new();

    let tarball = std::env::var("REKORDBOX_TEST_BACKUP").unwrap_or_else(|_| {
        let home = std::env::var("HOME").unwrap();
        format!("{home}/Library/Pioneer/rekordbox-backups/db_20260215_233936.tar.gz")
    });

    if !Path::new(&tarball).exists() {
        return None;
    }

    let dest = "/tmp/reklawdbox-test";
    let db_path = format!("{dest}/master.db");

    // Ensure extraction happens exactly once across all test threads
    let ok = EXTRACTED.get_or_init(|| {
        if Path::new(&db_path).exists() {
            return true;
        }
        std::fs::create_dir_all(dest).ok();
        let status = Command::new("tar")
            .args([
                "xzf",
                &tarball,
                "-C",
                dest,
                "master.db",
                "master.db-shm",
                "master.db-wal",
            ])
            .status();
        match status {
            Ok(s) => s.success(),
            Err(_) => false,
        }
    });

    if !ok {
        return None;
    }

    let conn =
        Connection::open(&db_path).unwrap_or_else(|e| panic!("failed to open {db_path}: {e}"));
    conn.execute_batch(&format!("PRAGMA key = '{REKORDBOX_SQLCIPHER_KEY}'"))
        .unwrap_or_else(|e| panic!("failed to set key: {e}"));
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))
        .unwrap_or_else(|e| panic!("key verification failed: {e}"));
    Some(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn create_test_db() -> Connection {
        let conn = open_test();
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
            CREATE TABLE djmdPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                Seq INTEGER,
                Name VARCHAR(255),
                Attribute INTEGER DEFAULT 0,
                ParentID VARCHAR(255) DEFAULT '',
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdSongPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                PlaylistID VARCHAR(255),
                ContentID VARCHAR(255),
                TrackNo INTEGER
            );

            -- Lookup data
            INSERT INTO djmdArtist (ID, Name) VALUES ('a1', 'Burial');
            INSERT INTO djmdArtist (ID, Name) VALUES ('a2', 'Actress');
            INSERT INTO djmdArtist (ID, Name) VALUES ('a3', 'Ricardo Villalobos');
            INSERT INTO djmdAlbum (ID, Name) VALUES ('al1', 'Untrue');
            INSERT INTO djmdAlbum (ID, Name) VALUES ('al2', 'R.I.P.');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g1', 'Dubstep');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'Techno');
            INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Minimal');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k1', 'Am');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Cm');
            INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Fm');
            INSERT INTO djmdLabel (ID, Name) VALUES ('l1', 'Hyperdub');
            INSERT INTO djmdLabel (ID, Name) VALUES ('l2', 'Ninja Tune');
            INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c1', 16711935, 'Rose');
            INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c2', 65280, 'Green');

            -- Tracks
            INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, ColorID, BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
            VALUES ('t1', 'Archangel', 'a1', 'al1', 'g1', 'k1', 'l1', 'c1', 13950, 204, 'iconic garage vocal', 2007, 240, '/Users/vz/Music/Burial/Untrue/01 Archangel.flac', '12', 1411, 44100, 5, '2023-01-15');
            INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, BPM, Rating, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
            VALUES ('t2', 'Endorphin', 'a1', 'al1', 'g1', 'k2', 'l1', 14000, 153, 2007, 300, '/Users/vz/Music/Burial/Untrue/02 Endorphin.flac', '5', 1411, 44100, 5, '2023-01-15');
            INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, BPM, Rating, ReleaseYear, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
            VALUES ('t3', 'R.I.P.', 'a2', 'al2', 'g2', 'k3', 12800, 102, 2012, 360, '/Users/vz/Music/Actress/R.I.P./01 R.I.P..flac', 1411, 44100, 5, '2023-02-20');
            INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
            VALUES ('t4', 'Dexter', 'a3', 'g3', 12500, 480, '/Users/vz/Music/Villalobos/Dexter.wav', 1411, 44100, 11, '2023-03-10');
            INSERT INTO djmdContent (ID, Title, ArtistID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
            VALUES ('t5', 'Unknown Track', 'a1', 0, 200, '/Users/vz/Music/unknown.mp3', 320, 44100, 1, '2023-04-01');
            INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
            VALUES ('t6', 'Loop Sample 01', 'a1', 'g2', 12000, 8, '/Users/alice/Music/rekordbox/Sampler/Loop/01.wav', 1411, 44100, 11, '2023-01-01');

            -- Playlists
            INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p1', 1, 'Deep Cuts', 0, 'root');
            INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p2', 2, 'Folders', 1, 'root');
            INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp1', 'p1', 't1', 1);
            INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp2', 'p1', 't3', 2);
            ",
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_search_all() {
        let conn = create_test_db();
        let params = SearchParams::default();
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 6); // includes sampler track
    }

    #[test]
    fn test_search_exclude_samples() {
        let conn = create_test_db();
        let params = SearchParams {
            exclude_samples: true,
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 5); // sampler track excluded
        assert!(!tracks.iter().any(|t| t.file_path.contains("Sampler")));
    }

    #[test]
    fn test_search_by_genre() {
        let conn = create_test_db();
        let params = SearchParams {
            genre: Some("Dubstep".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2); // Archangel + Endorphin
        assert!(tracks.iter().all(|t| t.genre == "Dubstep"));
    }

    #[test]
    fn test_search_by_bpm_range() {
        let conn = create_test_db();
        let params = SearchParams {
            bpm_min: Some(130.0),
            bpm_max: Some(145.0),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2); // 139.5 and 140.0
        assert!(tracks.iter().all(|t| t.bpm >= 130.0 && t.bpm <= 145.0));
    }

    #[test]
    fn test_search_has_no_genre() {
        let conn = create_test_db();
        let params = SearchParams {
            has_genre: Some(false),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 1); // Unknown Track has no genre
        assert_eq!(tracks[0].title, "Unknown Track");
    }

    #[test]
    fn test_search_by_rating() {
        let conn = create_test_db();
        let params = SearchParams {
            rating_min: Some(3),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2); // Archangel (4 stars) + Endorphin (3 stars)
    }

    #[test]
    fn test_search_by_rating_supports_star_scale_storage() {
        let conn = create_test_db();
        conn.execute("UPDATE djmdContent SET Rating = 5 WHERE ID = 't4'", [])
            .expect("fixture rating update should succeed");

        let params = SearchParams {
            rating_min: Some(5),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).expect("rating filter should succeed");
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, "t4");
        assert_eq!(tracks[0].rating, 5);
    }

    #[test]
    fn test_search_by_key() {
        let conn = create_test_db();
        let params = SearchParams {
            key: Some("Am".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "Archangel");
    }

    #[test]
    fn test_search_by_playlist() {
        let conn = create_test_db();
        let params = SearchParams {
            playlist: Some("p1".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2); // Archangel + R.I.P.
    }

    #[test]
    fn test_search_by_path_substring() {
        let conn = create_test_db();
        // Substring: "Burial" matches t1 and t2 (anywhere in path)
        let params = SearchParams {
            path: Some("Burial".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks.iter().all(|t| t.file_path.contains("Burial")));
    }

    #[test]
    fn test_search_by_path_prefix() {
        let conn = create_test_db();
        // Prefix: scopes to /Users/vz/Music/Burial/ — matches t1 and t2
        let params = SearchParams {
            path_prefix: Some("/Users/vz/Music/Burial/".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 2);
        assert!(tracks
            .iter()
            .all(|t| t.file_path.starts_with("/Users/vz/Music/Burial/")));
    }

    #[test]
    fn test_path_prefix_excludes_substring_matches() {
        let conn = create_test_db();
        // "Music" appears in all paths as a substring but none start with it.
        let params = SearchParams {
            path_prefix: Some("Music".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 0);

        // Same term with substring match finds all 6 tracks.
        let params = SearchParams {
            path: Some("Music".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 6);
    }

    #[test]
    fn test_path_prefix_scopes_to_user_root() {
        let conn = create_test_db();
        // /Users/vz/ matches t1-t5 but not t6 (/Users/alice/)
        let params = SearchParams {
            path_prefix: Some("/Users/vz/".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 5);
        assert!(tracks
            .iter()
            .all(|t| t.file_path.starts_with("/Users/vz/")));
    }

    #[test]
    fn test_path_prefix_escapes_like_chars() {
        let conn = create_test_db();
        // A prefix containing LIKE wildcards should be escaped, not interpreted.
        let params = SearchParams {
            path_prefix: Some("/Users/%/Music".to_string()),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert_eq!(tracks.len(), 0); // literal "%" doesn't appear in any path
    }

    #[test]
    fn test_get_track() {
        let conn = create_test_db();
        let track = get_track(&conn, "t1").unwrap().unwrap();
        assert_eq!(track.title, "Archangel");
        assert_eq!(track.artist, "Burial");
        assert_eq!(track.genre, "Dubstep");
        assert_eq!(track.bpm, 139.5);
        assert_eq!(track.rating, 4);
        assert_eq!(track.comments, "iconic garage vocal");
        assert_eq!(track.label, "Hyperdub");
        assert_eq!(track.year, 2007);
        assert_eq!(track.file_kind, FileKind::Flac);
        assert_eq!(track.position, None);
    }

    #[test]
    fn test_get_track_not_found() {
        let conn = create_test_db();
        let track = get_track(&conn, "nonexistent").unwrap();
        assert!(track.is_none());
    }

    #[test]
    fn test_get_playlists() {
        let conn = create_test_db();
        let playlists = get_playlists(&conn).unwrap();
        assert_eq!(playlists.len(), 2);
        let deep_cuts = playlists.iter().find(|p| p.name == "Deep Cuts").unwrap();
        assert_eq!(deep_cuts.track_count, 2);
        assert!(!deep_cuts.is_folder);
        let folders = playlists.iter().find(|p| p.name == "Folders").unwrap();
        assert!(folders.is_folder);
    }

    #[test]
    fn test_get_playlists_track_count_excludes_deleted_tracks() {
        let conn = create_test_db();
        conn.execute(
            "UPDATE djmdContent SET rb_local_deleted = 1 WHERE ID = 't3'",
            [],
        )
        .expect("fixture update should succeed");

        let playlists = get_playlists(&conn).expect("playlist query should succeed");
        let deep_cuts = playlists
            .iter()
            .find(|p| p.id == "p1")
            .expect("fixture playlist should exist");
        assert_eq!(deep_cuts.track_count, 1);

        let tracks = get_playlist_tracks(&conn, "p1", None).expect("playlist tracks should load");
        assert_eq!(tracks.len(), 1);
    }

    #[test]
    fn test_get_playlist_tracks() {
        let conn = create_test_db();
        let tracks = get_playlist_tracks(&conn, "p1", None).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].title, "Archangel");
        assert_eq!(tracks[0].position, Some(1));
        assert_eq!(tracks[1].title, "R.I.P.");
        assert_eq!(tracks[1].position, Some(2));

        assert_eq!(tracks[0].file_kind, FileKind::Flac);
    }

    #[test]
    fn test_library_stats() {
        let conn = create_test_db();
        // Default: excludes samples
        let stats = get_library_stats(&conn).unwrap();
        assert_eq!(stats.total_tracks, 5); // sampler excluded
        assert_eq!(stats.rated_count, 3);
        assert_eq!(stats.unrated_count, 2);
        assert_eq!(stats.playlist_count, 1); // only non-folder playlists
        assert!(stats.avg_bpm > 0.0);
        assert!(!stats.genres.is_empty());
        assert!(!stats.key_distribution.is_empty());

        // With samples included
        let stats_all = get_library_stats_filtered(&conn, false).unwrap();
        assert_eq!(stats_all.total_tracks, 6); // includes sampler
    }

    #[test]
    fn test_get_tracks_by_exact_genre() {
        let conn = create_test_db();
        let tracks = get_tracks_by_exact_genre(&conn, "Dubstep", false).unwrap();
        assert_eq!(tracks.len(), 2); // Archangel + Endorphin
        assert!(tracks.iter().all(|t| t.genre == "Dubstep"));

        let tracks = get_tracks_by_exact_genre(&conn, "Techno", true).unwrap();
        assert_eq!(tracks.len(), 1); // R.I.P. only (sampler excluded)
        assert_eq!(tracks[0].title, "R.I.P.");

        let tracks = get_tracks_by_exact_genre(&conn, "Techno", false).unwrap();
        assert_eq!(tracks.len(), 2); // R.I.P. + Loop Sample 01
    }

    #[test]
    fn test_get_tracks_by_ids() {
        let conn = create_test_db();
        // Request t3 before t1 — verify caller order is preserved.
        let tracks = get_tracks_by_ids(&conn, &["t3".to_string(), "t1".to_string()]).unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].id, "t3");
        assert_eq!(tracks[1].id, "t1");
    }

    #[test]
    fn test_get_tracks_by_ids_batches_large_input() {
        let conn = create_test_db();
        let ids: Vec<String> = (0..1200).map(|_| "t1".to_string()).collect();
        let tracks = get_tracks_by_ids(&conn, &ids).unwrap();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, "t1");
    }

    /// Load all tracks from the DB by paging with OFFSET.
    fn load_all_tracks(conn: &Connection) -> Vec<Track> {
        let mut all = Vec::new();
        let page_size = 200;
        let mut offset = 0;
        loop {
            let sql = format!(
                "{TRACK_SELECT} WHERE c.rb_local_deleted = 0 ORDER BY c.ID LIMIT {page_size} OFFSET {offset}"
            );
            let mut stmt = conn.prepare(&sql).unwrap();
            let batch: Vec<Track> = stmt
                .query_map([], row_to_track)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let count = batch.len();
            all.extend(batch);
            if count < page_size {
                break;
            }
            offset += page_size;
        }
        all
    }

    // ==================== Integration tests (real DB) ====================
    // Run with: cargo test -- --ignored

    #[test]
    #[ignore]
    fn test_real_db_opens() {
        let conn = open_real_db().expect("backup tarball not found — set REKORDBOX_TEST_BACKUP");
        let count: i32 = conn
            .query_row("SELECT count(*) FROM djmdContent", [], |r| r.get(0))
            .unwrap();
        assert!(count > 0, "DB opened but djmdContent is empty");
    }

    #[test]
    #[ignore]
    fn test_real_db_schema_tables() {
        let conn = open_real_db().expect("backup tarball not found");
        let required = [
            "djmdContent",
            "djmdArtist",
            "djmdAlbum",
            "djmdGenre",
            "djmdKey",
            "djmdLabel",
            "djmdColor",
            "djmdPlaylist",
            "djmdSongPlaylist",
        ];
        for table in required {
            let exists: bool = conn
                .query_row(
                    "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                    params![table],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(exists, "missing table: {table}");
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_schema_columns() {
        let conn = open_real_db().expect("backup tarball not found");
        // Verify critical columns exist by running a minimal query on each
        let checks = [
            (
                "djmdContent",
                "ID, Title, BPM, Commnt, Rating, FolderPath, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID, ReleaseYear, Length, DJPlayCount, BitRate, SampleRate, FileType, created_at, rb_local_deleted",
            ),
            ("djmdArtist", "ID, Name"),
            ("djmdAlbum", "ID, Name"),
            ("djmdGenre", "ID, Name"),
            ("djmdKey", "ID, ScaleName"),
            ("djmdLabel", "ID, Name"),
            ("djmdColor", "ID, ColorCode, Commnt"),
            (
                "djmdPlaylist",
                "ID, Name, Attribute, ParentID, Seq, rb_local_deleted",
            ),
            ("djmdSongPlaylist", "ID, PlaylistID, ContentID, TrackNo"),
        ];
        for (table, cols) in checks {
            let sql = format!("SELECT {cols} FROM {table} LIMIT 1");
            conn.prepare(&sql)
                .unwrap_or_else(|e| panic!("column check failed for {table}: {e}"));
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_track_count() {
        let conn = open_real_db().expect("backup tarball not found");
        let stats = get_library_stats(&conn).unwrap();
        assert!(
            stats.total_tracks > 2000,
            "expected >2000 tracks, got {}",
            stats.total_tracks
        );
        assert!(stats.avg_bpm > 0.0, "avg_bpm should be positive");
        assert!(
            stats.playlist_count > 0,
            "should have at least one playlist"
        );
    }

    #[test]
    #[ignore]
    fn test_real_db_search_returns_results() {
        let conn = open_real_db().expect("backup tarball not found");

        // Unfiltered search
        let params = SearchParams {
            limit: Some(10),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "unfiltered search returned no results");

        // BPM range search
        let params = SearchParams {
            bpm_min: Some(120.0),
            bpm_max: Some(130.0),
            limit: Some(50),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "BPM 120-130 range returned no results");
        for t in &tracks {
            assert!(
                t.bpm >= 120.0 && t.bpm <= 130.0,
                "track {} BPM {} outside range",
                t.title,
                t.bpm
            );
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_field_encoding() {
        let conn = open_real_db().expect("backup tarball not found");
        let params = SearchParams {
            limit: Some(200),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();

        for t in &tracks {
            // BPM: 0 (unanalyzed) or 30-300 reasonable range
            assert!(
                t.bpm == 0.0 || (t.bpm >= 30.0 && t.bpm <= 300.0),
                "track '{}' has unreasonable BPM: {}",
                t.title,
                t.bpm
            );
            // Rating: 0-5
            assert!(
                t.rating <= 5,
                "track '{}' has invalid rating: {}",
                t.title,
                t.rating
            );
            // Length: should be positive for real tracks
            assert!(
                t.length > 0,
                "track '{}' has non-positive length: {}",
                t.title,
                t.length
            );
            // file_path should be non-empty
            assert!(
                !t.file_path.is_empty(),
                "track '{}' has empty file_path",
                t.title
            );
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_null_handling() {
        let conn = open_real_db().expect("backup tarball not found");

        // has_genre=false should work without panic
        let params = SearchParams {
            has_genre: Some(false),
            limit: Some(50),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        for t in &tracks {
            assert!(
                t.genre.is_empty(),
                "track '{}' has genre '{}' but expected none",
                t.title,
                t.genre
            );
        }

        // has_genre=true
        let params = SearchParams {
            has_genre: Some(true),
            limit: Some(50),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        for t in &tracks {
            assert!(
                !t.genre.is_empty(),
                "track '{}' has empty genre but expected one",
                t.title
            );
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_unicode() {
        let conn = open_real_db().expect("backup tarball not found");
        let all = load_all_tracks(&conn);

        // Find any tracks with non-ASCII characters
        let unicode_tracks: Vec<_> = all
            .iter()
            .filter(|t| {
                !t.title.is_ascii() || !t.artist.is_ascii()
            })
            .collect();

        // Verify they survive serde round-trip
        for t in &unicode_tracks {
            let json = serde_json::to_string(t).unwrap();
            let back: crate::types::Track = serde_json::from_str(&json).unwrap();
            assert_eq!(t.title, back.title, "unicode title round-trip failed");
            assert_eq!(t.artist, back.artist, "unicode artist round-trip failed");
        }

        // Even if no unicode tracks exist, no panic means success
    }

    #[test]
    #[ignore]
    fn test_real_db_playlists() {
        let conn = open_real_db().expect("backup tarball not found");
        let playlists = get_playlists(&conn).unwrap();
        assert!(!playlists.is_empty(), "no playlists found");

        let has_folder = playlists.iter().any(|p| p.is_folder);
        let has_regular = playlists.iter().any(|p| !p.is_folder && !p.is_smart);
        // At least one type should exist
        assert!(
            has_folder || has_regular,
            "no folders or regular playlists found"
        );

        // For regular playlists with tracks, verify track loading
        for p in playlists
            .iter()
            .filter(|p| !p.is_folder && p.track_count > 0)
            .take(3)
        {
            let tracks = get_playlist_tracks(&conn, &p.id, Some(10)).unwrap();
            assert!(
                !tracks.is_empty(),
                "playlist '{}' claims {} tracks but returned none",
                p.name,
                p.track_count
            );
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_get_track_by_id() {
        let conn = open_real_db().expect("backup tarball not found");

        // Get a track via search, then fetch by ID
        let params = SearchParams {
            limit: Some(1),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty());

        let by_id = get_track(&conn, &tracks[0].id)
            .unwrap()
            .expect("track not found by ID");
        assert_eq!(tracks[0].id, by_id.id);
        assert_eq!(tracks[0].title, by_id.title);
        assert_eq!(tracks[0].artist, by_id.artist);
    }

    #[test]
    #[ignore]
    fn test_real_db_library_stats_consistency() {
        let conn = open_real_db().expect("backup tarball not found");
        let stats = get_library_stats(&conn).unwrap();

        // rated + unrated = total
        assert_eq!(
            stats.rated_count + stats.unrated_count,
            stats.total_tracks,
            "rated ({}) + unrated ({}) != total ({})",
            stats.rated_count,
            stats.unrated_count,
            stats.total_tracks
        );

        // genre counts sum to total
        let genre_sum: i32 = stats.genres.iter().map(|g| g.count).sum();
        assert_eq!(
            genre_sum, stats.total_tracks,
            "genre count sum ({genre_sum}) != total ({})",
            stats.total_tracks
        );

        // key counts sum to total
        let key_sum: i32 = stats.key_distribution.iter().map(|k| k.count).sum();
        assert_eq!(
            key_sum, stats.total_tracks,
            "key count sum ({key_sum}) != total ({})",
            stats.total_tracks
        );
    }

    #[test]
    #[ignore]
    fn test_real_db_rated_count_matches_rating_filtered_search() {
        let conn = open_real_db().expect("backup tarball not found");
        let stats = get_library_stats(&conn).unwrap();

        let params = SearchParams {
            rating_min: Some(1),
            exclude_samples: true,
            limit: Some(50),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).expect("rating-filtered search should succeed");

        if stats.rated_count > 0 {
            assert!(
                !tracks.is_empty(),
                "rated_count={} but rating_min=1 search returned no rows",
                stats.rated_count
            );
        }
        assert!(
            tracks.iter().all(|track| track.rating >= 1),
            "rating_min=1 search should only return tracks with star rating >= 1"
        );
    }

    #[test]
    #[ignore]
    fn test_real_db_all_tracks_load() {
        let conn = open_real_db().expect("backup tarball not found");
        let all = load_all_tracks(&conn);
        assert!(all.len() > 2000, "expected >2000 tracks, got {}", all.len());

        // Every track should have a non-empty ID
        for t in &all {
            assert!(!t.id.is_empty(), "track has empty ID");
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_special_characters_in_search() {
        let conn = open_real_db().expect("backup tarball not found");
        let nasty_inputs = [
            "'; DROP TABLE djmdContent; --",
            "\" OR 1=1 --",
            "track & bass",
            "100%",
            "it's a test",
            "null\0byte",
            "emoji 🎵",
            "",
        ];

        for input in nasty_inputs {
            let params = SearchParams {
                query: Some(input.to_string()),
                limit: Some(5),
                ..Default::default()
            };
            // Should not panic or error
            let result = search_tracks(&conn, &params);
            assert!(result.is_ok(), "search panicked on input: {input:?}");
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_sample_exclusion() {
        let conn = open_real_db().expect("backup tarball not found");

        let stats_filtered = get_library_stats(&conn).unwrap();
        let stats_all = get_library_stats_filtered(&conn, false).unwrap();

        // Filtered count should be less than or equal to unfiltered
        assert!(
            stats_filtered.total_tracks <= stats_all.total_tracks,
            "filtered {} > unfiltered {}",
            stats_filtered.total_tracks,
            stats_all.total_tracks
        );

        // Verify the difference is the sampler tracks
        let diff = stats_all.total_tracks - stats_filtered.total_tracks;
        eprintln!(
            "[integration] Sample exclusion: {} sampler tracks filtered out",
            diff
        );

        // Search with exclude_samples=true should not return sampler paths
        let params = SearchParams {
            exclude_samples: true,
            limit: Some(200),
            ..Default::default()
        };
        let tracks = search_tracks(&conn, &params).unwrap();
        for t in &tracks {
            assert!(
                !is_sampler_path(&t.file_path),
                "sampler track not excluded: {}",
                t.file_path
            );
        }
    }

    #[test]
    #[ignore]
    fn test_real_db_genre_normalization_coverage() {
        let conn = open_real_db().expect("backup tarball not found");
        let stats = get_library_stats(&conn).unwrap();

        let mut alias_count = 0;
        let mut canonical_count = 0;
        let mut unknown_genres = Vec::new();

        for gc in &stats.genres {
            if gc.name == "(none)" || gc.name.is_empty() {
                continue;
            }
            if crate::genre::canonical_genre_from_alias(&gc.name).is_some() {
                alias_count += gc.count;
            } else if crate::genre::is_known_genre(&gc.name) {
                canonical_count += gc.count;
            } else {
                unknown_genres.push(format!("{}: {} tracks", gc.name, gc.count));
            }
        }

        eprintln!("[integration] Canonical: {canonical_count} tracks, Alias: {alias_count} tracks");
        eprintln!("[integration] Unknown genres: {}", unknown_genres.len());
        for g in &unknown_genres {
            eprintln!("  {g}");
        }

        // Most tracks should be canonical or alias-able
        assert!(
            alias_count > 100,
            "expected >100 alias-able tracks, got {alias_count}"
        );
    }
}
