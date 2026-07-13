//! Preview, clear, and staged Rekordbox XML export workflows.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::adapters::rekordbox::{self, backup, xml};
use crate::domain::library::Track;
use crate::domain::metadata::{ChangeManager, TrackDiff};

#[derive(Debug)]
pub(crate) enum ExportOutcome {
    NoChanges,
    Written {
        path: PathBuf,
        track_count: usize,
        changes_applied: usize,
        backup: &'static str,
        playlist_count: usize,
    },
}

pub(crate) fn preview_changes(changes: &ChangeManager, current_tracks: &[Track]) -> Vec<TrackDiff> {
    changes.preview(current_tracks)
}

#[derive(Debug)]
pub(crate) enum PreviewWorkflowOutcome {
    NoChangesStaged,
    NoMatchingChanges,
    NoDifferences,
    Differences(Vec<TrackDiff>),
}

pub(crate) fn preview_workflow(
    conn: &rusqlite::Connection,
    changes: &ChangeManager,
    filter_ids: Option<&[String]>,
) -> Result<PreviewWorkflowOutcome, rusqlite::Error> {
    let mut ids = changes.pending_ids();
    if ids.is_empty() {
        return Ok(PreviewWorkflowOutcome::NoChangesStaged);
    }
    if let Some(filter_ids) = filter_ids {
        let filter = filter_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        ids.retain(|id| filter.contains(id.as_str()));
        if ids.is_empty() {
            return Ok(PreviewWorkflowOutcome::NoMatchingChanges);
        }
    }
    let current_tracks = rekordbox::get_tracks_by_ids(conn, &ids)?;
    let diffs = preview_changes(changes, &current_tracks);
    if diffs.is_empty() {
        Ok(PreviewWorkflowOutcome::NoDifferences)
    } else {
        Ok(PreviewWorkflowOutcome::Differences(diffs))
    }
}

pub(crate) fn build_preview_summary(diffs: &[TrackDiff]) -> serde_json::Value {
    let total_tracks = diffs.len();
    let mut by_field: HashMap<&str, usize> = HashMap::new();
    let mut by_genre: HashMap<&str, usize> = HashMap::new();

    for diff in diffs {
        for change in &diff.changes {
            *by_field.entry(&change.field).or_default() += 1;
            if change.field == "genre" {
                *by_genre.entry(&change.new_value).or_default() += 1;
            }
        }
    }

    let total_field_changes: usize = by_field.values().sum();
    let mut by_field_sorted: Vec<_> = by_field.into_iter().collect();
    by_field_sorted.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    let mut by_genre_sorted: Vec<_> = by_genre.into_iter().collect();
    by_genre_sorted.sort_by_key(|entry| std::cmp::Reverse(entry.1));

    let by_field_arr: Vec<_> = by_field_sorted
        .into_iter()
        .map(|(field, count)| serde_json::json!({"field": field, "count": count}))
        .collect();
    let mut result = serde_json::json!({
        "total_tracks": total_tracks,
        "total_field_changes": total_field_changes,
        "by_field": by_field_arr,
    });
    if !by_genre_sorted.is_empty() {
        result["by_genre"] = serde_json::json!(
            by_genre_sorted
                .into_iter()
                .map(|(genre, count)| serde_json::json!({"genre": genre, "count": count}))
                .collect::<Vec<_>>()
        );
    }
    result
}

pub(crate) fn clear_changes(
    changes: &ChangeManager,
    track_ids: Option<Vec<String>>,
    fields: Option<&[String]>,
) -> Result<serde_json::Value, String> {
    if let Some(fields) = fields {
        for field in fields {
            if crate::domain::metadata::EditableField::from_str(field).is_none() {
                return Err(format!(
                    "unknown field '{}'. Valid fields: {}",
                    field,
                    crate::domain::metadata::EditableField::all_names_csv()
                ));
            }
        }
        let (affected, remaining) = changes.clear_fields(track_ids, fields);
        Ok(serde_json::json!({
            "affected": affected,
            "remaining": remaining,
            "fields_cleared": fields,
        }))
    } else {
        let (cleared, remaining) = changes.clear(track_ids);
        Ok(serde_json::json!({
            "cleared": cleared,
            "remaining": remaining,
        }))
    }
}

fn default_output_path() -> PathBuf {
    static EXPORT_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S-%3f");
    let sequence = EXPORT_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("reklawdbox-exports")
        .join(format!(
            "reklawdbox-{timestamp}-{}-{sequence}.xml",
            std::process::id()
        ))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExportError {
    LabelGate(u32),
    Workflow(String),
}

/// Own the exact export order: gate, lock, snapshot, no-op, backup, read, XML, commit.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn export_changes<R, L>(
    changes: &ChangeManager,
    label_gate: &std::sync::atomic::AtomicU32,
    export_lock: &tokio::sync::Mutex<()>,
    skip_label_gate: bool,
    resolve_db_path: R,
    load_tracks: L,
    output_path: Option<PathBuf>,
    playlists: Vec<xml::PlaylistDef>,
) -> Result<ExportOutcome, ExportError>
where
    R: FnOnce() -> Result<PathBuf, String>,
    L: FnOnce(&[String]) -> Result<Vec<Track>, String>,
{
    export_with_dependencies(
        changes,
        label_gate,
        export_lock,
        skip_label_gate,
        resolve_db_path,
        output_path,
        playlists,
        |path| async move {
            backup::run_pre_op_backup(&path)
                .await
                .map(|status| status.as_str())
                .map_err(|error| format!("pre-op {error}"))
        },
        load_tracks,
        |tracks, playlists, path| {
            xml::write_xml_with_playlists(tracks, playlists, path)
                .map_err(|error| format!("Write error: {error}"))
        },
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn export_with_dependencies<R, B, BF, L, W>(
    changes: &ChangeManager,
    label_gate: &std::sync::atomic::AtomicU32,
    export_lock: &tokio::sync::Mutex<()>,
    skip_label_gate: bool,
    resolve_db_path: R,
    output_path: Option<PathBuf>,
    playlists: Vec<xml::PlaylistDef>,
    backup_fn: B,
    load_tracks: L,
    write_xml: W,
) -> Result<ExportOutcome, ExportError>
where
    R: FnOnce() -> Result<PathBuf, String>,
    B: FnOnce(PathBuf) -> BF,
    BF: std::future::Future<Output = Result<&'static str, String>>,
    L: FnOnce(&[String]) -> Result<Vec<Track>, String>,
    W: FnOnce(&[Track], &[xml::PlaylistDef], &Path) -> Result<(), String>,
{
    let gate_count = label_gate.load(std::sync::atomic::Ordering::Relaxed);
    if gate_count > 0 && !skip_label_gate {
        return Err(ExportError::LabelGate(gate_count));
    }

    let _export_guard = export_lock.lock().await;
    let has_playlists = !playlists.is_empty();
    let snapshot = changes.take_guard(None);
    if snapshot.is_empty() && !has_playlists {
        return Ok(ExportOutcome::NoChanges);
    }

    let db_path = resolve_db_path().map_err(ExportError::Workflow)?;
    let backup_status = backup_fn(db_path.to_path_buf())
        .await
        .map_err(ExportError::Workflow)?;

    let mut ids = Vec::new();
    let mut seen_ids = HashSet::new();
    for change in snapshot.changes() {
        if seen_ids.insert(change.track_id.clone()) {
            ids.push(change.track_id.clone());
        }
    }
    for playlist in &playlists {
        for track_id in &playlist.track_ids {
            if seen_ids.insert(track_id.clone()) {
                ids.push(track_id.clone());
            }
        }
    }

    let current_tracks = load_tracks(&ids).map_err(ExportError::Workflow)?;
    let found_ids: HashSet<&str> = current_tracks
        .iter()
        .map(|track| track.id.as_str())
        .collect();
    let missing_ids: Vec<String> = ids
        .iter()
        .filter(|id| !found_ids.contains(id.as_str()))
        .cloned()
        .collect();
    if !missing_ids.is_empty() {
        return Err(ExportError::Workflow(format!(
            "Track IDs not found in database: {}",
            missing_ids.join(", ")
        )));
    }

    let modified_tracks = changes.apply_snapshot(&current_tracks, snapshot.changes());
    let output_path = output_path.unwrap_or_else(default_output_path);
    write_xml(&modified_tracks, &playlists, &output_path).map_err(ExportError::Workflow)?;

    let outcome = ExportOutcome::Written {
        path: output_path,
        track_count: modified_tracks.len(),
        changes_applied: snapshot.len(),
        backup: backup_status,
        playlist_count: playlists.len(),
    };
    snapshot.commit();
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::metadata::TrackChange;

    fn track(id: &str) -> Track {
        Track {
            id: id.to_string(),
            title: "Title".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            genre: "Old".to_string(),
            bpm: 120.0,
            key: String::new(),
            rating: 0,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 2000,
            length: 180,
            file_path: "/tmp/test.flac".to_string(),
            play_count: 0,
            bit_rate: 0,
            sample_rate: 0,
            file_kind: crate::domain::library::FileKind::Flac,
            date_added: String::new(),
            position: None,
            played_at: None,
        }
    }

    #[test]
    fn metadata_workflow_preserves_preview_and_clear() {
        let changes = ChangeManager::new();
        changes.stage(vec![TrackChange {
            track_id: "track-1".to_string(),
            genre: Some("New".to_string()),
            ..Default::default()
        }]);
        let preview = preview_changes(&changes, &[track("track-1")]);
        assert_eq!(preview.len(), 1);
        let cleared = clear_changes(&changes, None, None).unwrap();
        assert_eq!(cleared["cleared"], 1);
        assert!(changes.pending_ids().is_empty());
    }

    #[tokio::test]
    async fn boundary_metadata_export_never_writes_rekordbox_db() {
        let changes = ChangeManager::new();
        changes.stage(vec![TrackChange {
            track_id: "track-1".to_string(),
            genre: Some("New".to_string()),
            ..Default::default()
        }]);
        let events = std::sync::Mutex::new(Vec::new());
        let gate = std::sync::atomic::AtomicU32::new(0);
        let export_lock = tokio::sync::Mutex::new(());
        let temp = tempfile::NamedTempFile::new().unwrap();
        let outcome = export_with_dependencies(
            &changes,
            &gate,
            &export_lock,
            false,
            || Ok(PathBuf::from("/read-only/master.db")),
            Some(temp.path().to_path_buf()),
            Vec::new(),
            |_| async {
                events.lock().unwrap().push("backup");
                Ok("success")
            },
            |ids| {
                events.lock().unwrap().push("read");
                assert_eq!(ids, &["track-1".to_string()]);
                Ok(vec![track("track-1")])
            },
            |tracks, _, _| {
                events.lock().unwrap().push("xml");
                assert_eq!(tracks[0].genre, "New");
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(matches!(outcome, ExportOutcome::Written { .. }));
        assert_eq!(*events.lock().unwrap(), vec!["backup", "read", "xml"]);
        assert!(changes.pending_ids().is_empty());

        changes.stage(vec![TrackChange {
            track_id: "track-2".to_string(),
            year: Some(2002),
            ..Default::default()
        }]);
        let failed = export_with_dependencies(
            &changes,
            &gate,
            &export_lock,
            false,
            || Ok(PathBuf::from("/read-only/master.db")),
            None,
            Vec::new(),
            |_| async { Err("backup failed".to_string()) },
            |_| panic!("read must not run after backup failure"),
            |_, _, _| panic!("XML must not run after backup failure"),
        )
        .await;
        assert!(matches!(
            failed,
            Err(ExportError::Workflow(message)) if message == "backup failed"
        ));
        assert_eq!(changes.pending_ids(), vec!["track-2".to_string()]);
    }

    #[test]
    fn rekordbox_connection_is_statically_read_only() {
        let connection = include_str!("../../adapters/rekordbox/connection.rs");
        assert!(connection.contains("SQLITE_OPEN_READ_ONLY"));
        assert!(!connection.contains("SQLITE_OPEN_READ_WRITE"));
    }
}
