use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use super::color;
use super::model::{EditableField, FieldDiff, TrackChange, TrackDiff};
use crate::domain::library::Track;

fn acquire_or_recover_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| {
        tracing::warn!("mutex poisoned, recovering");
        e.into_inner()
    })
}

/// Per-track set of fields modified (staged or cleared) after the most recent `take()`.
/// Used by `restore()` to avoid overwriting user intent with snapshot values.
type TouchedFields = HashMap<String, HashSet<EditableField>>;

pub struct ChangeManager {
    changes: Mutex<HashMap<String, TrackChange>>,
    touched_since_take: Mutex<TouchedFields>,
    /// Set by `clear(None)` after `take()` to signal that the user intended to
    /// wipe everything.  `restore()` treats all snapshot fields as touched when
    /// this is true, preventing resurrection of taken-then-cleared changes.
    cleared_all_since_take: AtomicBool,
}

pub(crate) struct ChangeSnapshotGuard<'a> {
    manager: &'a ChangeManager,
    snapshot: Option<Vec<TrackChange>>,
}

impl ChangeSnapshotGuard<'_> {
    pub(crate) fn changes(&self) -> &[TrackChange] {
        self.snapshot.as_deref().unwrap_or_default()
    }

    pub(crate) fn len(&self) -> usize {
        self.changes().len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.changes().is_empty()
    }

    pub(crate) fn commit(mut self) {
        self.snapshot.take();
    }
}

impl Drop for ChangeSnapshotGuard<'_> {
    fn drop(&mut self) {
        if let Some(snapshot) = self.snapshot.take() {
            self.manager.restore(snapshot);
        }
    }
}

impl ChangeManager {
    pub fn new() -> Self {
        Self {
            changes: Mutex::new(HashMap::new()),
            touched_since_take: Mutex::new(HashMap::new()),
            cleared_all_since_take: AtomicBool::new(false),
        }
    }

    /// Stage changes for one or more tracks. Merges with previously staged changes for the same track.
    pub fn stage(&self, changes: Vec<TrackChange>) -> (usize, usize) {
        let mut staged_changes_by_track_id = acquire_or_recover_lock(&self.changes);
        let mut touched = acquire_or_recover_lock(&self.touched_since_take);
        let mut staged = 0;
        for change in changes {
            if !has_any_staged_field(&change) {
                continue;
            }
            staged += 1;
            record_touched_fields(&change, &mut touched);
            staged_changes_by_track_id
                .entry(change.track_id.clone())
                .and_modify(|existing| merge_track_change(existing, &change))
                .or_insert(change);
        }
        (staged, staged_changes_by_track_id.len())
    }

    pub fn pending_ids(&self) -> Vec<String> {
        let map = acquire_or_recover_lock(&self.changes);
        let mut ids: Vec<String> = map.keys().cloned().collect();
        ids.sort();
        ids
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        acquire_or_recover_lock(&self.changes).len()
    }

    pub fn get(&self, track_id: &str) -> Option<TrackChange> {
        self.changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(track_id)
            .cloned()
    }

    pub fn preview(&self, current_tracks: &[Track]) -> Vec<TrackDiff> {
        let map = acquire_or_recover_lock(&self.changes);
        let track_map: HashMap<&str, &Track> =
            current_tracks.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut result = Vec::new();

        for (track_id, change) in map.iter() {
            let Some(track) = track_map.get(track_id.as_str()) else {
                continue;
            };

            let mut field_diffs = Vec::new();

            for &f in EditableField::ALL {
                if let Some(new_val) = change.field_value_str(f) {
                    let old_val = track.editable_field_str(f);
                    if new_val != old_val {
                        field_diffs.push(FieldDiff {
                            field: f.as_str().to_string(),
                            old_value: old_val,
                            new_value: new_val,
                        });
                    }
                }
            }

            if !field_diffs.is_empty() {
                field_diffs.sort_by(|a, b| a.field.cmp(&b.field));
                result.push(TrackDiff {
                    track_id: track_id.clone(),
                    title: track.title.clone(),
                    artist: track.artist.clone(),
                    changes: field_diffs,
                });
            }
        }

        result.sort_by(|a, b| a.track_id.cmp(&b.track_id));
        result
    }

    #[cfg(test)]
    pub fn apply_changes(&self, tracks: &[Track]) -> Vec<Track> {
        let map = acquire_or_recover_lock(&self.changes);
        apply_changes_with_map(tracks, &map)
    }

    /// Apply a specific snapshot of staged changes, independent of in-memory staged state.
    pub fn apply_snapshot(&self, tracks: &[Track], snapshot: &[TrackChange]) -> Vec<Track> {
        let snapshot_map: HashMap<String, TrackChange> = snapshot
            .iter()
            .map(|change| (change.track_id.clone(), change.clone()))
            .collect();
        apply_changes_with_map(tracks, &snapshot_map)
    }

    /// Remove and return staged changes. If `track_ids` is None, drains all staged changes.
    /// Clears the touched-since-take set so subsequent mutations are tracked against this snapshot.
    pub fn take(&self, track_ids: Option<Vec<String>>) -> Vec<TrackChange> {
        let mut map = acquire_or_recover_lock(&self.changes);
        let mut touched = acquire_or_recover_lock(&self.touched_since_take);
        match track_ids {
            Some(ids) => {
                for id in &ids {
                    touched.remove(id);
                }
                ids.into_iter().filter_map(|id| map.remove(&id)).collect()
            }
            None => {
                touched.clear();
                self.cleared_all_since_take.store(false, Ordering::Release);
                let mut drained: Vec<TrackChange> = map.drain().map(|(_, change)| change).collect();
                drained.sort_by(|a, b| a.track_id.cmp(&b.track_id));
                drained
            }
        }
    }

    pub(crate) fn take_guard(&self, track_ids: Option<Vec<String>>) -> ChangeSnapshotGuard<'_> {
        ChangeSnapshotGuard {
            manager: self,
            snapshot: Some(self.take(track_ids)),
        }
    }

    /// Restore previously taken changes without overwriting fields the user
    /// touched (staged or cleared) since the snapshot was taken.
    pub fn restore(&self, snapshot: Vec<TrackChange>) -> (usize, usize) {
        if self.cleared_all_since_take.load(Ordering::Acquire) {
            tracing::warn!(
                snapshot_len = snapshot.len(),
                "restore skipped: clear(None) was called since last take()"
            );
            let map = acquire_or_recover_lock(&self.changes);
            return (0, map.len());
        }
        let mut map = acquire_or_recover_lock(&self.changes);
        let touched = acquire_or_recover_lock(&self.touched_since_take);
        let restored = snapshot.len();
        let empty_set = HashSet::new();
        for change in snapshot {
            let touched_fields = touched.get(&change.track_id).unwrap_or(&empty_set);
            let existing = map
                .entry(change.track_id.clone())
                .or_insert_with(|| TrackChange {
                    track_id: change.track_id.clone(),
                    genre: None,
                    comments: None,
                    rating: None,
                    color: None,
                    label: None,
                    year: None,
                    album: None,
                });
            merge_untouched_fields(existing, &change, touched_fields);
        }
        // Remove phantom entries where all fields were filtered out by touched tracking.
        map.retain(|_, v| has_any_staged_field(v));
        (restored, map.len())
    }

    /// Clear specific fields from staged changes. Returns (tracks_affected, remaining_tracks).
    /// If all fields on a track become None, the entry is removed entirely.
    /// Records cleared fields as touched even when the track has no current entry
    /// (e.g. after `take()` drained it), so that `restore()` won't resurrect them.
    pub fn clear_fields(
        &self,
        track_ids: Option<Vec<String>>,
        fields: &[String],
    ) -> (usize, usize) {
        let mut map = acquire_or_recover_lock(&self.changes);
        let mut touched_map = acquire_or_recover_lock(&self.touched_since_take);
        let target_ids: Vec<String> = match track_ids {
            Some(ids) => ids,
            None => map.keys().cloned().collect(),
        };

        let parsed_fields: Vec<EditableField> = fields
            .iter()
            .filter_map(|f| EditableField::from_str(f.as_str()))
            .collect();

        let mut affected = 0;
        for id in &target_ids {
            // Always record the touch — the user expressed intent to clear these
            // fields, even if the track has no current staged entry.
            let touched_set = touched_map.entry(id.clone()).or_default();
            for &ef in &parsed_fields {
                touched_set.insert(ef);
            }

            if let Some(entry) = map.get_mut(id) {
                let mut field_touched = false;
                for &ef in &parsed_fields {
                    if entry.clear_field(ef) {
                        field_touched = true;
                    }
                }
                if field_touched {
                    affected += 1;
                }
                if !has_any_staged_field(entry) {
                    map.remove(id);
                }
            }
        }
        (affected, map.len())
    }

    pub fn clear(&self, track_ids: Option<Vec<String>>) -> (usize, usize) {
        let mut map = acquire_or_recover_lock(&self.changes);
        let mut touched_map = acquire_or_recover_lock(&self.touched_since_take);
        let all_fields: HashSet<EditableField> = EditableField::ALL.iter().copied().collect();
        let cleared = match track_ids {
            Some(ids) => {
                let mut count = 0;
                for id in ids {
                    // Always record the touch — the user expressed intent to clear
                    // this track, even if it was already drained by take().
                    touched_map.insert(id.clone(), all_fields.clone());
                    if map.remove(&id).is_some() {
                        count += 1;
                    }
                }
                count
            }
            None => {
                let count = map.len();
                for id in map.keys() {
                    touched_map.insert(id.clone(), all_fields.clone());
                }
                map.clear();
                self.cleared_all_since_take.store(true, Ordering::Release);
                count
            }
        };
        (cleared, map.len())
    }
}

// ---------------------------------------------------------------------------
// TrackChange field-access helpers (driven by EditableField)
// ---------------------------------------------------------------------------

impl TrackChange {
    /// Whether the given field has a staged value.
    fn is_field_set(&self, field: EditableField) -> bool {
        match field {
            EditableField::Genre => self.genre.is_some(),
            EditableField::Comments => self.comments.is_some(),
            EditableField::Rating => self.rating.is_some(),
            EditableField::Color => self.color.is_some(),
            EditableField::Label => self.label.is_some(),
            EditableField::Year => self.year.is_some(),
            EditableField::Album => self.album.is_some(),
        }
    }

    /// Copy a single field from `other` into `self`, overwriting any existing value.
    fn merge_field_from(&mut self, other: &TrackChange, field: EditableField) {
        match field {
            EditableField::Genre => self.genre = other.genre.clone(),
            EditableField::Comments => self.comments = other.comments.clone(),
            EditableField::Rating => self.rating = other.rating,
            EditableField::Color => self.color = other.color.clone(),
            EditableField::Label => self.label = other.label.clone(),
            EditableField::Year => self.year = other.year,
            EditableField::Album => self.album = other.album.clone(),
        }
    }

    /// Clear a single field to `None`. Returns `true` if the field was set.
    fn clear_field(&mut self, field: EditableField) -> bool {
        let was_set = self.is_field_set(field);
        match field {
            EditableField::Genre => self.genre = None,
            EditableField::Comments => self.comments = None,
            EditableField::Rating => self.rating = None,
            EditableField::Color => self.color = None,
            EditableField::Label => self.label = None,
            EditableField::Year => self.year = None,
            EditableField::Album => self.album = None,
        }
        was_set
    }

    /// Return the staged value for `field` as a display string, or `None` if unset.
    fn field_value_str(&self, field: EditableField) -> Option<String> {
        match field {
            EditableField::Genre => self.genre.clone(),
            EditableField::Comments => self.comments.clone(),
            EditableField::Rating => self.rating.map(|v| v.to_string()),
            EditableField::Color => self.color.clone(),
            EditableField::Label => self.label.clone(),
            EditableField::Year => self.year.map(|v| v.to_string()),
            EditableField::Album => self.album.clone(),
        }
    }

    /// Apply a single staged field to a `Track`, mutating it in place.
    fn apply_field_to_track(&self, track: &mut Track, field: EditableField) {
        match field {
            EditableField::Genre => {
                if let Some(ref v) = self.genre {
                    track.genre = v.clone();
                }
            }
            EditableField::Comments => {
                if let Some(ref v) = self.comments {
                    track.comments = v.clone();
                }
            }
            EditableField::Rating => {
                if let Some(v) = self.rating {
                    track.rating = v;
                }
            }
            EditableField::Color => {
                if let Some(ref v) = self.color {
                    track.color = v.clone();
                    track.color_code = color::color_name_to_code(v).unwrap_or(0);
                }
            }
            EditableField::Label => {
                if let Some(ref v) = self.label {
                    track.label = v.clone();
                }
            }
            EditableField::Year => {
                if let Some(v) = self.year {
                    track.year = v;
                }
            }
            EditableField::Album => {
                if let Some(ref v) = self.album {
                    track.album = v.clone();
                }
            }
        }
    }
}

impl Track {
    /// Return the current value of an editable field as a display string.
    fn editable_field_str(&self, field: EditableField) -> String {
        match field {
            EditableField::Genre => self.genre.clone(),
            EditableField::Comments => self.comments.clone(),
            EditableField::Rating => self.rating.to_string(),
            EditableField::Color => self.color.clone(),
            EditableField::Label => self.label.clone(),
            EditableField::Year => self.year.to_string(),
            EditableField::Album => self.album.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Free functions that iterate via EditableField::ALL
// ---------------------------------------------------------------------------

fn has_any_staged_field(change: &TrackChange) -> bool {
    EditableField::ALL.iter().any(|f| change.is_field_set(*f))
}

fn merge_track_change(existing: &mut TrackChange, incoming: &TrackChange) {
    for &f in EditableField::ALL {
        if incoming.is_field_set(f) {
            existing.merge_field_from(incoming, f);
        }
    }
}

/// Fill missing fields from `incoming` into `existing`, skipping any field
/// present in `touched`.
fn merge_untouched_fields(
    existing: &mut TrackChange,
    incoming: &TrackChange,
    touched: &HashSet<EditableField>,
) {
    for &f in EditableField::ALL {
        if !existing.is_field_set(f) && !touched.contains(&f) {
            existing.merge_field_from(incoming, f);
        }
    }
}

fn record_touched_fields(change: &TrackChange, touched: &mut TouchedFields) {
    if !has_any_staged_field(change) {
        return;
    }
    let set = touched.entry(change.track_id.clone()).or_default();
    for &f in EditableField::ALL {
        if change.is_field_set(f) {
            set.insert(f);
        }
    }
}

fn apply_changes_with_map(
    tracks: &[Track],
    changes_by_track_id: &HashMap<String, TrackChange>,
) -> Vec<Track> {
    tracks
        .iter()
        .map(|track| {
            if let Some(change) = changes_by_track_id.get(&track.id) {
                let mut modified = track.clone();
                for &f in EditableField::ALL {
                    change.apply_field_to_track(&mut modified, f);
                }
                modified
            } else {
                track.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::library::FileKind;

    fn make_track(id: &str, genre: &str, rating: u8) -> Track {
        Track {
            id: id.to_string(),
            title: format!("Track {id}"),
            artist: "Artist".to_string(),
            album: String::new(),
            genre: genre.to_string(),
            bpm: 128.0,
            key: "Am".to_string(),
            rating,
            comments: String::new(),
            color: String::new(),
            color_code: 0,
            label: String::new(),
            remixer: String::new(),
            year: 2023,
            length: 300,
            file_path: format!("/music/{id}.flac"),
            play_count: 0,
            bit_rate: 1411,
            sample_rate: 44100,
            file_kind: FileKind::Flac,
            date_added: "2023-01-01".to_string(),
            position: None,
            played_at: None,
        }
    }

    #[test]
    fn test_stage_and_count() {
        let cm = ChangeManager::new();
        let (staged, total) = cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Deep House".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);
        assert_eq!(staged, 2);
        assert_eq!(total, 2);
        assert_eq!(cm.pending_count(), 2);
    }

    #[test]
    fn test_stage_merges() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: None,
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        // Second stage for same track: genre updates, rating preserved from first stage
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Deep House".to_string()),
            comments: Some("great track".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        assert_eq!(cm.pending_count(), 1);

        // Verify merge: genre updated, comments added, rating preserved
        let tracks = vec![make_track("t1", "Techno", 2)];
        let diffs = cm.preview(&tracks);
        assert_eq!(diffs.len(), 1);
        let td = &diffs[0];
        assert_eq!(td.track_id, "t1");
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "genre" && f.new_value == "Deep House")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "comments" && f.new_value == "great track")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "rating" && f.new_value == "4")
        );
    }

    #[test]
    fn test_stage_ignores_noop_changes() {
        let cm = ChangeManager::new();
        let (staged, total) = cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        assert_eq!(staged, 0);
        assert_eq!(total, 0);
        assert_eq!(cm.pending_count(), 0);
        assert!(cm.get("t1").is_none());
    }

    #[test]
    fn test_stage_noop_does_not_modify_existing_change() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: None,
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let (staged, total) = cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        assert_eq!(staged, 0);
        assert_eq!(total, 1);
        let change = cm.get("t1").expect("existing change should remain");
        assert_eq!(change.genre.as_deref(), Some("House"));
        assert_eq!(change.rating, Some(4));
    }

    #[test]
    fn test_preview() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3)];
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Deep House".to_string()),
            comments: Some("great bassline".to_string()),
            rating: Some(5),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let diffs = cm.preview(&tracks);
        assert_eq!(diffs.len(), 1);
        let td = &diffs[0];
        assert_eq!(td.changes.len(), 3); // genre, comments, rating changed
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "genre" && f.new_value == "Deep House")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "comments" && f.new_value == "great bassline")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "rating" && f.new_value == "5")
        );
    }

    #[test]
    fn test_preview_no_change() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3)];
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()), // same as current
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        let diffs = cm.preview(&tracks);
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_apply_changes() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3), make_track("t2", "Techno", 2)];
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Deep House".to_string()),
            comments: None,
            rating: Some(5),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let modified = cm.apply_changes(&tracks);
        assert_eq!(modified[0].genre, "Deep House");
        assert_eq!(modified[0].rating, 5);
        assert_eq!(modified[1].genre, "Techno");
        assert_eq!(modified[1].rating, 2);
    }

    #[test]
    fn test_clear_specific() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("A".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("B".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let (cleared, remaining) = cm.clear(Some(vec!["t1".to_string()]));
        assert_eq!(cleared, 1);
        assert_eq!(remaining, 1);
    }

    #[test]
    fn test_clear_all() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("A".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("B".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let (cleared, remaining) = cm.clear(None);
        assert_eq!(cleared, 2);
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_clear_fields() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("great".to_string()),
            rating: Some(4),
            color: Some("Green".to_string()),
            label: None,
            year: None,
            album: None,
        }]);

        let (affected, remaining) =
            cm.clear_fields(Some(vec!["t1".to_string()]), &["color".to_string()]);
        assert_eq!(affected, 1);
        assert_eq!(remaining, 1); // entry still exists (other fields set)

        let change = cm.get("t1").unwrap();
        assert!(change.color.is_none());
        assert_eq!(change.genre, Some("House".to_string()));
        assert_eq!(change.rating, Some(4));
    }

    #[test]
    fn test_clear_fields_removes_empty_entry() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let (affected, remaining) =
            cm.clear_fields(Some(vec!["t1".to_string()]), &["genre".to_string()]);
        assert_eq!(affected, 1);
        assert_eq!(remaining, 0); // entry removed since all fields are None
        assert!(cm.get("t1").is_none());
    }

    #[test]
    fn test_clear_fields_all_tracks() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("House".to_string()),
                comments: None,
                rating: Some(3),
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let (affected, remaining) = cm.clear_fields(None, &["genre".to_string()]);
        assert_eq!(affected, 2);
        assert_eq!(remaining, 1); // t1 still has rating, t2 removed entirely

        assert!(cm.get("t1").is_some());
        assert!(cm.get("t1").unwrap().genre.is_none());
        assert!(cm.get("t2").is_none());
    }

    #[test]
    fn test_preview_grouped() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3), make_track("t2", "Techno", 2)];
        cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Deep House".to_string()),
                comments: None,
                rating: Some(5),
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: None,
                comments: Some("nice track".to_string()),
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let diffs = cm.preview(&tracks);
        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].track_id, "t1");
        assert_eq!(diffs[0].changes.len(), 2);
        assert_eq!(diffs[1].track_id, "t2");
        assert_eq!(diffs[1].changes.len(), 1);
        assert_eq!(diffs[1].changes[0].field, "comments");
    }

    #[test]
    fn test_apply_changes_with_color_code() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3)];
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: None,
            rating: None,
            color: Some("Green".to_string()),
            label: None,
            year: None,
            album: None,
        }]);

        let modified = cm.apply_changes(&tracks);
        assert_eq!(modified[0].color, "Green");
        assert_eq!(modified[0].color_code, 0x00FF00);
    }

    #[test]
    fn test_apply_changes_color_code_preserves_original_when_no_change() {
        let cm = ChangeManager::new();
        let mut track = make_track("t1", "House", 3);
        track.color = "Red".to_string();
        track.color_code = 0xFF0000;
        let tracks = vec![track];

        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Techno".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let modified = cm.apply_changes(&tracks);
        assert_eq!(modified[0].color, "Red");
        assert_eq!(modified[0].color_code, 0xFF0000);
    }

    #[test]
    fn test_apply_changes_unknown_color_resets_color_code() {
        let cm = ChangeManager::new();
        let mut track = make_track("t1", "House", 3);
        track.color = "Red".to_string();
        track.color_code = 0xFF0000;
        let tracks = vec![track];

        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: None,
            rating: None,
            color: Some("Purple".to_string()),
            label: None,
            year: None,
            album: None,
        }]);

        let modified = cm.apply_changes(&tracks);
        assert_eq!(modified[0].color, "Purple");
        assert_eq!(modified[0].color_code, 0);
    }

    #[test]
    fn test_take_all_drains_pending_changes() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("House".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let snapshot = cm.take(None);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(cm.pending_count(), 0);
    }

    #[test]
    fn test_pending_ids_are_sorted() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("House".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        assert_eq!(cm.pending_ids(), vec!["t1".to_string(), "t2".to_string()]);
    }

    #[test]
    fn test_take_all_returns_sorted_snapshot() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("House".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
                label: None,
                year: None,
                album: None,
            },
        ]);

        let snapshot = cm.take(None);
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].track_id, "t1");
        assert_eq!(snapshot[1].track_id, "t2");
    }

    #[test]
    fn test_restore_keeps_newer_fields_and_restores_missing_ones() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("old".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);
        assert_eq!(cm.pending_count(), 0);

        // Simulate newer changes arriving while export is in progress.
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: Some("new".to_string()),
            rating: Some(5),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        cm.restore(snapshot);
        let restored = cm.get("t1").expect("t1 should be restored");
        assert_eq!(restored.genre.as_deref(), Some("House"));
        assert_eq!(restored.comments.as_deref(), Some("new"));
        assert_eq!(restored.rating, Some(5));
    }

    #[test]
    fn test_restore_does_not_resurrect_cleared_field() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("notes".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);

        // User explicitly clears genre while export is in flight.
        // First re-stage so there's something to clear.
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        cm.clear_fields(Some(vec!["t1".to_string()]), &["genre".to_string()]);

        // Export fails — restore snapshot.
        cm.restore(snapshot);
        let restored = cm.get("t1").expect("t1 should exist");
        // Genre must NOT be resurrected — the user explicitly cleared it.
        assert_eq!(restored.genre, None);
        // Comments were not touched post-take, so they should be restored.
        assert_eq!(restored.comments.as_deref(), Some("notes"));
    }

    #[test]
    fn test_restore_does_not_resurrect_field_on_absent_entry() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);

        // User stages and then fully clears, removing the entry entirely.
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Techno".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        cm.clear(Some(vec!["t1".to_string()]));
        assert!(cm.get("t1").is_none());

        // Restore should not bring back any fields that were touched.
        cm.restore(snapshot);
        // All fields on t1 were touched via clear(), so the phantom entry
        // should be cleaned up — no ghost entry in the map.
        assert!(cm.get("t1").is_none());
        assert_eq!(cm.pending_count(), 0);
    }

    #[test]
    fn test_restore_respects_clear_on_taken_track() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("notes".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);
        // t1 is no longer in the map — it was drained by take().

        // User clears t1 while the export is in flight. The entry doesn't
        // exist in the map, but the intent must still be recorded.
        cm.clear(Some(vec!["t1".to_string()]));

        // Export fails — restore snapshot.
        cm.restore(snapshot);
        // The clear expressed intent to remove t1 entirely, so restore
        // must not resurrect it.
        assert!(cm.get("t1").is_none());
        assert_eq!(cm.pending_count(), 0);
    }

    #[test]
    fn test_restore_respects_clear_fields_on_taken_track() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("notes".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);

        // User clears just genre on a taken track (no entry in map).
        cm.clear_fields(Some(vec!["t1".to_string()]), &["genre".to_string()]);

        // Export fails — restore snapshot.
        cm.restore(snapshot);
        let restored = cm
            .get("t1")
            .expect("t1 should exist with untouched comments");
        // Genre was explicitly cleared — must not be resurrected.
        assert_eq!(restored.genre, None);
        // Comments were not touched — should be restored from snapshot.
        assert_eq!(restored.comments.as_deref(), Some("notes"));
    }

    #[test]
    fn test_restore_partial_touch_preserves_untouched_fields() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("old notes".to_string()),
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);

        // User only stages a new genre — comments and rating are untouched.
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Techno".to_string()),
            comments: None,
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        cm.restore(snapshot);
        let restored = cm.get("t1").expect("t1 should exist");
        // Genre was re-staged, so keep the new value.
        assert_eq!(restored.genre.as_deref(), Some("Techno"));
        // Comments and rating were not touched post-take, so restore from snapshot.
        assert_eq!(restored.comments.as_deref(), Some("old notes"));
        assert_eq!(restored.rating, Some(4));
    }

    #[test]
    fn test_restore_respects_clear_all_after_take() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("notes".to_string()),
            rating: None,
            color: None,
            label: None,
            year: None,
            album: None,
        }]);

        let snapshot = cm.take(None);
        // t1 is drained. User calls clear(None) — "wipe everything".
        cm.clear(None);

        // Export fails — restore snapshot.
        cm.restore(snapshot);
        // clear(None) expressed intent to wipe all, so restore must not
        // resurrect any snapshot entries.
        assert!(cm.get("t1").is_none());
        assert_eq!(cm.pending_count(), 0);
    }

    #[test]
    fn change_snapshot_guard_drop_restores_snapshot() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            ..Default::default()
        }]);

        {
            let guard = cm.take_guard(None);
            assert_eq!(guard.len(), 1);
            assert_eq!(guard.changes()[0].track_id, "t1");
            assert_eq!(cm.pending_count(), 0);
        }

        assert_eq!(cm.pending_count(), 1);
        assert_eq!(
            cm.get("t1").and_then(|change| change.genre).as_deref(),
            Some("House")
        );
    }

    #[test]
    fn change_snapshot_guard_commit_leaves_changes_drained() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            ..Default::default()
        }]);

        let guard = cm.take_guard(None);
        assert!(!guard.is_empty());
        guard.commit();

        assert_eq!(cm.pending_count(), 0);
        assert!(cm.get("t1").is_none());
    }

    #[test]
    fn change_snapshot_guard_drop_preserves_post_take_stage_and_clear() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            comments: Some("old notes".to_string()),
            rating: Some(4),
            ..Default::default()
        }]);

        let guard = cm.take_guard(None);
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Techno".to_string()),
            ..Default::default()
        }]);
        cm.clear_fields(Some(vec!["t1".to_string()]), &["comments".to_string()]);
        drop(guard);

        let restored = cm.get("t1").expect("untouched fields should be restored");
        assert_eq!(restored.genre.as_deref(), Some("Techno"));
        assert_eq!(restored.comments, None);
        assert_eq!(restored.rating, Some(4));
    }

    #[tokio::test]
    async fn change_snapshot_guard_aborted_task_restores_snapshot() {
        const TEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

        let cm = std::sync::Arc::new(ChangeManager::new());
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("House".to_string()),
            ..Default::default()
        }]);
        let ready = std::sync::Arc::new(tokio::sync::Notify::new());
        let task_cm = std::sync::Arc::clone(&cm);
        let task_ready = std::sync::Arc::clone(&ready);
        let mut task = tokio::spawn(async move {
            let _guard = task_cm.take_guard(None);
            task_ready.notify_one();
            std::future::pending::<()>().await;
        });

        if tokio::time::timeout(TEST_TIMEOUT, ready.notified())
            .await
            .is_err()
        {
            task.abort();
            let _ = tokio::time::timeout(TEST_TIMEOUT, &mut task)
                .await
                .expect("guard cancellation cleanup should finish within five seconds");
            panic!("guard task did not signal after taking the snapshot within five seconds");
        }

        let snapshot_was_drained = cm.pending_count() == 0;
        task.abort();
        let join_result = tokio::time::timeout(TEST_TIMEOUT, &mut task)
            .await
            .expect("aborted guard task should join within five seconds");
        let join_error = join_result.expect_err("the deliberately pending task should be aborted");

        assert!(snapshot_was_drained, "guard should drain staged changes");
        assert!(join_error.is_cancelled(), "task should report cancellation");
        assert_eq!(cm.pending_count(), 1);
        assert_eq!(
            cm.get("t1").and_then(|change| change.genre).as_deref(),
            Some("House")
        );
    }

    // -----------------------------------------------------------------------
    // take(Some(...)) behavior
    // -----------------------------------------------------------------------

    #[test]
    fn test_take_some_returns_only_requested_ids() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".into(),
                genre: Some("House".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "t2".into(),
                genre: Some("Techno".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "t3".into(),
                genre: Some("Trance".into()),
                ..Default::default()
            },
        ]);

        let taken = cm.take(Some(vec!["t1".into(), "t3".into()]));
        assert_eq!(taken.len(), 2);
        assert_eq!(taken[0].track_id, "t1");
        assert_eq!(taken[1].track_id, "t3");

        // t2 should remain
        assert_eq!(cm.pending_count(), 1);
        let ids = cm.pending_ids();
        assert_eq!(ids, vec!["t2"]);
    }

    #[test]
    fn test_take_some_skips_absent_ids() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("House".into()),
            ..Default::default()
        }]);

        let taken = cm.take(Some(vec!["t1".into(), "nonexistent".into()]));
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].track_id, "t1");
        assert_eq!(cm.pending_count(), 0);
    }

    #[test]
    fn test_take_some_preserves_input_order() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "a".into(),
                genre: Some("House".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "b".into(),
                genre: Some("Techno".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "c".into(),
                genre: Some("Trance".into()),
                ..Default::default()
            },
        ]);

        // Request in reverse order — result should match request order, not sort order
        let taken = cm.take(Some(vec!["c".into(), "a".into(), "b".into()]));
        assert_eq!(taken[0].track_id, "c");
        assert_eq!(taken[1].track_id, "a");
        assert_eq!(taken[2].track_id, "b");
    }

    #[test]
    fn test_take_some_does_not_reset_cleared_all_flag() {
        let cm = ChangeManager::new();
        cm.stage(vec![
            TrackChange {
                track_id: "t1".into(),
                genre: Some("House".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "t2".into(),
                genre: Some("Techno".into()),
                ..Default::default()
            },
        ]);

        // clear(None) sets the cleared_all_since_take flag
        cm.clear(None);
        assert_eq!(cm.pending_count(), 0);

        // Stage fresh data
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("Trance".into()),
            ..Default::default()
        }]);

        // take(Some) does NOT reset cleared_all_since_take
        let snapshot = cm.take(Some(vec!["t1".into()]));
        assert_eq!(snapshot.len(), 1);

        // restore should bail because cleared_all_since_take is still true
        let (restored, total) = cm.restore(snapshot);
        assert_eq!(restored, 0);
        assert_eq!(total, 0);
    }

    // -----------------------------------------------------------------------
    // restore() return value semantics
    // -----------------------------------------------------------------------

    #[test]
    fn test_restore_returns_snapshot_len_regardless_of_merge() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("House".into()),
            ..Default::default()
        }]);

        let snapshot = cm.take(None);
        assert_eq!(snapshot.len(), 1);

        // Stage a new value for the same field before restore
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("Techno".into()),
            ..Default::default()
        }]);

        // Restore — the genre field is touched so snapshot's genre won't overwrite
        let (restored_count, total) = cm.restore(snapshot);
        // restored_count is always snapshot.len()
        assert_eq!(restored_count, 1);
        assert_eq!(total, 1);

        // Verify the new value was preserved (not overwritten by snapshot)
        let final_changes = cm.take(None);
        assert_eq!(final_changes[0].genre.as_deref(), Some("Techno"));
    }

    // -----------------------------------------------------------------------
    // Duplicate track_ids in single stage() call
    // -----------------------------------------------------------------------

    #[test]
    fn test_stage_duplicate_ids_merges_within_call() {
        let cm = ChangeManager::new();
        let (staged, total) = cm.stage(vec![
            TrackChange {
                track_id: "t1".into(),
                genre: Some("House".into()),
                ..Default::default()
            },
            TrackChange {
                track_id: "t1".into(),
                comments: Some("dup".into()),
                ..Default::default()
            },
        ]);
        // Both count toward staged
        assert_eq!(staged, 2);
        // But only one entry exists
        assert_eq!(total, 1);

        let taken = cm.take(None);
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].genre.as_deref(), Some("House"));
        assert_eq!(taken[0].comments.as_deref(), Some("dup"));
    }

    // -----------------------------------------------------------------------
    // clear_fields edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_clear_fields_unknown_field_is_ignored() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("House".into()),
            ..Default::default()
        }]);

        let (affected, total) = cm.clear_fields(None, &["nonexistent_field".into()]);
        assert_eq!(affected, 0);
        assert_eq!(total, 1);

        // Genre should still be staged
        let taken = cm.take(None);
        assert_eq!(taken[0].genre.as_deref(), Some("House"));
    }

    #[test]
    fn test_clear_fields_empty_fields_slice() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("House".into()),
            ..Default::default()
        }]);

        let (affected, total) = cm.clear_fields(None, &[]);
        assert_eq!(affected, 0);
        assert_eq!(total, 1);

        // Genre should still be staged
        let taken = cm.take(None);
        assert_eq!(taken[0].genre.as_deref(), Some("House"));
    }

    #[test]
    fn test_clear_fields_none_tracks_when_empty() {
        let cm = ChangeManager::new();
        let (affected, total) = cm.clear_fields(None, &["genre".into()]);
        assert_eq!(affected, 0);
        assert_eq!(total, 0);
    }

    // -----------------------------------------------------------------------
    // apply_snapshot independence from staged state
    // -----------------------------------------------------------------------

    #[test]
    fn test_apply_snapshot_ignores_current_staged_state() {
        let cm = ChangeManager::new();
        let tracks = vec![make_track("t1", "House", 3)];

        // Stage a genre change in the live state
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("Techno".into()),
            ..Default::default()
        }]);

        // apply_snapshot uses a separate snapshot, not the staged state
        let snapshot = vec![TrackChange {
            track_id: "t1".into(),
            comments: Some("from snapshot".into()),
            ..Default::default()
        }];
        let modified = cm.apply_snapshot(&tracks, &snapshot);
        assert_eq!(modified[0].comments, "from snapshot");
        // Genre should be unchanged (snapshot didn't touch it, staged state is irrelevant)
        assert_eq!(modified[0].genre, "House");

        // Staged state should still be intact
        assert_eq!(cm.pending_count(), 1);
    }

    // -----------------------------------------------------------------------
    // take(None) empty state
    // -----------------------------------------------------------------------

    #[test]
    fn test_take_none_when_empty() {
        let cm = ChangeManager::new();
        let taken = cm.take(None);
        assert!(taken.is_empty());
    }

    // -----------------------------------------------------------------------
    // restore when nothing was taken
    // -----------------------------------------------------------------------

    #[test]
    fn test_restore_empty_snapshot() {
        let cm = ChangeManager::new();
        cm.stage(vec![TrackChange {
            track_id: "t1".into(),
            genre: Some("House".into()),
            ..Default::default()
        }]);

        let (restored, total) = cm.restore(vec![]);
        assert_eq!(restored, 0);
        assert_eq!(total, 1);

        // Staged data should be unaffected
        assert_eq!(cm.pending_count(), 1);
    }
}
