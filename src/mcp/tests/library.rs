use crate::mcp::enrichment::{
    ResolveTracksOpts, apply_offset_limit, resolve_tracks, track_has_unknown_genre,
};
use crate::mcp::library::{SearchFilterParams, SearchTracksParams, handle_search_tracks};
use std::sync::Mutex;

use rmcp::model::CallToolResult;

use crate::adapters::rekordbox as db;

use super::common::{create_selector_pagination_test_db, extract_json, track_ids};

fn selector_pagination_opts(
    default_max_tracks: Option<u32>,
    max_tracks_cap: Option<u32>,
    exclude_samplers: bool,
) -> ResolveTracksOpts {
    ResolveTracksOpts {
        default_max_tracks,
        max_tracks_cap,
        exclude_samplers,
    }
}

fn tracks_from_tool_result(result: &CallToolResult) -> Vec<crate::domain::library::Track> {
    serde_json::from_value(extract_json(result)).expect("tool result should contain tracks")
}

#[test]
fn selector_pagination_playlist_offset_preserves_position() {
    let conn = create_selector_pagination_test_db();
    let tracks = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(1),
        Some(1),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("playlist selector should resolve");

    assert_eq!(track_ids(&tracks), ["t6"]);
    assert_eq!(tracks[0].position, Some(2));
}

#[test]
fn selector_pagination_playlist_pages_do_not_repeat() {
    let conn = create_selector_pagination_test_db();
    let opts = selector_pagination_opts(Some(50), Some(200), false);
    let first = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(2),
        Some(0),
        &opts,
    )
    .expect("first playlist page should resolve");
    let second = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(2),
        Some(2),
        &opts,
    )
    .expect("second playlist page should resolve");

    assert_eq!(track_ids(&first), ["t1", "t6"]);
    assert_eq!(track_ids(&second), ["t2", "t3"]);
}

#[test]
fn selector_pagination_explicit_ids_follow_caller_order_after_deduplication() {
    let conn = create_selector_pagination_test_db();
    let ids = vec![
        "t3".to_string(),
        "t1".to_string(),
        "t1".to_string(),
        "t2".to_string(),
    ];
    let tracks = resolve_tracks(
        &conn,
        Some(&ids),
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(1),
        Some(1),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("explicit ID selector should resolve");

    assert_eq!(track_ids(&tracks), ["t1"]);
}

#[test]
fn selector_pagination_unknown_genre_search_filters_before_offset() {
    let conn = create_selector_pagination_test_db();
    let tracks = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams {
            has_unknown_genre: Some(true),
            ..Default::default()
        },
        Some(1),
        Some(1),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("unknown-genre search selector should resolve");

    assert_eq!(track_ids(&tracks), ["t4"]);
}

#[test]
fn selector_pagination_unknown_genre_playlist_filters_before_offset() {
    let conn = create_selector_pagination_test_db();
    let tracks = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams {
            has_unknown_genre: Some(true),
            ..Default::default()
        },
        Some(1),
        Some(1),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("unknown-genre playlist selector should resolve");

    assert_eq!(track_ids(&tracks), ["t2"]);
    assert_eq!(tracks[0].position, Some(3));
}

#[test]
fn selector_pagination_sampler_filter_runs_before_offset() {
    let conn = create_selector_pagination_test_db();
    let tracks = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(1),
        Some(1),
        &selector_pagination_opts(None, None, true),
    )
    .expect("sampler-excluding playlist selector should resolve");

    assert_eq!(track_ids(&tracks), ["t2"]);
    assert_eq!(tracks[0].position, Some(3));
}

#[test]
fn selector_pagination_zero_beyond_default_and_cap() {
    let conn = create_selector_pagination_test_db();
    let ids = vec!["t1".to_string(), "t2".to_string()];
    let zero = resolve_tracks(
        &conn,
        Some(&ids),
        None,
        SearchFilterParams::default(),
        Some(0),
        None,
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("zero limit should resolve");
    assert!(zero.is_empty());

    let beyond = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams {
            has_unknown_genre: Some(true),
            ..Default::default()
        },
        Some(1),
        Some(9),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("beyond-end offset should resolve");
    assert!(beyond.is_empty());

    let defaulted = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams::default(),
        None,
        None,
        &selector_pagination_opts(Some(2), Some(200), false),
    )
    .expect("default limit should resolve");
    assert_eq!(track_ids(&defaulted), ["t1", "t2"]);

    let capped = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams::default(),
        Some(10),
        None,
        &selector_pagination_opts(Some(50), Some(2), false),
    )
    .expect("capped limit should resolve");
    assert_eq!(track_ids(&capped), ["t1", "t2"]);
}

#[test]
fn selector_pagination_sql_fast_paths_apply_offset_once() {
    let conn = create_selector_pagination_test_db();
    let bounded_search = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams::default(),
        Some(2),
        Some(1),
        &selector_pagination_opts(Some(50), Some(200), false),
    )
    .expect("bounded search page should resolve");
    assert_eq!(track_ids(&bounded_search), ["t2", "t3"]);

    let unbounded_search = resolve_tracks(
        &conn,
        None,
        None,
        SearchFilterParams::default(),
        Some(2),
        Some(1),
        &selector_pagination_opts(None, None, false),
    )
    .expect("unbounded search page should resolve");
    assert_eq!(track_ids(&unbounded_search), ["t2", "t3"]);

    let unbounded_playlist = resolve_tracks(
        &conn,
        None,
        Some("selector-playlist"),
        SearchFilterParams::default(),
        Some(2),
        Some(2),
        &selector_pagination_opts(None, None, false),
    )
    .expect("unbounded playlist page should resolve");
    assert_eq!(track_ids(&unbounded_playlist), ["t2", "t3"]);
}

#[test]
fn search_tracks_unknown_genre_paginates_after_filtering() {
    let conn = Mutex::new(create_selector_pagination_test_db());
    let search = |offset, include_samples| {
        let guard = conn.lock().expect("test DB mutex should lock");
        let result = handle_search_tracks(
            guard,
            SearchTracksParams {
                filters: SearchFilterParams {
                    has_unknown_genre: Some(true),
                    ..Default::default()
                },
                playlist: Some("selector-playlist".to_string()),
                include_samples,
                limit: Some(1),
                offset: Some(offset),
            },
        )
        .expect("unknown-genre search handler should succeed");
        tracks_from_tool_result(&result)
    };

    assert_eq!(track_ids(&search(0, None)), ["t2"]);
    assert_eq!(track_ids(&search(1, None)), ["t4"]);
    assert!(search(2, None).is_empty());
    assert_eq!(track_ids(&search(2, Some(true))), ["t6"]);
}

#[test]
fn search_tracks_unknown_genre_false_retains_sql_pagination() {
    let conn = Mutex::new(create_selector_pagination_test_db());
    let result = handle_search_tracks(
        conn.lock().expect("test DB mutex should lock"),
        SearchTracksParams {
            filters: SearchFilterParams::default(),
            playlist: Some("selector-playlist".to_string()),
            include_samples: None,
            limit: Some(2),
            offset: Some(1),
        },
    )
    .expect("ordinary search handler should succeed");

    assert_eq!(track_ids(&tracks_from_tool_result(&result)), ["t2", "t3"]);
}

#[test]
fn selector_pagination_helpers_skip_then_take_without_reordering() {
    let conn = create_selector_pagination_test_db();
    let all = db::search_tracks_unbounded(&conn, &db::SearchParams::default())
        .expect("fixture tracks should load");

    assert_eq!(
        track_ids(&apply_offset_limit(all.clone(), None, None)),
        ["t1", "t2", "t3", "t4", "t5", "t6"]
    );
    assert_eq!(
        track_ids(&apply_offset_limit(all.clone(), Some(1), None)),
        ["t2", "t3", "t4", "t5", "t6"]
    );
    assert_eq!(
        track_ids(&apply_offset_limit(all.clone(), None, Some(2))),
        ["t1", "t2"]
    );
    assert_eq!(
        track_ids(&apply_offset_limit(all.clone(), Some(1), Some(2))),
        ["t2", "t3"]
    );
    assert!(apply_offset_limit(all.clone(), Some(99), Some(1)).is_empty());
    assert!(apply_offset_limit(all, Some(0), Some(0)).is_empty());
}

#[test]
fn selector_pagination_helpers_unknown_genre_predicate_is_exact() {
    let conn = create_selector_pagination_test_db();
    let mut track = db::get_track(&conn, "t1")
        .expect("fixture lookup should succeed")
        .expect("fixture track should exist");

    track.genre = "Techno".to_string();
    assert!(!track_has_unknown_genre(&track));
    track.genre = "Hip-Hop".to_string();
    assert!(!track_has_unknown_genre(&track));
    track.genre.clear();
    assert!(!track_has_unknown_genre(&track));
    track.genre = "Alien Rhythms".to_string();
    assert!(track_has_unknown_genre(&track));
}

#[test]
fn verify_cue_removal_detects_wrong_cue_and_loss_of_the_target_track() {
    use crate::mcp::library::{PreviewCueRemovalParams, VerifyCueRemovalParams, cues};
    let conn = db::test_support::create_cue_test_db();
    // A track that contains only the target must still survive its removal.
    conn.execute_batch(
        "INSERT INTO djmdContent(ID,UUID) VALUES ('solo','solo-uuid');
         INSERT INTO djmdCue(ID,ContentID,Kind,Comment) VALUES ('target','solo',0,'AUTO');",
    )
    .unwrap();
    let preview = || {
        let result = cues::preview(
            &conn,
            PreviewCueRemovalParams {
                track_id: Some("solo".into()),
                comment_exact: "AUTO".into(),
            },
        )
        .unwrap();
        extract_json(&result)
    };
    let before = preview();
    let verify = || {
        let result = cues::verify(
            &conn,
            VerifyCueRemovalParams {
                track_id: Some("solo".into()),
                comment_exact: "AUTO".into(),
                verification_fingerprint: before["verification_fingerprint"]
                    .as_str()
                    .unwrap()
                    .into(),
            },
        )
        .unwrap();
        extract_json(&result)
    };
    let pending = verify();
    assert_eq!(pending["remaining_memory_points"], 1);
    assert_eq!(pending["protected_library_matches"], true);
    assert_eq!(pending["verified"], false);

    conn.execute("DELETE FROM djmdCue WHERE ID='target'", [])
        .unwrap();
    let correct = verify();
    assert_eq!(correct["verified"], true);
    assert_eq!(correct["protected_cue_rows"], before["protected_cue_rows"]);
    assert_eq!(correct["active_tracks"], before["active_tracks"]);
    assert!(preview()["verification_fingerprint"].is_null());

    conn.execute("UPDATE djmdCue SET rb_local_deleted=1 WHERE ID='b'", [])
        .unwrap();
    let wrong_cue = verify();
    assert_eq!(wrong_cue["remaining_memory_points"], 0);
    assert_eq!(wrong_cue["protected_library_matches"], false);
    assert_eq!(wrong_cue["verified"], false);
    assert_eq!(
        wrong_cue["protected_cue_rows"].as_u64().unwrap() + 1,
        before["protected_cue_rows"]
    );
    conn.execute("UPDATE djmdCue SET rb_local_deleted=0 WHERE ID='b'", [])
        .unwrap();

    conn.execute(
        "UPDATE djmdContent SET rb_local_deleted=1 WHERE ID='solo'",
        [],
    )
    .unwrap();
    let missing_track = verify();
    assert_eq!(missing_track["verified"], false);
    assert_eq!(
        missing_track["protected_cue_rows"],
        before["protected_cue_rows"]
    );
    assert_eq!(
        missing_track["active_tracks"].as_u64().unwrap() + 1,
        before["active_tracks"]
    );
}

#[test]
fn cue_tools_validate_removal_inputs_and_cap_the_list_page() {
    use crate::mcp::library::{
        ListCuesParams, PreviewCueRemovalParams, VerifyCueRemovalParams, cues,
    };
    let conn = db::test_support::create_cue_test_db();
    assert!(
        cues::preview(
            &conn,
            PreviewCueRemovalParams {
                track_id: None,
                comment_exact: " \t".into(),
            }
        )
        .is_err()
    );
    for fingerprint in ["a".repeat(63), "A".repeat(64), "g".repeat(64)] {
        assert!(
            cues::verify(
                &conn,
                VerifyCueRemovalParams {
                    track_id: None,
                    comment_exact: "AUTO".into(),
                    verification_fingerprint: fingerprint,
                }
            )
            .is_err()
        );
    }
    conn.execute_batch(
        "WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<201)
         INSERT INTO djmdCue(ID,ContentID,Kind,Comment) SELECT 'cue-'||i,'1',0,'page' FROM n;",
    )
    .unwrap();
    let result = cues::list(
        &conn,
        ListCuesParams {
            track_id: Some("1".into()),
            comment_exact: Some("page".into()),
            memory_points_only: None,
            limit: Some(1000),
            offset: None,
        },
    )
    .unwrap();
    let page = extract_json(&result);
    assert_eq!(page["total_cues"], 201);
    assert_eq!(page["cues"].as_array().unwrap().len(), 200);
}
