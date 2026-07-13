use std::collections::HashMap;
use std::fmt::Write as _;

use rusqlite::{Connection, params};

use crate::domain::library::{Session, Track, TrackPlayStats};

use super::tracks::{SearchParams, TRACK_COLUMNS, TRACK_JOINS, apply_search_filters, row_to_track};

// ---------------------------------------------------------------------------
// DJ session history
// ---------------------------------------------------------------------------

pub fn get_sessions(
    conn: &Connection,
    limit: Option<u32>,
    after: Option<&str>,
) -> Result<Vec<Session>, rusqlite::Error> {
    let limit = limit.unwrap_or(20).min(100);

    let mut sql = String::from(
        "SELECT h.ID, COALESCE(h.Name, '') AS Name, COALESCE(h.DateCreated, '') AS DateCreated,
           (SELECT COUNT(*) FROM djmdSongHistory sh WHERE sh.HistoryID = h.ID AND sh.rb_local_deleted = 0) AS TrackCount
         FROM djmdHistory h
         WHERE h.rb_local_deleted = 0 AND COALESCE(h.Attribute, 0) != 1",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];

    if let Some(after_date) = after {
        let idx = bind_values.len() + 1;
        write!(sql, " AND h.DateCreated >= ?{idx}").unwrap();
        bind_values.push(Box::new(after_date.to_string()));
    }

    write!(sql, " ORDER BY h.DateCreated DESC LIMIT {limit}").unwrap();

    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let sessions: Vec<(String, String, String, i32)> = stmt
        .query_map(bind_params.as_slice(), |row| {
            Ok((
                row.get::<_, String>("ID")?,
                row.get::<_, String>("Name")?,
                row.get::<_, String>("DateCreated")?,
                row.get::<_, i32>("TrackCount")?,
            ))
        })?
        .collect::<Result<_, _>>()?;

    if sessions.is_empty() {
        return Ok(vec![]);
    }

    let placeholders: Vec<String> = (1..=sessions.len()).map(|i| format!("?{i}")).collect();
    let duration_sql = format!(
        "SELECT sh.HistoryID,
           CAST((julianday(MAX(sh.created_at)) - julianday(MIN(sh.created_at))) * 86400 AS INTEGER) AS span_seconds,
           (SELECT c.Length FROM djmdContent c WHERE c.ID =
             (SELECT sh2.ContentID FROM djmdSongHistory sh2 WHERE sh2.HistoryID = sh.HistoryID
              AND sh2.rb_local_deleted = 0 ORDER BY sh2.TrackNo DESC LIMIT 1)
           ) AS last_track_length
         FROM djmdSongHistory sh
         WHERE sh.HistoryID IN ({}) AND sh.rb_local_deleted = 0
           AND sh.created_at IS NOT NULL AND sh.created_at != ''
         GROUP BY sh.HistoryID",
        placeholders.join(", ")
    );
    let mut dur_stmt = conn.prepare(&duration_sql)?;
    let dur_refs: Vec<&dyn rusqlite::types::ToSql> = sessions
        .iter()
        .map(|(id, _, _, _)| id as &dyn rusqlite::types::ToSql)
        .collect();
    let duration_rows: Vec<(String, Option<i32>, Option<i32>)> = dur_stmt
        .query_map(dur_refs.as_slice(), |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?
        .collect::<Result<_, _>>()?;
    let durations: HashMap<String, Option<i32>> = duration_rows
        .into_iter()
        .map(|(id, span, last_len)| {
            let duration = match span {
                Some(s) if s > 0 => Some(s + last_len.unwrap_or(0)),
                _ => None,
            };
            (id, duration)
        })
        .collect();

    Ok(sessions
        .into_iter()
        .map(|(id, name, date_created, track_count)| {
            let duration_seconds = durations.get(&id).copied().flatten();
            Session {
                id,
                name,
                date_created,
                track_count,
                duration_seconds,
            }
        })
        .collect())
}

pub fn get_session_tracks(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<Track>, rusqlite::Error> {
    let sql = format!(
        "{TRACK_COLUMNS},
    sh.TrackNo AS Position,
    sh.created_at AS PlayedAt
{TRACK_JOINS}
INNER JOIN djmdSongHistory sh ON sh.ContentID = c.ID
WHERE sh.HistoryID = ?1 AND c.rb_local_deleted = 0 AND sh.rb_local_deleted = 0
ORDER BY sh.TrackNo"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![session_id], |row| {
        let mut track = row_to_track(row)?;
        track.position = Some(row.get::<_, u32>("Position")?);
        track.played_at = row
            .get::<_, Option<String>>("PlayedAt")?
            .filter(|s| !s.is_empty());
        Ok(track)
    })?;
    rows.collect()
}

pub fn get_play_stats(
    conn: &Connection,
    search: &SearchParams,
    include_unplayed: bool,
    limit: Option<u32>,
) -> Result<Vec<TrackPlayStats>, rusqlite::Error> {
    let limit = limit.unwrap_or(200).min(500);

    let mut sql = String::from(
        "SELECT c.ID AS TrackID, COALESCE(c.Title, '') AS Title, COALESCE(a.Name, '') AS ArtistName,
           COUNT(sh.ID) AS PlayCount, COUNT(DISTINCT sh.HistoryID) AS SessionCount,
           MAX(sh.created_at) AS LastPlayed, GROUP_CONCAT(DISTINCT sh.HistoryID) AS SessionIDs
         FROM djmdSongHistory sh
         INNER JOIN djmdContent c ON c.ID = sh.ContentID
         LEFT JOIN djmdArtist a ON c.ArtistID = a.ID
         LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID
         LEFT JOIN djmdGenre g ON c.GenreID = g.ID
         LEFT JOIN djmdKey k ON c.KeyID = k.ID
         LEFT JOIN djmdLabel l ON c.LabelID = l.ID
         LEFT JOIN djmdColor col ON c.ColorID = col.ID
         LEFT JOIN djmdArtist ra ON c.RemixerID = ra.ID
         WHERE c.rb_local_deleted = 0 AND sh.rb_local_deleted = 0
           AND sh.HistoryID IN (SELECT h.ID FROM djmdHistory h WHERE h.rb_local_deleted = 0)",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
    apply_search_filters(&mut sql, search, &mut bind_values);
    write!(
        sql,
        " GROUP BY c.ID ORDER BY PlayCount DESC, LastPlayed DESC LIMIT {limit}"
    )
    .unwrap();

    let mut stmt = conn.prepare(&sql)?;
    let bind_params: Vec<&dyn rusqlite::types::ToSql> = bind_values
        .iter()
        .map(std::convert::AsRef::as_ref)
        .collect();
    let mut results: Vec<TrackPlayStats> = stmt
        .query_map(bind_params.as_slice(), |row| {
            let session_ids_raw: Option<String> = row.get("SessionIDs")?;
            let session_ids: Vec<String> = session_ids_raw
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(std::string::ToString::to_string)
                .collect();
            Ok(TrackPlayStats {
                track_id: row.get("TrackID")?,
                title: row.get::<_, String>("Title")?.trim().to_string(),
                artist: row.get::<_, String>("ArtistName")?.trim().to_string(),
                play_count: row.get("PlayCount")?,
                session_count: row.get("SessionCount")?,
                last_played: row.get("LastPlayed")?,
                session_ids,
            })
        })?
        .collect::<Result<_, _>>()?;

    if include_unplayed {
        let mut unplayed_sql = String::from(
            "SELECT c.ID AS TrackID, COALESCE(c.Title, '') AS Title, COALESCE(a.Name, '') AS ArtistName
             FROM djmdContent c
             LEFT JOIN djmdArtist a ON c.ArtistID = a.ID
             LEFT JOIN djmdAlbum al ON c.AlbumID = al.ID
             LEFT JOIN djmdGenre g ON c.GenreID = g.ID
             LEFT JOIN djmdKey k ON c.KeyID = k.ID
             LEFT JOIN djmdLabel l ON c.LabelID = l.ID
             LEFT JOIN djmdColor col ON c.ColorID = col.ID
             LEFT JOIN djmdArtist ra ON c.RemixerID = ra.ID
             LEFT JOIN djmdSongHistory sh ON sh.ContentID = c.ID AND sh.rb_local_deleted = 0
               AND sh.HistoryID IN (SELECT h.ID FROM djmdHistory h WHERE h.rb_local_deleted = 0)
             WHERE c.rb_local_deleted = 0 AND sh.ID IS NULL",
        );
        let mut unplayed_bind: Vec<Box<dyn rusqlite::types::ToSql>> = vec![];
        apply_search_filters(&mut unplayed_sql, search, &mut unplayed_bind);
        let unplayed_limit = limit.saturating_sub(results.len() as u32);
        write!(unplayed_sql, " ORDER BY c.Title LIMIT {unplayed_limit}").unwrap();

        let mut stmt = conn.prepare(&unplayed_sql)?;
        let bind_params: Vec<&dyn rusqlite::types::ToSql> = unplayed_bind
            .iter()
            .map(std::convert::AsRef::as_ref)
            .collect();
        let unplayed: Vec<TrackPlayStats> = stmt
            .query_map(bind_params.as_slice(), |row| {
                Ok(TrackPlayStats {
                    track_id: row.get("TrackID")?,
                    title: row.get::<_, String>("Title")?.trim().to_string(),
                    artist: row.get::<_, String>("ArtistName")?.trim().to_string(),
                    play_count: 0,
                    session_count: 0,
                    last_played: None,
                    session_ids: vec![],
                })
            })?
            .collect::<Result<_, _>>()?;
        results.extend(unplayed);
    }

    Ok(results)
}
