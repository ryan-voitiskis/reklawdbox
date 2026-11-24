use crate::domain::library::{Cue, CueKind};
use rusqlite::{Connection, Result, params, params_from_iter, types::ValueRef};
use serde::Serialize;
use sha2::{Digest, Sha256};

const ACTIVE_CUES: &str = "FROM djmdCue q JOIN djmdContent t ON t.ID = q.ContentID
    WHERE COALESCE(q.rb_local_deleted, 0) = 0 AND COALESCE(t.rb_local_deleted, 0) = 0";

const LOOP_END_MS: &str = "NULLIF(NULLIF(q.OutMsec, -1), 4294967295)";

fn memory_point_predicate() -> String {
    format!("q.Kind = 0 AND {LOOP_END_MS} IS NULL")
}

#[derive(Serialize)]
pub(crate) struct CuePage {
    total_cues: u64,
    total_tracks: u64,
    cues: Vec<Cue>,
}

pub(crate) fn list_cues(
    conn: &Connection,
    track: Option<&str>,
    comment: Option<&str>,
    memory_points_only: bool,
    limit: u32,
    offset: u32,
) -> Result<CuePage> {
    let tx = conn.unchecked_transaction()?;
    let mut scope = ACTIVE_CUES.to_owned();
    let mut filters = Vec::new();
    if let Some(id) = track {
        scope.push_str(" AND q.ContentID = ?");
        filters.push(id);
    }
    if let Some(text) = comment {
        scope.push_str(" AND q.Comment COLLATE BINARY = ?");
        filters.push(text);
    }
    if memory_points_only {
        scope.push_str(" AND ");
        scope.push_str(&memory_point_predicate());
    }
    let (total_cues, total_tracks) = tx.query_row(
        &format!("SELECT COUNT(*), COUNT(DISTINCT q.ContentID) {scope}"),
        params_from_iter(&filters),
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let cues = tx
        .prepare(&format!(
            "SELECT q.ID, q.ContentID, t.Title,
        (SELECT a.Name FROM djmdArtist a WHERE a.ID=t.ArtistID),
        q.Kind, q.InMsec, {LOOP_END_MS}, q.Comment, q.Color, q.ColorTableIndex, q.ActiveLoop
        {scope} ORDER BY q.ContentID, q.InMsec, q.ID LIMIT {limit} OFFSET {offset}"
        ))?
        .query_map(params_from_iter(&filters), |r| {
            let raw_kind = r.get(4)?;
            let loop_end_ms = r.get(6)?;
            let (kind, hot_slot) = CueKind::from_raw(raw_kind, loop_end_ms);
            Ok(Cue {
                cue_id: r.get(0)?,
                track_id: r.get(1)?,
                title: r.get(2)?,
                artist: r.get(3)?,
                kind,
                raw_kind,
                hot_slot,
                start_ms: r.get(5)?,
                loop_end_ms,
                comment: r.get(7)?,
                color: r.get(8)?,
                color_table_index: r.get(9)?,
                active_loop: r.get(10)?,
            })
        })?
        .collect::<Result<_>>()?;
    tx.commit()?;
    Ok(CuePage {
        total_cues,
        total_tracks,
        cues,
    })
}

pub(crate) struct RemovalInventory {
    pub target_count: usize,
    pub target_tracks: usize,
    pub protected_count: usize,
    pub active_tracks: usize,
    pub fingerprint: String,
}

// Encode type and length so that NULL, empty strings, blobs and adjacent fields
// remain distinguishable, including on new versions with additional cue columns.
fn hash_value(hash: &mut Sha256, value: ValueRef<'_>) {
    let (tag, bytes): (u8, &[u8]) = match value {
        ValueRef::Null => (0, &[]),
        ValueRef::Integer(n) => (1, &n.to_le_bytes()),
        ValueRef::Real(n) => (2, &n.to_bits().to_le_bytes()),
        ValueRef::Text(s) => (3, s),
        ValueRef::Blob(b) => (4, b),
    };
    hash.update([tag]);
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

pub(crate) fn removal_inventory(
    conn: &Connection,
    track: Option<&str>,
    comment: &str,
) -> Result<RemovalInventory> {
    let tx = conn.unchecked_transaction()?;
    let mut hash = Sha256::new();
    hash.update(b"reklawdbox.memory-cue-removal.v1");
    hash_value(
        &mut hash,
        track.map_or(ValueRef::Null, |s| ValueRef::Text(s.as_bytes())),
    );
    hash_value(&mut hash, ValueRef::Text(comment.as_bytes()));
    let mut active_tracks = 0;
    // Removing a track must never pass verification as removal of its last cue.
    {
        let mut stmt = tx.prepare(
            "SELECT ID, UUID FROM djmdContent WHERE COALESCE(rb_local_deleted,0)=0 ORDER BY ID",
        )?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            active_tracks += 1;
            hash.update(b"T");
            hash_value(&mut hash, r.get_ref(0)?);
            hash_value(&mut hash, r.get_ref(1)?);
        }
    }
    let memory_point = memory_point_predicate();
    let sql = format!("SELECT (({memory_point}) AND q.Comment COLLATE BINARY = ?1 AND (?2 IS NULL OR q.ContentID=?2)) IS TRUE,
        q.ContentID, q.* {ACTIVE_CUES} ORDER BY q.ContentID,q.ID");
    let mut stmt = tx.prepare(&sql)?;
    let columns = stmt.column_count();
    for name in &stmt.column_names()[2..] {
        hash_value(&mut hash, ValueRef::Text(name.as_bytes()));
    }
    let mut rows = stmt.query(params![comment, track])?;
    let mut target_tracks = 0;
    let mut previous_track: Option<String> = None;
    let mut target_count = 0;
    let mut protected_count = 0;
    while let Some(r) = rows.next()? {
        if r.get::<_, bool>(0)? {
            target_count += 1;
            let id: String = r.get(1)?;
            if previous_track.as_ref() != Some(&id) {
                previous_track = Some(id);
                target_tracks += 1;
            }
        } else {
            protected_count += 1;
            hash.update(b"C");
            for col in 2..columns {
                hash_value(&mut hash, r.get_ref(col)?);
            }
        }
    }
    drop(rows);
    drop(stmt);
    tx.commit()?;
    Ok(RemovalInventory {
        target_count,
        target_tracks,
        protected_count,
        active_tracks,
        fingerprint: format!("{:x}", hash.finalize()),
    })
}
