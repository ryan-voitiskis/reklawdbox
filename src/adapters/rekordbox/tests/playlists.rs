use super::super::*;
use super::support::create_test_db;
use crate::domain::library::FileKind;

#[test]
fn rekordbox_playlists_test_get_playlists() {
    let conn = create_test_db();
    let playlists = get_playlists(&conn).unwrap();
    assert_eq!(playlists.len(), 2);
    let deep_cuts = playlists.iter().find(|p| p.name == "Deep Cuts").unwrap();
    assert_eq!(deep_cuts.track_count, 2);
    assert!(!deep_cuts.is_folder);
    let folders = playlists.iter().find(|p| p.name == "Folders").unwrap();
    assert!(folders.is_folder);
}

#[test]
fn rekordbox_playlists_test_get_playlists_track_count_excludes_deleted_tracks() {
    let conn = create_test_db();
    conn.execute(
        "UPDATE djmdContent SET rb_local_deleted = 1 WHERE ID = 't3'",
        [],
    )
    .expect("fixture update should succeed");

    let playlists = get_playlists(&conn).expect("playlist query should succeed");
    let deep_cuts = playlists
        .iter()
        .find(|p| p.id == "p1")
        .expect("fixture playlist should exist");
    assert_eq!(deep_cuts.track_count, 1);

    let tracks = get_playlist_tracks(&conn, "p1", None).expect("playlist tracks should load");
    assert_eq!(tracks.len(), 1);
}

#[test]
fn rekordbox_playlists_test_get_playlist_tracks() {
    let conn = create_test_db();
    let tracks = get_playlist_tracks(&conn, "p1", None).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].title, "Archangel");
    assert_eq!(tracks[0].position, Some(1));
    assert_eq!(tracks[1].title, "R.I.P.");
    assert_eq!(tracks[1].position, Some(2));

    assert_eq!(tracks[0].file_kind, FileKind::Flac);
}

#[test]
fn rekordbox_playlists_test_get_playlist_tracks_bounded_limit_and_offset() {
    let conn = create_test_db();
    let tracks = get_playlist_tracks_page(&conn, "p1", Some(1), Some(1)).unwrap();
    let default_limit = get_playlist_tracks_page(&conn, "p1", None, Some(1)).unwrap();

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[0].position, Some(2));
    assert_eq!(default_limit.len(), 1);
    assert_eq!(default_limit[0].id, "t3");
    assert_eq!(default_limit[0].position, Some(2));
}

#[test]
fn rekordbox_playlists_test_get_playlist_tracks_unbounded_limit_and_offset() {
    let conn = create_test_db();
    let tracks = get_playlist_tracks_unbounded_page(&conn, "p1", Some(1), Some(1)).unwrap();
    let old_wrapper = get_playlist_tracks_unbounded(&conn, "p1", None).unwrap();

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[0].position, Some(2));
    assert_eq!(old_wrapper.len(), 2);
    assert_eq!(old_wrapper[0].position, Some(1));
    assert_eq!(old_wrapper[1].position, Some(2));
}

#[test]
fn rekordbox_playlists_test_get_playlist_tracks_unbounded_offset_without_limit() {
    let conn = create_test_db();
    let tracks = get_playlist_tracks_unbounded_page(&conn, "p1", None, Some(1)).unwrap();

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[0].position, Some(2));
}

#[test]
fn rekordbox_playlists_test_get_playlist_tracks_zero_and_beyond_end_offsets() {
    let conn = create_test_db();
    let from_start = get_playlist_tracks_unbounded_page(&conn, "p1", Some(1), Some(0)).unwrap();
    let beyond = get_playlist_tracks_unbounded_page(&conn, "p1", Some(10), Some(10)).unwrap();

    assert_eq!(from_start.len(), 1);
    assert_eq!(from_start[0].id, "t1");
    assert_eq!(from_start[0].position, Some(1));
    assert!(beyond.is_empty());
}

#[test]
fn rekordbox_playlists_pending_batch_page_playlist_equal_positions_use_stable_id_tiebreaker() {
    let conn = create_test_db();
    conn.execute(
        "INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) \
         VALUES ('sp-tie', 'p1', 't2', 1)",
        [],
    )
    .unwrap();

    let first = get_playlist_tracks_unbounded_page(&conn, "p1", Some(1), Some(0)).unwrap();
    let second = get_playlist_tracks_unbounded_page(&conn, "p1", Some(1), Some(1)).unwrap();
    let repeated = get_playlist_tracks_unbounded_page(&conn, "p1", Some(2), Some(0)).unwrap();
    assert_eq!(first[0].id, "t1");
    assert_eq!(second[0].id, "t2");
    assert_eq!(
        repeated
            .iter()
            .map(|track| track.id.as_str())
            .collect::<Vec<_>>(),
        ["t1", "t2"]
    );
}

#[test]
fn rekordbox_playlists_test_count_playlist_tracks() {
    let conn = create_test_db();
    assert_eq!(count_playlist_tracks(&conn, "p1").unwrap(), 2);

    // The count must agree with an unbounded read, so a paged caller can tell
    // a complete playlist from a truncated page.
    let all = get_playlist_tracks_unbounded(&conn, "p1", None).unwrap();
    assert_eq!(count_playlist_tracks(&conn, "p1").unwrap(), all.len());

    // Soft-deleted tracks are excluded, matching get_playlist_tracks.
    conn.execute(
        "UPDATE djmdContent SET rb_local_deleted = 1 WHERE ID = 't3'",
        [],
    )
    .expect("fixture update should succeed");
    assert_eq!(count_playlist_tracks(&conn, "p1").unwrap(), 1);

    // Unknown playlist is zero, not an error.
    assert_eq!(count_playlist_tracks(&conn, "nope").unwrap(), 0);
}

#[test]
fn rekordbox_playlists_test_paging_covers_every_track_exactly_once() {
    let conn = create_test_db();
    let total = count_playlist_tracks(&conn, "p1").unwrap();

    // Walk the playlist one track at a time and confirm the union is the whole
    // playlist, in order, with no gaps or repeats — the property a caller
    // rebuilding a >200-track playlist depends on.
    let mut seen = Vec::new();
    let mut offset = 0u32;
    while (seen.len() as u32) < total as u32 {
        let page = get_playlist_tracks_page(&conn, "p1", Some(1), Some(offset)).unwrap();
        if page.is_empty() {
            break;
        }
        seen.extend(page.iter().map(|t| t.id.clone()));
        offset += page.len() as u32;
    }
    let expected: Vec<String> = get_playlist_tracks_unbounded(&conn, "p1", None)
        .unwrap()
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), total);
}
