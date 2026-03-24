use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use crate::color;
use crate::types::{EditableField, FieldDiff, Track, TrackChange, TrackDiff};

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
            .unwrap_or_else(|e| e.into_inner())
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

            if let Some(ref new_genre) = change.genre
                && *new_genre != track.genre
            {
                field_diffs.push(FieldDiff {
                    field: "genre".to_string(),
                    old_value: track.genre.clone(),
                    new_value: new_genre.clone(),
                });
            }

            if let Some(ref new_comments) = change.comments
                && *new_comments != track.comments
            {
                field_diffs.push(FieldDiff {
                    field: "comments".to_string(),
                    old_value: track.comments.clone(),
                    new_value: new_comments.clone(),
                });
            }

            if let Some(new_rating) = change.rating
                && new_rating != track.rating
            {
                field_diffs.push(FieldDiff {
                    field: "rating".to_string(),
                    old_value: track.rating.to_string(),
                    new_value: new_rating.to_string(),
                });
            }

            if let Some(ref new_color) = change.color
                && *new_color != track.color
            {
                field_diffs.push(FieldDiff {
                    field: "color".to_string(),
                    old_value: track.color.clone(),
                    new_value: new_color.clone(),
                });
            }

            if let Some(ref new_label) = change.label
                && *new_label != track.label
            {
                field_diffs.push(FieldDiff {
                    field: "label".to_string(),
                    old_value: track.label.clone(),
                    new_value: new_label.clone(),
                });
            }

            if let Some(new_year) = change.year
                && new_year != track.year
            {
                field_diffs.push(FieldDiff {
                    field: "year".to_string(),
                    old_value: track.year.to_string(),
                    new_value: new_year.to_string(),
                });
            }

            if let Some(ref new_album) = change.album
                && *new_album != track.album
            {
                field_diffs.push(FieldDiff {
                    field: "album".to_string(),
                    old_value: track.album.clone(),
                    new_value: new_album.clone(),
                });
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

    /// Restore previously taken changes without overwriting fields the user
    /// touched (staged or cleared) since the snapshot was taken.
    pub fn restore(&self, snapshot: Vec<TrackChange>) -> (usize, usize) {
        // If clear(None) was called since the last take(), the user expressed
        // intent to wipe everything — don't resurrect any snapshot fields.
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

        // Resolve field names once.
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
                    if clear_field(entry, ef) {
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

fn has_any_staged_field(change: &TrackChange) -> bool {
    change.genre.is_some()
        || change.comments.is_some()
        || change.rating.is_some()
        || change.color.is_some()
        || change.label.is_some()
        || change.year.is_some()
        || change.album.is_some()
}

fn merge_track_change(existing: &mut TrackChange, incoming: &TrackChange) {
    if incoming.genre.is_some() {
        existing.genre = incoming.genre.clone();
    }
    if incoming.comments.is_some() {
        existing.comments = incoming.comments.clone();
    }
    if incoming.rating.is_some() {
        existing.rating = incoming.rating;
    }
    if incoming.color.is_some() {
        existing.color = incoming.color.clone();
    }
    if incoming.label.is_some() {
        existing.label = incoming.label.clone();
    }
    if incoming.year.is_some() {
        existing.year = incoming.year;
    }
    if incoming.album.is_some() {
        existing.album = incoming.album.clone();
    }
}

/// Fill missing fields from `incoming` into `existing`, skipping any field
/// present in `touched`.
fn merge_untouched_fields(
    existing: &mut TrackChange,
    incoming: &TrackChange,
    touched: &HashSet<EditableField>,
) {
    if existing.genre.is_none() && !touched.contains(&EditableField::Genre) {
        existing.genre = incoming.genre.clone();
    }
    if existing.comments.is_none() && !touched.contains(&EditableField::Comments) {
        existing.comments = incoming.comments.clone();
    }
    if existing.rating.is_none() && !touched.contains(&EditableField::Rating) {
        existing.rating = incoming.rating;
    }
    if existing.color.is_none() && !touched.contains(&EditableField::Color) {
        existing.color = incoming.color.clone();
    }
    if existing.label.is_none() && !touched.contains(&EditableField::Label) {
        existing.label = incoming.label.clone();
    }
    if existing.year.is_none() && !touched.contains(&EditableField::Year) {
        existing.year = incoming.year;
    }
    if existing.album.is_none() && !touched.contains(&EditableField::Album) {
        existing.album = incoming.album.clone();
    }
}

/// Record which fields are set in `change` as touched for its track_id.
fn record_touched_fields(change: &TrackChange, touched: &mut TouchedFields) {
    if !has_any_staged_field(change) {
        return;
    }
    let set = touched.entry(change.track_id.clone()).or_default();
    if change.genre.is_some() {
        set.insert(EditableField::Genre);
    }
    if change.comments.is_some() {
        set.insert(EditableField::Comments);
    }
    if change.rating.is_some() {
        set.insert(EditableField::Rating);
    }
    if change.color.is_some() {
        set.insert(EditableField::Color);
    }
    if change.label.is_some() {
        set.insert(EditableField::Label);
    }
    if change.year.is_some() {
        set.insert(EditableField::Year);
    }
    if change.album.is_some() {
        set.insert(EditableField::Album);
    }
}

/// Clear a single field on a `TrackChange`, returning true if the field was set.
fn clear_field(entry: &mut TrackChange, field: EditableField) -> bool {
    match field {
        EditableField::Genre if entry.genre.is_some() => {
            entry.genre = None;
            true
        }
        EditableField::Comments if entry.comments.is_some() => {
            entry.comments = None;
            true
        }
        EditableField::Rating if entry.rating.is_some() => {
            entry.rating = None;
            true
        }
        EditableField::Color if entry.color.is_some() => {
            entry.color = None;
            true
        }
        EditableField::Label if entry.label.is_some() => {
            entry.label = None;
            true
        }
        EditableField::Year if entry.year.is_some() => {
            entry.year = None;
            true
        }
        EditableField::Album if entry.album.is_some() => {
            entry.album = None;
            true
        }
        _ => false,
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
                if let Some(ref genre) = change.genre {
                    modified.genre = genre.clone();
                }
                if let Some(ref comments) = change.comments {
                    modified.comments = comments.clone();
                }
                if let Some(rating) = change.rating {
                    modified.rating = rating;
                }
                if let Some(ref color) = change.color {
                    modified.color = color.clone();
                    modified.color_code = color::color_name_to_code(color).unwrap_or(0);
                }
                if let Some(ref label) = change.label {
                    modified.label = label.clone();
                }
                if let Some(year) = change.year {
                    modified.year = year;
                }
                if let Some(ref album) = change.album {
                    modified.album = album.clone();
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
    use crate::types::FileKind;

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
        assert_eq!(diffs.len(), 1); // one track with changes
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
        assert!(diffs.is_empty()); // no actual change
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
        assert_eq!(modified[1].genre, "Techno"); // unchanged
        assert_eq!(modified[1].rating, 2); // unchanged
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

        // Clear just the color field
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

        // Clear genre from all tracks (no track_ids filter)
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
        assert_eq!(diffs.len(), 2); // two tracks
        // Sorted by track_id
        assert_eq!(diffs[0].track_id, "t1");
        assert_eq!(diffs[0].changes.len(), 2); // genre + rating
        assert_eq!(diffs[1].track_id, "t2");
        assert_eq!(diffs[1].changes.len(), 1); // comments
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

        // Stage a genre change only, no color change
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

    // ==================== Integration tests (real DB) ====================

    #[test]
    #[ignore]
    fn test_real_change_pipeline() {
        let conn = crate::db::open_real_db().expect("backup tarball not found");

        // 1. Search for some tracks
        let params = crate::db::SearchParams {
            query: None,
            artist: None,
            genre: None,
            rating_min: None,
            bpm_min: Some(120.0),
            bpm_max: Some(130.0),
            key: None,
            playlist: None,
            has_genre: None,
            has_label: None,
            year_zero: None,
            label: None,
            path: None,
            path_prefix: None,
            added_after: None,
            added_before: None,
            exclude_samples: false,
            limit: Some(5),
            offset: None,
        };
        let tracks = crate::db::search_tracks(&conn, &params).unwrap();
        assert!(!tracks.is_empty(), "need tracks for pipeline test");

        let track = &tracks[0];

        // 2. Stage changes
        let cm = ChangeManager::new();
        let (staged, total) = cm.stage(vec![TrackChange {
            track_id: track.id.clone(),
            genre: Some("Deep House".to_string()),
            comments: Some("integration test".to_string()),
            rating: Some(4),
            color: None,
            label: None,
            year: None,
            album: None,
        }]);
        assert_eq!(staged, 1);
        assert_eq!(total, 1);

        // 3. Preview changes
        let diffs = cm.preview(&tracks);
        assert!(!diffs.is_empty(), "expected diffs for staged changes");
        let td = &diffs[0];
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "genre" && f.new_value == "Deep House")
        );
        assert!(
            td.changes
                .iter()
                .any(|f| f.field == "comments" && f.new_value == "integration test")
        );

        // 4. Apply changes
        let modified = cm.apply_changes(&tracks);
        let modified_track = modified.iter().find(|t| t.id == track.id).unwrap();
        assert_eq!(modified_track.genre, "Deep House");
        assert_eq!(modified_track.comments, "integration test");
        assert_eq!(modified_track.rating, 4);

        // 5. Generate XML from modified tracks
        let xml = crate::xml::generate_xml(&modified);
        assert!(xml.contains("Genre=\"Deep House\""));
        assert!(xml.contains("Comments=\"integration test\""));
        assert!(xml.contains("Rating=\"204\"")); // 4 stars = 204

        // 6. Verify unmodified tracks are unchanged
        for t in &modified {
            if t.id != track.id {
                let original = tracks.iter().find(|o| o.id == t.id).unwrap();
                assert_eq!(t.genre, original.genre);
                assert_eq!(t.comments, original.comments);
                assert_eq!(t.rating, original.rating);
            }
        }
    }
}
