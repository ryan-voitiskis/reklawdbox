use super::super::*;
use super::support::create_test_db;

// ==================== Library health query tests ====================

#[test]
fn rekordbox_health_test_tracks_not_in_any_playlist() {
    let conn = create_test_db();
    let search = SearchParams {
        exclude_samples: true,
        ..Default::default()
    };
    let result = tracks_not_in_any_playlist(&conn, &search).unwrap();
    assert_eq!(result.total_tracks, 6);
    assert_eq!(result.uncovered_count, 4);
    assert_eq!(result.tracks.len(), 4);
    let ids: Vec<&str> = result.tracks.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"t2"));
    assert!(ids.contains(&"t4"));
    assert!(ids.contains(&"t5"));
    assert!(ids.contains(&"t7"));
}

#[test]
fn rekordbox_health_test_tracks_not_in_any_playlist_with_samples() {
    let conn = create_test_db();
    let search = SearchParams::default();
    let result = tracks_not_in_any_playlist(&conn, &search).unwrap();
    assert_eq!(result.total_tracks, 7);
    assert_eq!(result.uncovered_count, 5);
}

#[test]
fn rekordbox_health_test_tracks_not_in_any_playlist_excludes_deleted() {
    let conn = create_test_db();
    conn.execute(
        "UPDATE djmdContent SET rb_local_deleted = 1 WHERE ID = 't2'",
        [],
    )
    .unwrap();
    let search = SearchParams {
        exclude_samples: true,
        ..Default::default()
    };
    let result = tracks_not_in_any_playlist(&conn, &search).unwrap();
    assert_eq!(result.total_tracks, 5);
    assert_eq!(result.uncovered_count, 3);
    assert!(!result.tracks.iter().any(|t| t.id == "t2"));
}

#[test]
fn rekordbox_health_test_all_track_paths() {
    let conn = create_test_db();
    let entries = all_track_paths(&conn, None).unwrap();
    assert_eq!(entries.len(), 6);
    let t1 = entries.iter().find(|e| e.id == "t1").unwrap();
    assert_eq!(t1.artist, "Burial");
    assert_eq!(t1.title, "Archangel");
    assert!(t1.path.contains("Archangel"));

    let entries = all_track_paths(&conn, Some("/Users/testuser/Music/Burial/")).unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn rekordbox_health_test_find_metadata_duplicates_none() {
    let conn = create_test_db();
    let groups = find_metadata_duplicates(&conn, None, None).unwrap();
    assert!(groups.is_empty());
}

#[test]
fn rekordbox_health_test_find_metadata_duplicates_with_dup() {
    let conn = create_test_db();
    conn.execute(
        "INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at) \
         VALUES ('t1_dup', 'Archangel', 'a1', 'g1', 13950, 240, '/Users/testuser/Music/Burial/Archangel_copy.flac', 320, 44100, 5, '2023-05-01')",
        [],
    ).unwrap();

    let groups = find_metadata_duplicates(&conn, None, None).unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].title, "archangel");
    assert_eq!(groups[0].artist, "burial");
    assert!(groups[0].track_ids.contains(&"t1".to_string()));
    assert!(groups[0].track_ids.contains(&"t1_dup".to_string()));
}

#[test]
fn rekordbox_health_test_find_metadata_duplicates_case_insensitive() {
    let conn = create_test_db();
    conn.execute(
        "INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at) \
         VALUES ('t1_dup', '  ARCHANGEL  ', 'a1', 'g1', 13950, 240, '/Users/testuser/Music/Burial/Archangel_copy.flac', 320, 44100, 5, '2023-05-01')",
        [],
    ).unwrap();

    let groups = find_metadata_duplicates(&conn, None, None).unwrap();
    assert_eq!(groups.len(), 1);
    assert!(groups[0].track_ids.contains(&"t1".to_string()));
    assert!(groups[0].track_ids.contains(&"t1_dup".to_string()));
}

#[test]
fn rekordbox_health_scan_duplicates_page_metadata_reports_total_and_stable_adjacent_pages() {
    let conn = create_test_db();
    conn.execute_batch(
        "INSERT INTO djmdContent (ID, Title, ArtistID, FolderPath, FileType) \
         VALUES ('t1_dup', 'Archangel', 'a1', '/tmp/archangel-copy.flac', 5); \
         INSERT INTO djmdContent (ID, Title, ArtistID, FolderPath, FileType) \
         VALUES ('t2_dup', 'Endorphin', 'a1', '/tmp/endorphin-copy.flac', 5);",
    )
    .unwrap();

    let first = find_metadata_duplicates_page(&conn, None, 1, 0).unwrap();
    let second = find_metadata_duplicates_page(&conn, None, 1, 1).unwrap();
    let repeated = find_metadata_duplicates_page(&conn, None, 1, 0).unwrap();

    assert_eq!(first.total, 2);
    assert_eq!(second.total, 2);
    assert_eq!(first.groups.len(), 1);
    assert_eq!(second.groups.len(), 1);
    assert_eq!(first.groups[0].title, "archangel");
    assert_eq!(second.groups[0].title, "endorphin");
    assert_eq!(first.groups[0].track_ids, ["t1", "t1_dup"]);
    assert_eq!(
        repeated.groups[0].track_ids, first.groups[0].track_ids,
        "identical pages should retain stable group and track order"
    );

    let zero = find_metadata_duplicates_page(&conn, None, 0, 0).unwrap();
    let beyond = find_metadata_duplicates_page(&conn, None, 50, 50).unwrap();
    assert_eq!(zero.total, 2);
    assert!(zero.groups.is_empty());
    assert_eq!(beyond.total, 2);
    assert!(beyond.groups.is_empty());
}

#[test]
fn rekordbox_health_test_playlist_membership_counts() {
    let conn = create_test_db();
    let ids = vec!["t1".to_string(), "t2".to_string(), "t3".to_string()];
    let counts = playlist_membership_counts(&conn, &ids).unwrap();
    assert_eq!(counts.get("t1"), Some(&1));
    assert_eq!(counts.get("t3"), Some(&1));
    assert_eq!(counts.get("t2"), None);
}
