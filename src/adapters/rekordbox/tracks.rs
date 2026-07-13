use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use rusqlite::{Connection, params};

use crate::domain::library::{
    FileKind, GenreCount, KeyCount, LibraryStats, Track, rating_to_stars,
};

/// Column list for track queries (SELECT clause without FROM).
macro_rules! track_columns {
    () => {
        "\
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
    COALESCE(c.created_at, '') AS DateAdded"
    };
}

/// FROM + JOIN clause for track queries.
macro_rules! track_joins {
    () => {
        "\
FROM djmdContent c
LEFT JOIN djmdArtist a ON c.ArtistID = a.ID
LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID
LEFT JOIN djmdGenre g ON c.GenreID = g.ID
LEFT JOIN djmdKey k ON c.KeyID = k.ID
LEFT JOIN djmdLabel l ON c.LabelID = l.ID
LEFT JOIN djmdColor col ON c.ColorID = col.ID
LEFT JOIN djmdArtist ra ON c.RemixerID = ra.ID"
    };
}

pub(crate) const TRACK_COLUMNS: &str = track_columns!();
pub(crate) const TRACK_JOINS: &str = track_joins!();

/// Base SELECT for track queries — joins all lookup tables.
pub(crate) const TRACK_SELECT: &str = concat!(track_columns!(), "\n", track_joins!(), "\n");

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
        played_at: None,
    })
}

pub(crate) fn decode_rating_stars(rating_raw: i32) -> u8 {
    match rating_raw {
        i32::MIN..=-1 => 0,
        0..=5 => rating_raw as u8,
        _ => rating_to_stars(rating_raw as u16),
    }
}

/// Rekordbox sampler files live under this path fragment across installations.
pub const SAMPLER_PATH_FRAGMENT: &str = "/rekordbox/Sampler/";

pub(super) fn sampler_path_like_pattern() -> String {
    format!("%{}%", escape_like(SAMPLER_PATH_FRAGMENT))
}

#[cfg(test)]
pub(crate) fn is_sampler_path(path: &str) -> bool {
    path.contains(SAMPLER_PATH_FRAGMENT)
}

pub fn validate_iso_date(s: &str, field: &str) -> Result<String, String> {
    let err = || format!("{field} must be a valid ISO date (YYYY-MM-DD), got: {s:?}");
    let b = s.as_bytes();
    if s.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(err());
    }
    let year: u32 = s[..4].parse().map_err(|_| err())?;
    let month: u32 = s[5..7].parse().map_err(|_| err())?;
    let day: u32 = s[8..10].parse().map_err(|_| err())?;
    if !(1..=12).contains(&month) || day < 1 {
        return Err(err());
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                29
            } else {
                28
            }
        }
        _ => unreachable!(),
    };
    if day > max_day {
        return Err(err());
    }
    Ok(s.to_string())
}

/// Advance a validated ISO date (YYYY-MM-DD) by one day.
/// Used to convert `<= date` into `< next_day` for datetime column comparisons.
pub(crate) fn next_day(date: &str) -> String {
    debug_assert!(
        validate_iso_date(date, "next_day").is_ok(),
        "next_day called with unvalidated date: {date}"
    );
    let year: u32 = date[..4].parse().unwrap();
    let month: u32 = date[5..7].parse().unwrap();
    let day: u32 = date[8..10].parse().unwrap();

    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) {
                29
            } else {
                28
            }
        }
        _ => unreachable!(),
    };

    if day < max_day {
        format!("{year:04}-{month:02}-{:02}", day + 1)
    } else if month < 12 {
        format!("{year:04}-{:02}-01", month + 1)
    } else {
        format!("{:04}-01-01", year + 1)
    }
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
    pub has_label: Option<bool>,
    pub year_zero: Option<bool>,
    pub label: Option<String>,
    pub path: Option<String>,
    pub path_prefix: Option<String>,
    pub added_after: Option<String>,
    pub added_before: Option<String>,
    pub exclude_samples: bool,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub(super) fn apply_search_filters(
    sql: &mut String,
    params: &SearchParams,
    bind_values: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    if let Some(ref query_text) = params.query {
        let bind_index = bind_values.len() + 1;
        write!(
            sql,
            " AND (c.Title LIKE ?{bind_index} ESCAPE '\\' OR a.Name LIKE ?{bind_index} ESCAPE '\\')"
        )
        .unwrap();
        bind_values.push(Box::new(format!("%{}%", escape_like(query_text))));
    }

    if let Some(ref artist) = params.artist {
        let idx = bind_values.len() + 1;
        write!(sql, " AND a.Name LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("%{}%", escape_like(artist))));
    }

    if let Some(ref genre) = params.genre {
        let idx = bind_values.len() + 1;
        write!(sql, " AND g.Name LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("%{}%", escape_like(genre))));
    }

    if let Some(rating_min) = params.rating_min {
        let idx_encoded = bind_values.len() + 1;
        let idx_star_scale = idx_encoded + 1;
        write!(sql,
            " AND (c.Rating >= ?{idx_encoded} OR (c.Rating BETWEEN 0 AND 5 AND c.Rating >= ?{idx_star_scale}))"
        ).unwrap();
        let min_rating = crate::domain::library::stars_to_rating(rating_min) as i32;
        bind_values.push(Box::new(min_rating));
        bind_values.push(Box::new(rating_min as i32));
    }

    if let Some(bpm_min) = params.bpm_min {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.BPM >= ?{idx}").unwrap();
        bind_values.push(Box::new((bpm_min * 100.0) as i32));
    }

    if let Some(bpm_max) = params.bpm_max {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.BPM <= ?{idx}").unwrap();
        bind_values.push(Box::new((bpm_max * 100.0) as i32));
    }

    if let Some(ref key) = params.key {
        let idx = bind_values.len() + 1;
        write!(sql, " AND k.ScaleName = ?{idx}").unwrap();
        bind_values.push(Box::new(key.clone()));
    }

    if let Some(has_genre) = params.has_genre {
        if has_genre {
            sql.push_str(" AND g.Name IS NOT NULL AND g.Name != ''");
        } else {
            sql.push_str(" AND (g.Name IS NULL OR g.Name = '')");
        }
    }

    if let Some(has_label) = params.has_label {
        if has_label {
            sql.push_str(" AND l.Name IS NOT NULL AND l.Name != ''");
        } else {
            sql.push_str(" AND (l.Name IS NULL OR l.Name = '')");
        }
    }

    if let Some(year_zero) = params.year_zero {
        if year_zero {
            sql.push_str(" AND COALESCE(c.ReleaseYear, 0) = 0");
        } else {
            sql.push_str(" AND COALESCE(c.ReleaseYear, 0) != 0");
        }
    }

    if let Some(ref label) = params.label {
        let idx = bind_values.len() + 1;
        write!(sql, " AND l.Name LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("%{}%", escape_like(label))));
    }

    if let Some(ref path) = params.path {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.FolderPath LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("%{}%", escape_like(path))));
    }

    if let Some(ref prefix) = params.path_prefix {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.FolderPath LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("{}%", escape_like(prefix))));
    }

    if let Some(ref added_after) = params.added_after {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.created_at >= ?{idx}").unwrap();
        bind_values.push(Box::new(added_after.clone()));
    }

    if let Some(ref added_before) = params.added_before {
        let idx = bind_values.len() + 1;
        // created_at stores full datetimes ("YYYY-MM-DD HH:MM:SS.SSS +00:00").
        // A bare date like "2026-01-31" sorts before any datetime on that day,
        // so we use < next-day instead of <= to include the whole boundary date.
        write!(sql, " AND c.created_at < ?{idx}").unwrap();
        bind_values.push(Box::new(next_day(added_before)));
    }

    if params.exclude_samples {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.FolderPath NOT LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(sampler_path_like_pattern()));
    }

    if let Some(ref playlist_id) = params.playlist {
        let idx = bind_values.len() + 1;
        write!(sql,
            " AND c.ID IN (SELECT sp.ContentID FROM djmdSongPlaylist sp WHERE sp.PlaylistID = ?{idx})"
        ).unwrap();
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

    sql.push_str(" ORDER BY c.Title, c.ID");
    if let Some(mut limit) = params.limit.or(default_limit) {
        if let Some(max_limit) = max_limit {
            limit = limit.min(max_limit);
        }
        write!(sql, " LIMIT {limit}").unwrap();
    }
    if let Some(offset) = params.offset {
        // SQLite requires LIMIT before OFFSET — use LIMIT -1 (unlimited) if needed
        if !sql.contains("LIMIT") {
            sql.push_str(" LIMIT -1");
        }
        write!(sql, " OFFSET {offset}").unwrap();
    }

    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
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

/// Compute the minimal set of root directories that cover all library audio files.
///
/// Algorithm:
/// 1. Query all distinct `FolderPath` values and extract their parent directory.
/// 2. Find the longest common path prefix across all directories.
/// 3. If the common prefix is at least 2 segments deep (e.g. `/Users/testuser/Music/`),
///    return it as the single root.
/// 4. Otherwise, collect the distinct directories at depth = common_depth + 1.
pub(crate) fn content_roots(
    conn: &Connection,
    exclude_samples: bool,
) -> Result<Vec<String>, rusqlite::Error> {
    let sampler_pattern = sampler_path_like_pattern();
    let sample_filter = if exclude_samples {
        " AND FolderPath NOT LIKE ?1 ESCAPE '\\'"
    } else {
        ""
    };
    let bind_params: &[&dyn rusqlite::types::ToSql] = if exclude_samples {
        &[&sampler_pattern]
    } else {
        &[]
    };

    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT FolderPath FROM djmdContent WHERE rb_local_deleted = 0{sample_filter}"
    ))?;
    let paths: Vec<String> = stmt
        .query_map(bind_params, |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;

    if paths.is_empty() {
        return Ok(Vec::new());
    }

    let dirs: Vec<Vec<&str>> = paths
        .iter()
        .filter_map(|p| {
            let trimmed = p.trim_end_matches('/');
            let last_slash = trimmed.rfind('/')?;
            let parent = &p[..=last_slash];
            Some(
                parent
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<&str>>(),
            )
        })
        .collect();

    if dirs.is_empty() {
        return Ok(Vec::new());
    }

    let mut common = dirs[0].clone();
    for dir in &dirs[1..] {
        let len = common
            .iter()
            .zip(dir.iter())
            .take_while(|(a, b)| a == b)
            .count();
        common.truncate(len);
    }
    let common_depth = common.len();

    if common_depth >= 2 {
        let root = format!("/{}/", common.join("/"));
        return Ok(vec![root]);
    }

    let mut roots: Vec<String> = dirs
        .iter()
        .filter_map(|segments| {
            let take = (common_depth + 1).min(segments.len());
            if take == 0 {
                return None;
            }
            Some(format!("/{}/", segments[..take].join("/")))
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    roots.sort();
    Ok(roots)
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
        " AND FolderPath NOT LIKE ?1 ESCAPE '\\'"
    } else {
        ""
    };
    let content_sample_filter = if exclude_samples {
        " AND c.FolderPath NOT LIKE ?1 ESCAPE '\\'"
    } else {
        ""
    };
    let bind_params: &[&dyn rusqlite::types::ToSql] = if exclude_samples {
        &[&sampler_pattern]
    } else {
        &[]
    };

    let total_tracks: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM djmdContent WHERE rb_local_deleted = 0{sample_filter}"),
        bind_params,
        |row| row.get(0),
    )?;

    let avg_bpm: f64 = conn
        .query_row(
            &format!("SELECT COALESCE(AVG(BPM), 0) FROM djmdContent WHERE rb_local_deleted = 0 AND BPM > 0{sample_filter}"),
            bind_params,
            |row| row.get(0),
        )
        .map(|v: f64| v / 100.0)?;

    let rated_count: i32 = conn.query_row(
        &format!("SELECT COUNT(*) FROM djmdContent WHERE rb_local_deleted = 0 AND Rating > 0{sample_filter}"),
        bind_params,
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
        .query_map(bind_params, |row| {
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
        .query_map(bind_params, |row| {
            Ok(KeyCount {
                name: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<_, _>>()?;

    let content_roots = content_roots(conn, exclude_samples)?;

    Ok(LibraryStats {
        total_tracks,
        genres,
        playlist_count,
        rated_count,
        unrated_count: total_tracks - rated_count,
        avg_bpm,
        key_distribution,
        content_roots,
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
