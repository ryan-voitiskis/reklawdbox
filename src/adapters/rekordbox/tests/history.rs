use super::super::*;
use super::support::create_test_db;

// ==================== History / play stats tests ====================

#[test]
fn rekordbox_history_test_get_sessions() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, None).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "h1");
    assert_eq!(sessions[1].id, "h2");
    assert_eq!(sessions[0].track_count, 3);
    assert_eq!(sessions[1].track_count, 1);
}

#[test]
fn rekordbox_history_test_get_sessions_after_filter() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, Some("2025-03-01")).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "h1");
}

#[test]
fn rekordbox_history_test_get_sessions_limit() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, Some(1), None).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "h1");
}

#[test]
fn rekordbox_history_test_get_sessions_duration() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, None).unwrap();
    let h1 = &sessions[0];
    assert_eq!(h1.duration_seconds, Some(900));
    let h2 = &sessions[1];
    assert_eq!(h2.duration_seconds, None);
}

#[test]
fn rekordbox_history_test_get_session_tracks() {
    let conn = create_test_db();
    let tracks = get_session_tracks(&conn, "h1").unwrap();
    assert_eq!(tracks.len(), 3);
    assert_eq!(tracks[0].title, "Archangel");
    assert_eq!(tracks[0].position, Some(1));
    assert_eq!(tracks[0].played_at.as_deref(), Some("2025-03-01 22:00:00"));
    assert_eq!(tracks[1].title, "R.I.P.");
    assert_eq!(tracks[1].position, Some(2));
    assert_eq!(tracks[1].played_at.as_deref(), Some("2025-03-01 22:05:00"));
    assert_eq!(tracks[2].title, "Endorphin");
    assert_eq!(tracks[2].position, Some(3));
    assert_eq!(tracks[2].played_at.as_deref(), Some("2025-03-01 22:10:00"));
}

#[test]
fn rekordbox_history_test_get_session_tracks_not_found() {
    let conn = create_test_db();
    let tracks = get_session_tracks(&conn, "nonexistent").unwrap();
    assert!(tracks.is_empty());
}

#[test]
fn rekordbox_history_test_get_play_stats_counts() {
    let conn = create_test_db();
    let params = SearchParams::default();
    let stats = get_play_stats(&conn, &params, false, None).unwrap();
    let t1 = stats.iter().find(|s| s.track_id == "t1").unwrap();
    assert_eq!(t1.play_count, 2);
    assert_eq!(t1.session_count, 2);
    let t3 = stats.iter().find(|s| s.track_id == "t3").unwrap();
    assert_eq!(t3.play_count, 1);
    assert_eq!(t3.session_count, 1);
}

#[test]
fn rekordbox_history_test_get_play_stats_with_genre_filter() {
    let conn = create_test_db();
    let params = SearchParams {
        genre: Some("Dubstep".to_string()),
        ..Default::default()
    };
    let stats = get_play_stats(&conn, &params, false, None).unwrap();
    assert_eq!(stats.len(), 2);
    assert!(
        stats
            .iter()
            .all(|s| s.track_id == "t1" || s.track_id == "t2")
    );
}

#[test]
fn rekordbox_history_test_get_play_stats_include_unplayed() {
    let conn = create_test_db();
    let params = SearchParams {
        exclude_samples: true,
        ..Default::default()
    };
    let stats = get_play_stats(&conn, &params, true, None).unwrap();
    let unplayed: Vec<_> = stats.iter().filter(|s| s.play_count == 0).collect();
    assert_eq!(unplayed.len(), 3);
    assert!(unplayed.iter().any(|s| s.track_id == "t4"));
    assert!(unplayed.iter().any(|s| s.track_id == "t5"));
    assert!(unplayed.iter().any(|s| s.track_id == "t7"));
}

#[test]
fn rekordbox_history_test_get_play_stats_session_ids() {
    let conn = create_test_db();
    let params = SearchParams::default();
    let stats = get_play_stats(&conn, &params, false, None).unwrap();
    let t1 = stats.iter().find(|s| s.track_id == "t1").unwrap();
    assert_eq!(t1.session_ids.len(), 2);
    assert!(t1.session_ids.contains(&"h1".to_string()));
    assert!(t1.session_ids.contains(&"h2".to_string()));
}
