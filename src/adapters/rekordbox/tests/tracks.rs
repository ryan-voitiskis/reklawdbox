use super::super::*;
use super::support::create_test_db;
use crate::domain::library::FileKind;

#[test]
fn rekordbox_tracks_validate_iso_date_valid() {
    assert!(validate_iso_date("2024-01-15", "date").is_ok());
    assert!(validate_iso_date("2000-02-29", "date").is_ok()); // leap year
    assert!(validate_iso_date("1999-12-31", "date").is_ok());
}

#[test]
fn rekordbox_tracks_validate_iso_date_invalid_dates() {
    assert!(validate_iso_date("2024-02-30", "date").is_err());
    assert!(validate_iso_date("2023-02-29", "date").is_err()); // non-leap
    assert!(validate_iso_date("2024-13-01", "date").is_err());
    assert!(validate_iso_date("2024-00-15", "date").is_err());
    assert!(validate_iso_date("2024-01-00", "date").is_err());
    assert!(validate_iso_date("2024-01-32", "date").is_err());
}

#[test]
fn rekordbox_tracks_validate_iso_date_bad_format() {
    assert!(validate_iso_date("20240115", "date").is_err());
    assert!(validate_iso_date("not-a-date", "date").is_err());
    assert!(validate_iso_date("", "date").is_err());
    assert!(validate_iso_date("2024/01/15", "date").is_err());
}

#[test]
fn rekordbox_tracks_validate_iso_date_century_leap() {
    assert!(validate_iso_date("2000-02-29", "date").is_ok()); // divisible by 400
    assert!(validate_iso_date("1900-02-29", "date").is_err()); // divisible by 100, not 400
}

#[test]
fn rekordbox_tracks_escape_like_special_chars() {
    assert_eq!(escape_like("100%"), "100\\%");
    assert_eq!(escape_like("under_score"), "under\\_score");
    assert_eq!(escape_like("back\\slash"), "back\\\\slash");
    assert_eq!(escape_like("normal text"), "normal text");
    // Combined: backslash must be escaped first so \% doesn't double-escape
    assert_eq!(escape_like("\\%"), "\\\\\\%");
    assert_eq!(escape_like("a\\b_c%d"), "a\\\\b\\_c\\%d");
}

#[test]
fn rekordbox_tracks_test_search_all() {
    let conn = create_test_db();
    let params = SearchParams::default();
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 7);
}

#[test]
fn rekordbox_tracks_test_search_exclude_samples() {
    let conn = create_test_db();
    let params = SearchParams {
        exclude_samples: true,
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 6);
    assert!(!tracks.iter().any(|t| t.file_path.contains("Sampler")));
}

#[test]
fn rekordbox_tracks_test_search_by_genre() {
    let conn = create_test_db();
    let params = SearchParams {
        genre: Some("Dubstep".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(tracks.iter().all(|t| t.genre == "Dubstep"));
}

#[test]
fn rekordbox_tracks_test_search_by_bpm_range() {
    let conn = create_test_db();
    let params = SearchParams {
        bpm_min: Some(130.0),
        bpm_max: Some(145.0),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(tracks.iter().all(|t| t.bpm >= 130.0 && t.bpm <= 145.0));
}

#[test]
fn rekordbox_tracks_test_search_has_no_genre() {
    let conn = create_test_db();
    let params = SearchParams {
        has_genre: Some(false),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Unknown Track");
}

#[test]
fn rekordbox_tracks_test_search_by_rating() {
    let conn = create_test_db();
    let params = SearchParams {
        rating_min: Some(3),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
}

#[test]
fn rekordbox_tracks_test_search_by_rating_supports_star_scale_storage() {
    let conn = create_test_db();
    conn.execute("UPDATE djmdContent SET Rating = 5 WHERE ID = 't4'", [])
        .expect("fixture rating update should succeed");

    let params = SearchParams {
        rating_min: Some(5),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).expect("rating filter should succeed");
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t4");
    assert_eq!(tracks[0].rating, 5);
}

#[test]
fn rekordbox_tracks_test_search_by_key() {
    let conn = create_test_db();
    let params = SearchParams {
        key: Some("Am".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "Archangel");
}

#[test]
fn rekordbox_tracks_test_search_by_playlist() {
    let conn = create_test_db();
    let params = SearchParams {
        playlist: Some("p1".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
}

#[test]
fn rekordbox_tracks_test_search_by_path_substring() {
    let conn = create_test_db();
    let params = SearchParams {
        path: Some("Burial".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(tracks.iter().all(|t| t.file_path.contains("Burial")));
}

#[test]
fn rekordbox_tracks_test_search_by_path_prefix() {
    let conn = create_test_db();
    let params = SearchParams {
        path_prefix: Some("/Users/testuser/Music/Burial/".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
    assert!(
        tracks
            .iter()
            .all(|t| t.file_path.starts_with("/Users/testuser/Music/Burial/"))
    );
}

#[test]
fn rekordbox_tracks_test_path_prefix_excludes_substring_matches() {
    let conn = create_test_db();
    let params = SearchParams {
        path_prefix: Some("Music".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 0);

    let params = SearchParams {
        path: Some("Music".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 7);
}

#[test]
fn rekordbox_tracks_test_path_prefix_scopes_to_user_root() {
    let conn = create_test_db();
    let params = SearchParams {
        path_prefix: Some("/Users/testuser/".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 6);
    assert!(
        tracks
            .iter()
            .all(|t| t.file_path.starts_with("/Users/testuser/"))
    );
}

#[test]
fn rekordbox_tracks_test_path_prefix_escapes_like_chars() {
    let conn = create_test_db();
    let params = SearchParams {
        path_prefix: Some("/Users/%/Music".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 0);
}

#[test]
fn rekordbox_tracks_test_get_track() {
    let conn = create_test_db();
    let track = get_track(&conn, "t1").unwrap().unwrap();
    assert_eq!(track.title, "Archangel");
    assert_eq!(track.artist, "Burial");
    assert_eq!(track.genre, "Dubstep");
    assert_eq!(track.bpm, 139.5);
    assert_eq!(track.rating, 4);
    assert_eq!(track.comments, "iconic garage vocal");
    assert_eq!(track.label, "Hyperdub");
    assert_eq!(track.year, 2007);
    assert_eq!(track.file_kind, FileKind::Flac);
    assert_eq!(track.position, None);
}

#[test]
fn rekordbox_tracks_test_get_track_not_found() {
    let conn = create_test_db();
    let track = get_track(&conn, "nonexistent").unwrap();
    assert!(track.is_none());
}

#[test]
fn rekordbox_tracks_pending_batch_page_search_equal_titles_use_stable_id_tiebreaker() {
    let conn = create_test_db();
    conn.execute(
        "UPDATE djmdContent SET Title = 'Equal Title' WHERE ID IN ('t1', 't2')",
        [],
    )
    .unwrap();
    let mut search = SearchParams {
        query: Some("Equal Title".to_string()),
        limit: Some(1),
        offset: Some(0),
        ..Default::default()
    };
    let first = search_tracks_unbounded(&conn, &search).unwrap();
    search.offset = Some(1);
    let second = search_tracks_unbounded(&conn, &search).unwrap();
    search.limit = Some(2);
    search.offset = Some(0);
    let repeated = search_tracks_unbounded(&conn, &search).unwrap();
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
fn rekordbox_tracks_test_library_stats() {
    let conn = create_test_db();
    let stats = get_library_stats(&conn).unwrap();
    assert_eq!(stats.total_tracks, 6);
    assert_eq!(stats.rated_count, 3);
    assert_eq!(stats.unrated_count, 3);
    assert_eq!(stats.playlist_count, 1);
    assert!(stats.avg_bpm > 0.0);
    assert!(!stats.genres.is_empty());
    assert!(!stats.key_distribution.is_empty());
    assert_eq!(stats.content_roots, vec!["/Users/testuser/Music/"]);

    let stats_all = get_library_stats_filtered(&conn, false).unwrap();
    assert_eq!(stats_all.total_tracks, 7);
    assert_eq!(
        stats_all.content_roots,
        vec!["/Users/alice/", "/Users/testuser/"]
    );
}

#[test]
fn rekordbox_tracks_test_content_roots_empty_library() {
    let conn = open_test();
    conn.execute_batch(
        "CREATE TABLE djmdContent (
            ID VARCHAR(255) PRIMARY KEY,
            FolderPath VARCHAR(255) DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );",
    )
    .unwrap();
    let roots = content_roots(&conn, false).unwrap();
    assert!(roots.is_empty());
}

#[test]
fn rekordbox_tracks_test_get_tracks_by_exact_genre() {
    let conn = create_test_db();
    let tracks = get_tracks_by_exact_genre(&conn, "Dubstep", false).unwrap();
    assert_eq!(tracks.len(), 2); // Archangel + Endorphin
    assert!(tracks.iter().all(|t| t.genre == "Dubstep"));

    let tracks = get_tracks_by_exact_genre(&conn, "Techno", true).unwrap();
    assert_eq!(tracks.len(), 1); // R.I.P. only (sampler excluded)
    assert_eq!(tracks[0].title, "R.I.P.");

    let tracks = get_tracks_by_exact_genre(&conn, "Techno", false).unwrap();
    assert_eq!(tracks.len(), 2); // R.I.P. + Loop Sample 01
}

#[test]
fn rekordbox_tracks_test_get_tracks_by_ids() {
    let conn = create_test_db();
    let tracks = get_tracks_by_ids(&conn, &["t3".to_string(), "t1".to_string()]).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[1].id, "t1");
}

#[test]
fn rekordbox_tracks_test_get_tracks_by_ids_batches_large_input() {
    let conn = create_test_db();
    let ids: Vec<String> = (0..1200).map(|_| "t1".to_string()).collect();
    let tracks = get_tracks_by_ids(&conn, &ids).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t1");
}

#[test]
fn rekordbox_tracks_decode_rating_stars_passthrough_0_to_5() {
    for star in 0..=5 {
        assert_eq!(decode_rating_stars(star), star as u8, "star value {star}");
    }
}

#[test]
fn rekordbox_tracks_decode_rating_stars_negative_maps_to_zero() {
    assert_eq!(decode_rating_stars(-1), 0);
    assert_eq!(decode_rating_stars(-100), 0);
    assert_eq!(decode_rating_stars(i32::MIN), 0);
}

#[test]
fn rekordbox_tracks_decode_rating_stars_encoded_boundaries() {
    // Values 6+ are decoded via rating_to_stars (0-255 encoded scale)
    // 0..=25 -> 0 stars, but 0-5 are caught by passthrough, so 6..=25 -> 0
    assert_eq!(decode_rating_stars(6), 0);
    assert_eq!(decode_rating_stars(25), 0);
    // 26..=76 -> 1 star
    assert_eq!(decode_rating_stars(26), 1);
    assert_eq!(decode_rating_stars(51), 1);
    assert_eq!(decode_rating_stars(76), 1);
    // 77..=127 -> 2 stars
    assert_eq!(decode_rating_stars(77), 2);
    assert_eq!(decode_rating_stars(127), 2);
    // 128..=178 -> 3 stars
    assert_eq!(decode_rating_stars(128), 3);
    assert_eq!(decode_rating_stars(178), 3);
    // 179..=229 -> 4 stars
    assert_eq!(decode_rating_stars(179), 4);
    assert_eq!(decode_rating_stars(229), 4);
    // 230..=255 -> 5 stars
    assert_eq!(decode_rating_stars(230), 5);
    assert_eq!(decode_rating_stars(255), 5);
}

#[test]
fn rekordbox_tracks_decode_rating_stars_above_255_clamps_to_5() {
    // rating_to_stars treats >255 as 5 stars (catch-all arm)
    assert_eq!(decode_rating_stars(256), 5);
    assert_eq!(decode_rating_stars(1000), 5);
}

#[test]
fn rekordbox_tracks_next_day_normal() {
    assert_eq!(next_day("2023-06-15"), "2023-06-16");
}

#[test]
fn rekordbox_tracks_next_day_month_boundary_31() {
    assert_eq!(next_day("2023-01-31"), "2023-02-01");
}

#[test]
fn rekordbox_tracks_next_day_month_boundary_30() {
    assert_eq!(next_day("2023-04-30"), "2023-05-01");
}

#[test]
fn rekordbox_tracks_next_day_feb_non_leap() {
    assert_eq!(next_day("2023-02-28"), "2023-03-01");
}

#[test]
fn rekordbox_tracks_next_day_feb_leap_28() {
    assert_eq!(next_day("2024-02-28"), "2024-02-29");
}

#[test]
fn rekordbox_tracks_next_day_feb_leap_29() {
    assert_eq!(next_day("2024-02-29"), "2024-03-01");
}

#[test]
fn rekordbox_tracks_next_day_year_rollover() {
    assert_eq!(next_day("2023-12-31"), "2024-01-01");
}

#[test]
fn rekordbox_tracks_next_day_century_non_leap() {
    assert_eq!(next_day("1900-02-28"), "1900-03-01");
}

#[test]
fn rekordbox_tracks_next_day_century_leap() {
    assert_eq!(next_day("2000-02-29"), "2000-03-01");
}

#[test]
fn rekordbox_tracks_added_before_includes_boundary_date_with_datetime() {
    // Real Rekordbox created_at has time component: "2023-02-20 14:00:00.000 +00:00"
    // added_before="2023-02-20" must include tracks on that date.
    let conn = create_test_db();
    let params = SearchParams {
        added_before: Some("2023-02-20".to_string()),
        exclude_samples: true,
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    // t1, t2 (Jan 15), t3 (Feb 20 14:00), t7 (Feb 20 15:00) should be included
    assert!(ids.contains(&"t1"), "t1 added Jan 15 should be included");
    assert!(
        ids.contains(&"t3"),
        "t3 added Feb 20 should be included (boundary)"
    );
    assert!(
        ids.contains(&"t7"),
        "t7 added Feb 20 should be included (boundary)"
    );
    assert!(!ids.contains(&"t4"), "t4 added Mar 10 should be excluded");
}

#[test]
fn rekordbox_tracks_added_after_includes_boundary_date_with_datetime() {
    let conn = create_test_db();
    let params = SearchParams {
        added_after: Some("2023-02-20".to_string()),
        exclude_samples: true,
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    assert!(ids.contains(&"t3"), "t3 added Feb 20 should be included");
    assert!(ids.contains(&"t7"), "t7 added Feb 20 should be included");
    assert!(ids.contains(&"t4"), "t4 added Mar 10 should be included");
    assert!(!ids.contains(&"t1"), "t1 added Jan 15 should be excluded");
}

#[test]
fn rekordbox_tracks_search_has_unknown_genre() {
    // t7 has genre "Wonky Bass" which is not in the canonical taxonomy.
    // All other tracks have canonical genres (Dubstep, Techno, Minimal) or no genre.
    let conn = create_test_db();
    let params = SearchParams {
        has_genre: Some(true),
        ..Default::default()
    };
    let all_with_genre = search_tracks(&conn, &params).unwrap();

    // Post-filter for unknown genres (same logic as handle_search_tracks)
    let unknown: Vec<_> = all_with_genre
        .into_iter()
        .filter(|t| {
            !t.genre.is_empty()
                && !crate::domain::classification::taxonomy::is_known_genre(&t.genre)
                && crate::domain::classification::taxonomy::canonical_genre_from_alias(&t.genre)
                    .is_none()
        })
        .collect();
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].id, "t7");
    assert_eq!(unknown[0].genre, "Wonky Bass");
}
