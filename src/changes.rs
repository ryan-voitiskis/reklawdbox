use std::collections::HashMap;
use std::sync::Mutex;

use crate::types::{ChangeDiff, Track, TrackChange};

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
        let mut map = self.changes.lock().unwrap_or_else(|e| e.into_inner());
        let staged = changes.len();
        for change in changes {
            map.entry(change.track_id.clone())
                .and_modify(|existing| {
                    if change.genre.is_some() {
                        existing.genre = change.genre.clone();
                    }
                    if change.comments.is_some() {
                        existing.comments = change.comments.clone();
                    }
                    if change.rating.is_some() {
                        existing.rating = change.rating;
                    }
                    if change.color.is_some() {
                        existing.color = change.color.clone();
                    }
                })
                .or_insert(change);
        }
        (staged, map.len())
    }

    pub fn pending_ids(&self) -> Vec<String> {
        let map = self.changes.lock().unwrap_or_else(|e| e.into_inner());
        map.keys().cloned().collect()
    }

    pub fn pending_count(&self) -> usize {
        self.changes.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn get(&self, track_id: &str) -> Option<TrackChange> {
        self.changes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(track_id)
            .cloned()
    }

    pub fn preview(&self, current_tracks: &[Track]) -> Vec<ChangeDiff> {
        let map = self.changes.lock().unwrap_or_else(|e| e.into_inner());
        let track_map: HashMap<&str, &Track> =
            current_tracks.iter().map(|t| (t.id.as_str(), t)).collect();

        let mut diffs = Vec::new();

        for (track_id, change) in map.iter() {
            let Some(track) = track_map.get(track_id.as_str()) else {
                continue;
            };

            if let Some(ref new_genre) = change.genre {
                if *new_genre != track.genre {
                    diffs.push(ChangeDiff {
                        track_id: track_id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        field: "genre".to_string(),
                        old_value: track.genre.clone(),
                        new_value: new_genre.clone(),
                    });
                }
            }

            if let Some(ref new_comments) = change.comments {
                if *new_comments != track.comments {
                    diffs.push(ChangeDiff {
                        track_id: track_id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        field: "comments".to_string(),
                        old_value: track.comments.clone(),
                        new_value: new_comments.clone(),
                    });
                }
            }

            if let Some(new_rating) = change.rating {
                if new_rating != track.rating {
                    diffs.push(ChangeDiff {
                        track_id: track_id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        field: "rating".to_string(),
                        old_value: track.rating.to_string(),
                        new_value: new_rating.to_string(),
                    });
                }
            }

            if let Some(ref new_color) = change.color {
                if *new_color != track.color {
                    diffs.push(ChangeDiff {
                        track_id: track_id.clone(),
                        title: track.title.clone(),
                        artist: track.artist.clone(),
                        field: "color".to_string(),
                        old_value: track.color.clone(),
                        new_value: new_color.clone(),
                    });
                }
            }
        }

        // Sort for deterministic output
        diffs.sort_by(|a, b| a.track_id.cmp(&b.track_id).then(a.field.cmp(&b.field)));
        diffs
    }

    pub fn apply_changes(&self, tracks: &[Track]) -> Vec<Track> {
        let map = self.changes.lock().unwrap_or_else(|e| e.into_inner());
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
                    }
                    modified
                } else {
                    track.clone()
                }
            })
            .collect()
    }

    pub fn clear(&self, track_ids: Option<Vec<String>>) -> (usize, usize) {
        let mut map = self.changes.lock().unwrap_or_else(|e| e.into_inner());
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

#[cfg(test)]
mod tests {
    use super::*;

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
            file_type: 5,
            date_added: "2023-01-01".to_string(),
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
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "genre" && d.new_value == "Deep House")
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "comments" && d.new_value == "great track")
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "rating" && d.new_value == "4")
        );
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
        assert_eq!(diffs.len(), 3); // genre, comments, rating changed
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "genre" && d.new_value == "Deep House")
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "comments" && d.new_value == "great bassline")
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "rating" && d.new_value == "5")
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
            exclude_samples: false,
            limit: Some(5),
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
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "genre" && d.new_value == "Deep House")
        );
        assert!(
            diffs
                .iter()
                .any(|d| d.field == "comments" && d.new_value == "integration test")
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
