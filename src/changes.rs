use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use crate::color;
use crate::types::{EditableField, FieldDiff, Track, TrackChange, TrackDiff};

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<T> {
    mutex.lock().unwrap_or_else(|e| {
        eprintln!("[changes] mutex poisoned, recovering");
        e.into_inner()
    })
}

pub struct ChangeManager {
    changes: Mutex<HashMap<String, TrackChange>>,
}

impl ChangeManager {
    pub fn new() -> Self {
        Self {
            changes: Mutex::new(HashMap::new()),
        }
    }

    /// Stage changes for one or more tracks. Merges with previously staged changes for the same track.
    pub fn stage(&self, changes: Vec<TrackChange>) -> (usize, usize) {
        let mut map = lock_or_recover(&self.changes);
        let mut staged = 0;
        for change in changes {
            if !has_any_staged_field(&change) {
                continue;
            }
            staged += 1;
            map.entry(change.track_id.clone())
                .and_modify(|existing| merge_track_change(existing, &change))
                .or_insert(change);
        }
        (staged, map.len())
    }

    pub fn pending_ids(&self) -> Vec<String> {
        let map = lock_or_recover(&self.changes);
        let mut ids: Vec<String> = map.keys().cloned().collect();
        ids.sort();
        ids
    }

    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        lock_or_recover(&self.changes).len()
    }

    pub fn get(&self, track_id: &str) -> Option<TrackChange> {
        self.changes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(track_id)
            .cloned()
    }

    pub fn preview(&self, current_tracks: &[Track]) -> Vec<TrackDiff> {
        let map = lock_or_recover(&self.changes);
        let track_map: HashMap<&str, &Track> =
            current_tracks.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut result = Vec::new();

        for (track_id, change) in map.iter() {
            let Some(track) = track_map.get(track_id.as_str()) else {
                continue;
            };

            let mut fields = Vec::new();

            if let Some(ref new_genre) = change.genre
                && *new_genre != track.genre
            {
                fields.push(FieldDiff {
                    field: "genre".to_string(),
                    old_value: track.genre.clone(),
                    new_value: new_genre.clone(),
                });
            }

            if let Some(ref new_comments) = change.comments
                && *new_comments != track.comments
            {
                fields.push(FieldDiff {
                    field: "comments".to_string(),
                    old_value: track.comments.clone(),
                    new_value: new_comments.clone(),
                });
            }

            if let Some(new_rating) = change.rating
                && new_rating != track.rating
            {
                fields.push(FieldDiff {
                    field: "rating".to_string(),
                    old_value: track.rating.to_string(),
                    new_value: new_rating.to_string(),
                });
            }

            if let Some(ref new_color) = change.color
                && *new_color != track.color
            {
                fields.push(FieldDiff {
                    field: "color".to_string(),
                    old_value: track.color.clone(),
                    new_value: new_color.clone(),
                });
            }

            if !fields.is_empty() {
                fields.sort_by(|a, b| a.field.cmp(&b.field));
                result.push(TrackDiff {
                    track_id: track_id.clone(),
                    title: track.title.clone(),
                    artist: track.artist.clone(),
                    changes: fields,
                });
            }
        }

        result.sort_by(|a, b| a.track_id.cmp(&b.track_id));
        result
    }

    #[cfg(test)]
    pub fn apply_changes(&self, tracks: &[Track]) -> Vec<Track> {
        let map = lock_or_recover(&self.changes);
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
    pub fn take(&self, track_ids: Option<Vec<String>>) -> Vec<TrackChange> {
        let mut map = lock_or_recover(&self.changes);
        match track_ids {
            Some(ids) => ids.into_iter().filter_map(|id| map.remove(&id)).collect(),
            None => {
                let mut drained: Vec<TrackChange> = map.drain().map(|(_, change)| change).collect();
                drained.sort_by(|a, b| a.track_id.cmp(&b.track_id));
                drained
            }
        }
    }

    /// Restore previously taken changes without overwriting newer staged values.
    pub fn restore(&self, snapshot: Vec<TrackChange>) -> (usize, usize) {
        let mut map = lock_or_recover(&self.changes);
        let restored = snapshot.len();
        for change in snapshot {
            map.entry(change.track_id.clone())
                .and_modify(|existing| merge_missing_fields(existing, &change))
                .or_insert(change);
        }
        (restored, map.len())
    }

    /// Clear specific fields from staged changes. Returns (tracks_affected, remaining_tracks).
    /// If all fields on a track become None, the entry is removed entirely.
    pub fn clear_fields(
        &self,
        track_ids: Option<Vec<String>>,
        fields: &[String],
    ) -> (usize, usize) {
        let mut map = lock_or_recover(&self.changes);
        let target_ids: Vec<String> = match track_ids {
            Some(ids) => ids,
            None => map.keys().cloned().collect(),
        };

        let mut affected = 0;
        for id in &target_ids {
            if let Some(entry) = map.get_mut(id) {
                let mut touched = false;
                for field in fields {
                    match EditableField::from_str(field.as_str()) {
                        Some(EditableField::Genre) if entry.genre.is_some() => {
                            entry.genre = None;
                            touched = true;
                        }
                        Some(EditableField::Comments) if entry.comments.is_some() => {
                            entry.comments = None;
                            touched = true;
                        }
                        Some(EditableField::Rating) if entry.rating.is_some() => {
                            entry.rating = None;
                            touched = true;
                        }
                        Some(EditableField::Color) if entry.color.is_some() => {
                            entry.color = None;
                            touched = true;
                        }
                        _ => {}
                    }
                }
                if touched {
                    affected += 1;
                }
                // Remove entry if all fields are now None
                if entry.genre.is_none()
                    && entry.comments.is_none()
                    && entry.rating.is_none()
                    && entry.color.is_none()
                {
                    map.remove(id);
                }
            }
        }
        (affected, map.len())
    }

    pub fn clear(&self, track_ids: Option<Vec<String>>) -> (usize, usize) {
        let mut map = lock_or_recover(&self.changes);
        let cleared = match track_ids {
            Some(ids) => {
                let mut count = 0;
                for id in ids {
                    if map.remove(&id).is_some() {
                        count += 1;
                    }
                }
                count
            }
            None => {
                let count = map.len();
                map.clear();
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
}

fn merge_missing_fields(existing: &mut TrackChange, incoming: &TrackChange) {
    if existing.genre.is_none() {
        existing.genre = incoming.genre.clone();
    }
    if existing.comments.is_none() {
        existing.comments = incoming.comments.clone();
    }
    if existing.rating.is_none() {
        existing.rating = incoming.rating;
    }
    if existing.color.is_none() {
        existing.color = incoming.color.clone();
    }
}

fn apply_changes_with_map(tracks: &[Track], map: &HashMap<String, TrackChange>) -> Vec<Track> {
    tracks
        .iter()
        .map(|track| {
            if let Some(change) = map.get(&track.id) {
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
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
        }]);
        // Second stage for same track: genre updates, rating preserved from first stage
        cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: Some("Deep House".to_string()),
            comments: Some("great track".to_string()),
            rating: None,
            color: None,
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
        }]);

        let (staged, total) = cm.stage(vec![TrackChange {
            track_id: "t1".to_string(),
            genre: None,
            comments: None,
            rating: None,
            color: None,
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("B".to_string()),
                comments: None,
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("B".to_string()),
                comments: None,
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: None,
                comments: Some("nice track".to_string()),
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t2".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
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
            },
            TrackChange {
                track_id: "t1".to_string(),
                genre: Some("Techno".to_string()),
                comments: None,
                rating: None,
                color: None,
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
        }]);

        cm.restore(snapshot);
        let restored = cm.get("t1").expect("t1 should be restored");
        assert_eq!(restored.genre.as_deref(), Some("House"));
        assert_eq!(restored.comments.as_deref(), Some("new"));
        assert_eq!(restored.rating, Some(5));
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
            label: None,
            path: None,
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
