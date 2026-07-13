use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use rusqlite::{Connection, params};

use crate::types::Track;

use super::tracks::{
    SearchParams, TRACK_SELECT, apply_search_filters, escape_like, row_to_track,
    sampler_path_like_pattern,
};

pub fn paths_imported_in_scope(
    conn: &Connection,
    scope: &str,
) -> Result<HashSet<String>, rusqlite::Error> {
    let mut stmt = conn.prepare_cached(
        "SELECT FolderPath FROM djmdContent \
         WHERE rb_local_deleted = 0 AND FolderPath LIKE ?1 ESCAPE '\\'",
    )?;
    let like_pattern = format!("{}%", escape_like(scope));
    let rows = stmt.query_map(params![like_pattern], |row| row.get::<_, String>(0))?;
    let mut set = HashSet::new();
    for row in rows {
        set.insert(row?);
    }
    Ok(set)
}

// ---------------------------------------------------------------------------
// Library health queries
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct TrackPathEntry {
    pub id: String,
    pub artist: String,
    pub title: String,
    pub path: String,
}

pub(crate) struct PlaylistCoverageResult {
    pub tracks: Vec<Track>,
    pub uncovered_count: i64,
    pub total_tracks: i64,
}

pub(crate) struct DuplicateGroup {
    pub artist: String,
    pub title: String,
    pub track_ids: Vec<String>,
}

pub(crate) struct MetadataDuplicatePage {
    pub groups: Vec<DuplicateGroup>,
    pub total: usize,
}

pub(crate) fn tracks_not_in_any_playlist(
    conn: &Connection,
    search: &SearchParams,
) -> Result<PlaylistCoverageResult, rusqlite::Error> {
    let filter_joins = "\
         LEFT JOIN djmdArtist a ON c.ArtistID = a.ID \
         LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID \
         LEFT JOIN djmdGenre g ON c.GenreID = g.ID \
         LEFT JOIN djmdKey k ON c.KeyID = k.ID \
         LEFT JOIN djmdLabel l ON c.LabelID = l.ID \
         LEFT JOIN djmdColor col ON c.ColorID = col.ID \
         LEFT JOIN djmdArtist ra ON c.RemixerID = ra.ID";

    let mut total_sql =
        format!("SELECT COUNT(*) FROM djmdContent c {filter_joins} WHERE c.rb_local_deleted = 0");
    let mut total_bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
    apply_search_filters(&mut total_sql, search, &mut total_bind);
    let total_params: Vec<&dyn rusqlite::types::ToSql> =
        total_bind.iter().map(std::convert::AsRef::as_ref).collect();
    let total_tracks: i64 =
        conn.query_row(&total_sql, total_params.as_slice(), |row| row.get(0))?;

    let mut uncov_sql = format!(
        "SELECT COUNT(*) FROM djmdContent c {filter_joins} \
         WHERE c.rb_local_deleted = 0 \
         AND NOT EXISTS (SELECT 1 FROM djmdSongPlaylist sp WHERE sp.ContentID = c.ID)"
    );
    let mut uncov_bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
    apply_search_filters(&mut uncov_sql, search, &mut uncov_bind);
    let uncov_params: Vec<&dyn rusqlite::types::ToSql> =
        uncov_bind.iter().map(std::convert::AsRef::as_ref).collect();
    let uncovered_count: i64 =
        conn.query_row(&uncov_sql, uncov_params.as_slice(), |row| row.get(0))?;

    let mut sql = format!(
        "{TRACK_SELECT} WHERE c.rb_local_deleted = 0 \
         AND NOT EXISTS (SELECT 1 FROM djmdSongPlaylist sp WHERE sp.ContentID = c.ID)"
    );
    let mut track_bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
    apply_search_filters(&mut sql, search, &mut track_bind);
    sql.push_str(" ORDER BY c.Title");
    let limit = search.limit.unwrap_or(200).min(500);
    write!(sql, " LIMIT {limit}").unwrap();
    if let Some(offset) = search.offset {
        write!(sql, " OFFSET {offset}").unwrap();
    }

    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> =
        track_bind.iter().map(std::convert::AsRef::as_ref).collect();
    let tracks: Vec<Track> = stmt
        .query_map(params.as_slice(), row_to_track)?
        .collect::<Result<_, _>>()?;

    Ok(PlaylistCoverageResult {
        tracks,
        uncovered_count,
        total_tracks,
    })
}

/// Find metadata duplicates: groups by LOWER(TRIM(artist)) + LOWER(TRIM(title)).
/// Returns separate artist/title fields to avoid separator collision in display keys.
#[cfg(test)]
pub(crate) fn find_metadata_duplicates(
    conn: &Connection,
    path_prefix: Option<&str>,
    limit: Option<u32>,
) -> Result<Vec<DuplicateGroup>, rusqlite::Error> {
    let limit = limit.unwrap_or(50).min(200);
    Ok(find_metadata_duplicates_page(conn, path_prefix, limit, 0)?.groups)
}

pub(crate) fn find_metadata_duplicates_page(
    conn: &Connection,
    path_prefix: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<MetadataDuplicatePage, rusqlite::Error> {
    let limit = limit.min(200);
    let mut grouped_sql = String::from(
        "SELECT LOWER(TRIM(COALESCE(a.Name, ''))) AS dup_artist, \
                LOWER(TRIM(COALESCE(c.Title, ''))) AS dup_title, \
                GROUP_CONCAT(c.ID) AS track_ids \
         FROM djmdContent c \
         LEFT JOIN djmdArtist a ON c.ArtistID = a.ID \
         WHERE c.rb_local_deleted = 0 \
         AND c.FolderPath NOT LIKE ?1 ESCAPE '\\'",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(sampler_path_like_pattern())];

    if let Some(prefix) = path_prefix {
        let idx = bind_values.len() + 1;
        write!(grouped_sql, " AND c.FolderPath LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("{}%", escape_like(prefix))));
    }

    grouped_sql.push_str(
        " GROUP BY LOWER(TRIM(COALESCE(a.Name, ''))), LOWER(TRIM(COALESCE(c.Title, ''))) \
         HAVING COUNT(*) > 1 AND LOWER(TRIM(COALESCE(c.Title, ''))) != ''",
    );
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let total_sql = format!("SELECT COUNT(*) FROM ({grouped_sql}) duplicate_groups");
    let total_i64: i64 = conn.query_row(&total_sql, bind_params.as_slice(), |row| row.get(0))?;
    let total = usize::try_from(total_i64).unwrap_or(usize::MAX);

    let mut sql = grouped_sql;
    write!(
        sql,
        " ORDER BY COUNT(*) DESC, dup_artist, dup_title LIMIT {limit} OFFSET {offset}"
    )
    .unwrap();
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(bind_params.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    let groups = rows
        .into_iter()
        .map(|(artist, title, ids_str)| {
            let mut track_ids: Vec<String> = ids_str
                .split(',')
                .map(std::string::ToString::to_string)
                .collect();
            track_ids.sort();
            track_ids.dedup();
            DuplicateGroup {
                artist,
                title,
                track_ids,
            }
        })
        .collect();
    Ok(MetadataDuplicatePage { groups, total })
}

pub(crate) fn all_track_paths(
    conn: &Connection,
    path_prefix: Option<&str>,
) -> Result<Vec<TrackPathEntry>, rusqlite::Error> {
    let mut sql = String::from(
        "SELECT c.ID, COALESCE(a.Name, '') AS ArtistName, \
                COALESCE(c.Title, '') AS Title, COALESCE(c.FolderPath, '') AS FolderPath \
         FROM djmdContent c \
         LEFT JOIN djmdArtist a ON c.ArtistID = a.ID \
         WHERE c.rb_local_deleted = 0 \
         AND c.FolderPath NOT LIKE ?1 ESCAPE '\\'",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(sampler_path_like_pattern())];

    if let Some(prefix) = path_prefix {
        let idx = bind_values.len() + 1;
        write!(sql, " AND c.FolderPath LIKE ?{idx} ESCAPE '\\'").unwrap();
        bind_values.push(Box::new(format!("{}%", escape_like(prefix))));
    }

    sql.push_str(" ORDER BY c.FolderPath");

    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    stmt.query_map(bind_params.as_slice(), |row| {
        Ok(TrackPathEntry {
            id: row.get::<_, String>(0)?,
            artist: row.get::<_, String>(1)?.trim().to_string(),
            title: row.get::<_, String>(2)?.trim().to_string(),
            path: row.get::<_, String>(3)?,
        })
    })?
    .collect()
}

pub(crate) fn playlist_membership_counts(
    conn: &Connection,
    track_ids: &[String],
) -> Result<HashMap<String, i32>, rusqlite::Error> {
    if track_ids.is_empty() {
        return Ok(HashMap::new());
    }
    const MAX_BIND: usize = 900;
    let mut result = HashMap::new();

    for chunk in track_ids.chunks(MAX_BIND) {
        let placeholders: Vec<String> = (1..=chunk.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT ContentID, COUNT(DISTINCT PlaylistID) AS cnt \
             FROM djmdSongPlaylist \
             WHERE ContentID IN ({}) \
             GROUP BY ContentID",
            placeholders.join(", ")
        );
        let mut stmt = conn.prepare(&sql)?;
        let refs: Vec<&dyn rusqlite::types::ToSql> = chunk
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows: Vec<(String, i32)> = stmt
            .query_map(refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<_, _>>()?;
        for (id, count) in rows {
            result.insert(id, count);
        }
    }

    Ok(result)
}

/// Count active Rekordbox library tracks.
pub(crate) fn active_track_count(conn: &Connection) -> Result<i32, rusqlite::Error> {
    conn.query_row(
        "SELECT COUNT(*) FROM djmdContent WHERE rb_local_deleted = 0",
        [],
        |row| row.get(0),
    )
}

/// Count active non-sampler Rekordbox library tracks.
pub(crate) fn non_sampler_track_count(conn: &Connection) -> Result<i64, rusqlite::Error> {
    let sample_prefix = format!("%{}%", escape_like(super::tracks::SAMPLER_PATH_FRAGMENT));
    conn.query_row(
        "SELECT COUNT(*) FROM djmdContent
         WHERE rb_local_deleted = 0
           AND FolderPath NOT LIKE ?1 ESCAPE '\\'",
        params![sample_prefix],
        |row| row.get(0),
    )
}
