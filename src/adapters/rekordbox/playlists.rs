use std::fmt::Write as _;

use rusqlite::{Connection, params};

use crate::domain::library::{Playlist, Track};

use super::tracks::{TRACK_COLUMNS, TRACK_JOINS, row_to_track};

fn get_playlist_tracks_with_limit_policy(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
    offset: Option<u32>,
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

    let mut sql = format!(
        "{TRACK_COLUMNS},
    sp.TrackNo AS Position
{TRACK_JOINS}
INNER JOIN djmdSongPlaylist sp ON sp.ContentID = c.ID
WHERE sp.PlaylistID = ?1 AND c.rb_local_deleted = 0
ORDER BY sp.TrackNo, c.ID"
    );
    if let Some(limit) = resolved_limit {
        write!(sql, " LIMIT {limit}").unwrap();
    }
    if let Some(offset) = offset {
        // SQLite requires LIMIT before OFFSET. The unbounded variant has no
        // resolved limit, so use SQLite's unlimited sentinel in that case.
        if resolved_limit.is_none() {
            sql.push_str(" LIMIT -1");
        }
        write!(sql, " OFFSET {offset}").unwrap();
    }

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![playlist_id], row_to_playlist_track)?;
    rows.collect()
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
    let mut stmt = conn.prepare_cached(sql)?;
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
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, None, Some(200), Some(200))
}

/// Bounded playlist page for shared selector paths. Applies offset in
/// `sp.TrackNo` order while retaining the ordinary 200-track default and cap.
pub fn get_playlist_tracks_page(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, offset, Some(200), Some(200))
}

/// Unbounded variant of `get_playlist_tracks` with no safety limit.
/// Intended for full-library diagnostics and calibration paths, not ordinary
/// agent browsing responses.
pub fn get_playlist_tracks_unbounded(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, None, None, None)
}

/// Unbounded playlist page for diagnostic selector paths. Offset without a
/// limit uses SQLite's `LIMIT -1 OFFSET ...` form.
pub fn get_playlist_tracks_unbounded_page(
    conn: &Connection,
    playlist_id: &str,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<Track>, rusqlite::Error> {
    get_playlist_tracks_with_limit_policy(conn, playlist_id, limit, offset, None, None)
}
