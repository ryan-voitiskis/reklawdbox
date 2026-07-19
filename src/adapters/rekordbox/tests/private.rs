use super::super::test_support::PrivateRekordboxFixture;
use super::super::*;
use crate::domain::library::Track;
use rusqlite::{Connection, params};

// ==================== Integration tests (real DB) — history ====================

fn open_private_fixture() -> (PrivateRekordboxFixture, Connection) {
    let fixture = PrivateRekordboxFixture::from_env()
        .expect("private Rekordbox fixture requires REKORDBOX_TEST_BACKUP");
    let conn = fixture
        .open()
        .expect("private Rekordbox fixture should open read-only");
    (fixture, conn)
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_get_sessions() {
    let (_fixture_guard, conn) = open_private_fixture();
    let sessions = get_sessions(&conn, Some(5), None).unwrap();
    assert!(
        !sessions.is_empty(),
        "expected at least one session in real DB"
    );
    for s in &sessions {
        assert!(!s.id.is_empty());
        assert!(s.track_count >= 0);
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_get_session_tracks() {
    let (_fixture_guard, conn) = open_private_fixture();
    let sessions = get_sessions(&conn, Some(1), None).unwrap();
    assert!(!sessions.is_empty(), "need at least one session");
    let tracks = get_session_tracks(&conn, &sessions[0].id).unwrap();
    assert!(!tracks.is_empty(), "selected session has no tracks");
    for (i, t) in tracks.iter().enumerate() {
        assert_eq!(
            t.position,
            Some((i + 1) as u32),
            "track position mismatch at index {i}"
        );
    }
}

fn load_all_tracks(conn: &Connection) -> Vec<Track> {
    let mut all = Vec::new();
    let page_size = 200;
    let mut offset = 0;
    loop {
        let sql = format!(
            "{TRACK_SELECT} WHERE c.rb_local_deleted = 0 ORDER BY c.ID LIMIT {page_size} OFFSET {offset}"
        );
        let mut stmt = conn.prepare(&sql).unwrap();
        let batch: Vec<Track> = stmt
            .query_map([], row_to_track)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let count = batch.len();
        all.extend(batch);
        if count < page_size {
            break;
        }
        offset += page_size;
    }
    all
}

// ==================== Integration tests (real DB) ====================
// Run with: cargo test -- --ignored

#[test]
#[ignore]
fn private_rekordbox_test_real_db_opens() {
    let (_fixture_guard, conn) = open_private_fixture();
    let count: i32 = conn
        .query_row("SELECT count(*) FROM djmdContent", [], |r| r.get(0))
        .unwrap();
    assert!(count > 0, "DB opened but djmdContent is empty");
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_schema_tables() {
    let (_fixture_guard, conn) = open_private_fixture();
    let required = [
        "djmdContent",
        "djmdArtist",
        "djmdAlbum",
        "djmdGenre",
        "djmdKey",
        "djmdLabel",
        "djmdColor",
        "djmdPlaylist",
        "djmdSongPlaylist",
    ];
    for table in required {
        let exists: bool = conn
            .query_row(
                "SELECT count(*) > 0 FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "missing table: {table}");
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_schema_columns() {
    let (_fixture_guard, conn) = open_private_fixture();
    let checks = [
        (
            "djmdContent",
            "ID, Title, BPM, Commnt, Rating, FolderPath, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID, ReleaseYear, Length, DJPlayCount, BitRate, SampleRate, FileType, created_at, rb_local_deleted",
        ),
        ("djmdArtist", "ID, Name"),
        ("djmdAlbum", "ID, Name"),
        ("djmdGenre", "ID, Name"),
        ("djmdKey", "ID, ScaleName"),
        ("djmdLabel", "ID, Name"),
        ("djmdColor", "ID, ColorCode, Commnt"),
        (
            "djmdPlaylist",
            "ID, Name, Attribute, ParentID, Seq, rb_local_deleted",
        ),
        ("djmdSongPlaylist", "ID, PlaylistID, ContentID, TrackNo"),
    ];
    for (table, cols) in checks {
        let sql = format!("SELECT {cols} FROM {table} LIMIT 1");
        conn.prepare(&sql)
            .unwrap_or_else(|e| panic!("column check failed for {table}: {e}"));
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_track_count() {
    let (_fixture_guard, conn) = open_private_fixture();
    let stats = get_library_stats(&conn).unwrap();
    assert!(
        stats.total_tracks > 2000,
        "expected >2000 tracks, got {}",
        stats.total_tracks
    );
    assert!(stats.avg_bpm > 0.0, "avg_bpm should be positive");
    assert!(
        stats.playlist_count > 0,
        "should have at least one playlist"
    );
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_search_returns_results() {
    let (_fixture_guard, conn) = open_private_fixture();

    let params = SearchParams {
        limit: Some(10),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert!(!tracks.is_empty(), "unfiltered search returned no results");

    let params = SearchParams {
        bpm_min: Some(120.0),
        bpm_max: Some(130.0),
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert!(!tracks.is_empty(), "BPM 120-130 range returned no results");
    for (index, t) in tracks.iter().enumerate() {
        assert!(
            t.bpm >= 120.0 && t.bpm <= 130.0,
            "track at sample index {index} has BPM outside the requested range"
        );
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_field_encoding() {
    let (_fixture_guard, conn) = open_private_fixture();
    let params = SearchParams {
        limit: Some(200),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();

    for (index, t) in tracks.iter().enumerate() {
        assert!(
            t.bpm == 0.0 || (t.bpm >= 30.0 && t.bpm <= 300.0),
            "track at sample index {index} has unreasonable BPM"
        );
        assert!(
            t.rating <= 5,
            "track at sample index {index} has an invalid rating"
        );
        assert!(
            t.length > 0,
            "track at sample index {index} has non-positive length"
        );
        assert!(
            !t.file_path.is_empty(),
            "track at sample index {index} has an empty file path"
        );
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_null_handling() {
    let (_fixture_guard, conn) = open_private_fixture();

    let params = SearchParams {
        has_genre: Some(false),
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for (index, t) in tracks.iter().enumerate() {
        assert!(
            t.genre.is_empty(),
            "track at sample index {index} unexpectedly has a genre"
        );
    }

    let params = SearchParams {
        has_genre: Some(true),
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for (index, t) in tracks.iter().enumerate() {
        assert!(
            !t.genre.is_empty(),
            "track at sample index {index} unexpectedly lacks a genre"
        );
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_unicode() {
    let (_fixture_guard, conn) = open_private_fixture();
    let all = load_all_tracks(&conn);

    let unicode_tracks: Vec<_> = all
        .iter()
        .filter(|t| !t.title.is_ascii() || !t.artist.is_ascii())
        .collect();

    for t in &unicode_tracks {
        let json = serde_json::to_string(t).unwrap();
        let back: crate::domain::library::Track = serde_json::from_str(&json).unwrap();
        assert_eq!(t.title, back.title, "unicode title round-trip failed");
        assert_eq!(t.artist, back.artist, "unicode artist round-trip failed");
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_playlists() {
    let (_fixture_guard, conn) = open_private_fixture();
    let playlists = get_playlists(&conn).unwrap();
    assert!(!playlists.is_empty(), "no playlists found");

    let has_folder = playlists.iter().any(|p| p.is_folder);
    let has_regular = playlists.iter().any(|p| !p.is_folder && !p.is_smart);
    assert!(
        has_folder || has_regular,
        "no folders or regular playlists found"
    );

    for (index, p) in playlists
        .iter()
        .filter(|p| !p.is_folder && p.track_count > 0)
        .take(3)
        .enumerate()
    {
        let tracks = get_playlist_tracks(&conn, &p.id, Some(10)).unwrap();
        assert!(
            !tracks.is_empty(),
            "sample playlist at index {index} claims tracks but returned none"
        );
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_get_track_by_id() {
    let (_fixture_guard, conn) = open_private_fixture();

    let params = SearchParams {
        limit: Some(1),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert!(!tracks.is_empty());

    let by_id = get_track(&conn, &tracks[0].id)
        .unwrap()
        .expect("track not found by ID");
    assert_eq!(tracks[0].id, by_id.id);
    assert_eq!(tracks[0].title, by_id.title);
    assert_eq!(tracks[0].artist, by_id.artist);
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_library_stats_consistency() {
    let (_fixture_guard, conn) = open_private_fixture();
    let stats = get_library_stats(&conn).unwrap();

    assert_eq!(
        stats.rated_count + stats.unrated_count,
        stats.total_tracks,
        "rated ({}) + unrated ({}) != total ({})",
        stats.rated_count,
        stats.unrated_count,
        stats.total_tracks
    );

    let genre_sum: i32 = stats.genres.iter().map(|g| g.count).sum();
    assert_eq!(
        genre_sum, stats.total_tracks,
        "genre count sum ({genre_sum}) != total ({})",
        stats.total_tracks
    );

    let key_sum: i32 = stats.key_distribution.iter().map(|k| k.count).sum();
    assert_eq!(
        key_sum, stats.total_tracks,
        "key count sum ({key_sum}) != total ({})",
        stats.total_tracks
    );
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_rated_count_matches_rating_filtered_search() {
    let (_fixture_guard, conn) = open_private_fixture();
    let stats = get_library_stats(&conn).unwrap();

    let params = SearchParams {
        rating_min: Some(1),
        exclude_samples: true,
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).expect("rating-filtered search should succeed");

    if stats.rated_count > 0 {
        assert!(
            !tracks.is_empty(),
            "rated_count={} but rating_min=1 search returned no rows",
            stats.rated_count
        );
    }
    assert!(
        tracks.iter().all(|track| track.rating >= 1),
        "rating_min=1 search should only return tracks with star rating >= 1"
    );
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_all_tracks_load() {
    let (_fixture_guard, conn) = open_private_fixture();
    let all = load_all_tracks(&conn);
    assert!(all.len() > 2000, "expected >2000 tracks, got {}", all.len());

    for t in &all {
        assert!(!t.id.is_empty(), "track has empty ID");
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_special_characters_in_search() {
    let (_fixture_guard, conn) = open_private_fixture();
    let nasty_inputs = [
        "'; DROP TABLE djmdContent; --",
        "\" OR 1=1 --",
        "track & bass",
        "100%",
        "it's a test",
        "null\0byte",
        "emoji 🎵",
        "",
    ];

    for input in nasty_inputs {
        let params = SearchParams {
            query: Some(input.to_string()),
            limit: Some(5),
            ..Default::default()
        };
        let result = search_tracks(&conn, &params);
        assert!(result.is_ok(), "search panicked on input: {input:?}");
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_sample_exclusion() {
    let (_fixture_guard, conn) = open_private_fixture();

    let stats_filtered = get_library_stats(&conn).unwrap();
    let stats_all = get_library_stats_filtered(&conn, false).unwrap();

    assert!(
        stats_filtered.total_tracks <= stats_all.total_tracks,
        "filtered {} > unfiltered {}",
        stats_filtered.total_tracks,
        stats_all.total_tracks
    );

    let params = SearchParams {
        exclude_samples: true,
        limit: Some(200),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for (index, t) in tracks.iter().enumerate() {
        assert!(
            !is_sampler_path(&t.file_path),
            "sampler track at sample index {index} was not excluded"
        );
    }
}

#[test]
#[ignore]
fn private_rekordbox_test_real_db_genre_normalization_coverage() {
    let (_fixture_guard, conn) = open_private_fixture();
    let stats = get_library_stats(&conn).unwrap();

    let mut alias_count = 0;
    let mut canonical_count = 0;
    let mut unknown_count = 0;

    for gc in &stats.genres {
        if gc.name == "(none)" || gc.name.is_empty() {
            continue;
        }
        if crate::domain::classification::taxonomy::canonical_genre_from_alias(&gc.name).is_some() {
            alias_count += gc.count;
        } else if crate::domain::classification::taxonomy::is_known_genre(&gc.name) {
            canonical_count += gc.count;
        } else {
            unknown_count += gc.count;
        }
    }

    let classified_count = stats
        .genres
        .iter()
        .filter(|genre| genre.name != "(none)" && !genre.name.is_empty())
        .map(|genre| genre.count)
        .sum::<i32>();
    assert_eq!(
        canonical_count + alias_count + unknown_count,
        classified_count
    );
}

// --- decode_rating_stars ---
