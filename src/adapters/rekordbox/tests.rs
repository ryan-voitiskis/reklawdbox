use super::*;
use crate::types::{FileKind, Track};
use rusqlite::{Connection, OpenFlags, params};

#[test]
fn configured_missing_db_fails_closed_without_using_default() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.db");
    let fallback_called = std::cell::Cell::new(false);

    let resolved = resolve_db_path_from(Some(missing.into_os_string()), || {
        fallback_called.set(true);
        Some("/real/library/master.db".to_string())
    });

    assert!(resolved.is_none());
    assert!(!fallback_called.get());
}

#[test]
fn configured_existing_db_wins_over_default() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let configured = file.path().to_path_buf();

    let resolved = resolve_db_path_from(Some(configured.clone().into_os_string()), || {
        Some("/real/library/master.db".to_string())
    });

    assert_eq!(resolved.as_deref(), configured.to_str());
}

pub fn create_test_db() -> Connection {
    let conn = open_test();
    seed_test_db(&conn);
    conn
}

fn seed_test_db(conn: &Connection) {
    conn.execute_batch(
        "
        CREATE TABLE djmdArtist (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdAlbum (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdGenre (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdKey (
            ID VARCHAR(255) PRIMARY KEY,
            ScaleName VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdLabel (
            ID VARCHAR(255) PRIMARY KEY,
            Name VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdColor (
            ID VARCHAR(255) PRIMARY KEY,
            ColorCode INTEGER,
            Commnt VARCHAR(255),
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdContent (
            ID VARCHAR(255) PRIMARY KEY,
            Title VARCHAR(255),
            ArtistID VARCHAR(255),
            AlbumID VARCHAR(255),
            GenreID VARCHAR(255),
            KeyID VARCHAR(255),
            ColorID VARCHAR(255),
            LabelID VARCHAR(255),
            RemixerID VARCHAR(255),
            BPM INTEGER DEFAULT 0,
            Rating INTEGER DEFAULT 0,
            Commnt TEXT DEFAULT '',
            ReleaseYear INTEGER DEFAULT 0,
            Length INTEGER DEFAULT 0,
            FolderPath VARCHAR(255) DEFAULT '',
            DJPlayCount VARCHAR(255) DEFAULT '0',
            BitRate INTEGER DEFAULT 0,
            SampleRate INTEGER DEFAULT 0,
            FileType INTEGER DEFAULT 0,
            created_at TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdPlaylist (
            ID VARCHAR(255) PRIMARY KEY,
            Seq INTEGER,
            Name VARCHAR(255),
            Attribute INTEGER DEFAULT 0,
            ParentID VARCHAR(255) DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongPlaylist (
            ID VARCHAR(255) PRIMARY KEY,
            PlaylistID VARCHAR(255),
            ContentID VARCHAR(255),
            TrackNo INTEGER
        );

        -- Lookup data
        INSERT INTO djmdArtist (ID, Name) VALUES ('a1', 'Burial');
        INSERT INTO djmdArtist (ID, Name) VALUES ('a2', 'Actress');
        INSERT INTO djmdArtist (ID, Name) VALUES ('a3', 'Ricardo Villalobos');
        INSERT INTO djmdAlbum (ID, Name) VALUES ('al1', 'Untrue');
        INSERT INTO djmdAlbum (ID, Name) VALUES ('al2', 'R.I.P.');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g1', 'Dubstep');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'Techno');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g3', 'Minimal');
        INSERT INTO djmdGenre (ID, Name) VALUES ('g4', 'Wonky Bass');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k1', 'Am');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k2', 'Cm');
        INSERT INTO djmdKey (ID, ScaleName) VALUES ('k3', 'Fm');
        INSERT INTO djmdLabel (ID, Name) VALUES ('l1', 'Hyperdub');
        INSERT INTO djmdLabel (ID, Name) VALUES ('l2', 'Ninja Tune');
        INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c1', 16711935, 'Rose');
        INSERT INTO djmdColor (ID, ColorCode, Commnt) VALUES ('c2', 65280, 'Green');

        -- Tracks (created_at uses full datetime to match real Rekordbox format)
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, ColorID, BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
        VALUES ('t1', 'Archangel', 'a1', 'al1', 'g1', 'k1', 'l1', 'c1', 13950, 204, 'iconic garage vocal', 2007, 240, '/Users/testuser/Music/Burial/Untrue/01 Archangel.flac', '12', 1411, 44100, 5, '2023-01-15 10:30:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, LabelID, BPM, Rating, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate, SampleRate, FileType, created_at)
        VALUES ('t2', 'Endorphin', 'a1', 'al1', 'g1', 'k2', 'l1', 14000, 153, 2007, 300, '/Users/testuser/Music/Burial/Untrue/02 Endorphin.flac', '5', 1411, 44100, 5, '2023-01-15 10:31:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, AlbumID, GenreID, KeyID, BPM, Rating, ReleaseYear, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t3', 'R.I.P.', 'a2', 'al2', 'g2', 'k3', 12800, 102, 2012, 360, '/Users/testuser/Music/Actress/R.I.P./01 R.I.P..flac', 1411, 44100, 5, '2023-02-20 14:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t4', 'Dexter', 'a3', 'g3', 12500, 480, '/Users/testuser/Music/Villalobos/Dexter.wav', 1411, 44100, 11, '2023-03-10 09:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t5', 'Unknown Track', 'a1', 0, 200, '/Users/testuser/Music/unknown.mp3', 320, 44100, 1, '2023-04-01 12:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t6', 'Loop Sample 01', 'a1', 'g2', 12000, 8, '/Users/alice/Music/rekordbox/Sampler/Loop/01.wav', 1411, 44100, 11, '2023-01-01 08:00:00.000 +00:00');
        INSERT INTO djmdContent (ID, Title, ArtistID, GenreID, BPM, Length, FolderPath, BitRate, SampleRate, FileType, created_at)
        VALUES ('t7', 'Wonky Bassline', 'a2', 'g4', 12600, 300, '/Users/testuser/Music/Actress/Wonky.flac', 1411, 44100, 5, '2023-02-20 15:00:00.000 +00:00');

        -- Playlists
        INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p1', 1, 'Deep Cuts', 0, 'root');
        INSERT INTO djmdPlaylist (ID, Seq, Name, Attribute, ParentID) VALUES ('p2', 2, 'Folders', 1, 'root');
        INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp1', 'p1', 't1', 1);
        INSERT INTO djmdSongPlaylist (ID, PlaylistID, ContentID, TrackNo) VALUES ('sp2', 'p1', 't3', 2);

        -- History tables
        CREATE TABLE djmdHistory (
            ID VARCHAR(255) PRIMARY KEY,
            Seq INTEGER,
            Name VARCHAR(255),
            Attribute INTEGER DEFAULT 0,
            ParentID VARCHAR(255) DEFAULT '',
            DateCreated TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE djmdSongHistory (
            ID VARCHAR(255) PRIMARY KEY,
            HistoryID VARCHAR(255),
            ContentID VARCHAR(255),
            TrackNo INTEGER,
            created_at TEXT DEFAULT '',
            rb_local_deleted INTEGER DEFAULT 0
        );

        -- Sessions: h1 (2025-03-01), h2 (2025-02-15), hf1 is a folder (Attribute=1)
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('h1', 1, '2025-03-01', 0, '2025-03-01');
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('h2', 2, '2025-02-15', 0, '2025-02-15');
        INSERT INTO djmdHistory (ID, Seq, Name, Attribute, DateCreated) VALUES ('hf1', 3, 'History Folder', 1, '2025-01-01');

        -- Song history: h1 has 3 tracks (t1, t3, t2) with 5min gaps; h2 has 1 track (t1)
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh1', 'h1', 't1', 1, '2025-03-01 22:00:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh2', 'h1', 't3', 2, '2025-03-01 22:05:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh3', 'h1', 't2', 3, '2025-03-01 22:10:00');
        INSERT INTO djmdSongHistory (ID, HistoryID, ContentID, TrackNo, created_at) VALUES ('sh4', 'h2', 't1', 1, '2025-02-15 20:00:00');
        ",
    )
    .unwrap();
}

#[test]
fn sanitized_sqlcipher_fixture_opens_read_only_through_production_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sanitized-master.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(&format!("PRAGMA key = '{REKORDBOX_SQLCIPHER_KEY}'"))
            .unwrap();
        seed_test_db(&conn);
    }

    let plain = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(
        plain
            .query_row("SELECT count(*) FROM sqlite_master", [], |row| row
                .get::<_, i64>(0))
            .is_err(),
        "fixture must be encrypted rather than plain SQLite"
    );

    let conn = open(path.to_str().unwrap()).expect("production SQLCipher open should succeed");
    let tracks = search_tracks(&conn, &SearchParams::default()).unwrap();
    assert_eq!(tracks.len(), 7);
    assert!(
        tracks
            .iter()
            .any(|track| track.id == "t1" && track.title == "Archangel")
    );
    assert!(
        conn.execute(
            "INSERT INTO djmdArtist (ID, Name) VALUES ('write', 'blocked')",
            [],
        )
        .is_err(),
        "production fixture connection must remain read-only"
    );
}

#[test]
fn validate_iso_date_valid() {
    assert!(validate_iso_date("2024-01-15", "date").is_ok());
    assert!(validate_iso_date("2000-02-29", "date").is_ok()); // leap year
    assert!(validate_iso_date("1999-12-31", "date").is_ok());
}

#[test]
fn validate_iso_date_invalid_dates() {
    assert!(validate_iso_date("2024-02-30", "date").is_err());
    assert!(validate_iso_date("2023-02-29", "date").is_err()); // non-leap
    assert!(validate_iso_date("2024-13-01", "date").is_err());
    assert!(validate_iso_date("2024-00-15", "date").is_err());
    assert!(validate_iso_date("2024-01-00", "date").is_err());
    assert!(validate_iso_date("2024-01-32", "date").is_err());
}

#[test]
fn validate_iso_date_bad_format() {
    assert!(validate_iso_date("20240115", "date").is_err());
    assert!(validate_iso_date("not-a-date", "date").is_err());
    assert!(validate_iso_date("", "date").is_err());
    assert!(validate_iso_date("2024/01/15", "date").is_err());
}

#[test]
fn validate_iso_date_century_leap() {
    assert!(validate_iso_date("2000-02-29", "date").is_ok()); // divisible by 400
    assert!(validate_iso_date("1900-02-29", "date").is_err()); // divisible by 100, not 400
}

#[test]
fn escape_like_special_chars() {
    assert_eq!(escape_like("100%"), "100\\%");
    assert_eq!(escape_like("under_score"), "under\\_score");
    assert_eq!(escape_like("back\\slash"), "back\\\\slash");
    assert_eq!(escape_like("normal text"), "normal text");
    // Combined: backslash must be escaped first so \% doesn't double-escape
    assert_eq!(escape_like("\\%"), "\\\\\\%");
    assert_eq!(escape_like("a\\b_c%d"), "a\\\\b\\_c\\%d");
}

#[test]
fn test_search_all() {
    let conn = create_test_db();
    let params = SearchParams::default();
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 7);
}

#[test]
fn test_search_exclude_samples() {
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
fn test_search_by_genre() {
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
fn test_search_by_bpm_range() {
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
fn test_search_has_no_genre() {
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
fn test_search_by_rating() {
    let conn = create_test_db();
    let params = SearchParams {
        rating_min: Some(3),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
}

#[test]
fn test_search_by_rating_supports_star_scale_storage() {
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
fn test_search_by_key() {
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
fn test_search_by_playlist() {
    let conn = create_test_db();
    let params = SearchParams {
        playlist: Some("p1".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 2);
}

#[test]
fn test_search_by_path_substring() {
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
fn test_search_by_path_prefix() {
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
fn test_path_prefix_excludes_substring_matches() {
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
fn test_path_prefix_scopes_to_user_root() {
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
fn test_path_prefix_escapes_like_chars() {
    let conn = create_test_db();
    let params = SearchParams {
        path_prefix: Some("/Users/%/Music".to_string()),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    assert_eq!(tracks.len(), 0);
}

#[test]
fn test_get_track() {
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
fn test_get_track_not_found() {
    let conn = create_test_db();
    let track = get_track(&conn, "nonexistent").unwrap();
    assert!(track.is_none());
}

#[test]
fn test_get_playlists() {
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
fn test_get_playlists_track_count_excludes_deleted_tracks() {
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
fn test_get_playlist_tracks() {
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
fn test_get_playlist_tracks_bounded_limit_and_offset() {
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
fn test_get_playlist_tracks_unbounded_limit_and_offset() {
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
fn test_get_playlist_tracks_unbounded_offset_without_limit() {
    let conn = create_test_db();
    let tracks = get_playlist_tracks_unbounded_page(&conn, "p1", None, Some(1)).unwrap();

    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[0].position, Some(2));
}

#[test]
fn test_get_playlist_tracks_zero_and_beyond_end_offsets() {
    let conn = create_test_db();
    let from_start = get_playlist_tracks_unbounded_page(&conn, "p1", Some(1), Some(0)).unwrap();
    let beyond = get_playlist_tracks_unbounded_page(&conn, "p1", Some(10), Some(10)).unwrap();

    assert_eq!(from_start.len(), 1);
    assert_eq!(from_start[0].id, "t1");
    assert_eq!(from_start[0].position, Some(1));
    assert!(beyond.is_empty());
}

#[test]
fn pending_batch_page_playlist_equal_positions_use_stable_id_tiebreaker() {
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
fn pending_batch_page_search_equal_titles_use_stable_id_tiebreaker() {
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
fn test_library_stats() {
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
fn test_content_roots_empty_library() {
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
fn test_get_tracks_by_exact_genre() {
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
fn test_get_tracks_by_ids() {
    let conn = create_test_db();
    let tracks = get_tracks_by_ids(&conn, &["t3".to_string(), "t1".to_string()]).unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].id, "t3");
    assert_eq!(tracks[1].id, "t1");
}

#[test]
fn test_get_tracks_by_ids_batches_large_input() {
    let conn = create_test_db();
    let ids: Vec<String> = (0..1200).map(|_| "t1".to_string()).collect();
    let tracks = get_tracks_by_ids(&conn, &ids).unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, "t1");
}

// ==================== History / play stats tests ====================

#[test]
fn test_get_sessions() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, None).unwrap();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].id, "h1");
    assert_eq!(sessions[1].id, "h2");
    assert_eq!(sessions[0].track_count, 3);
    assert_eq!(sessions[1].track_count, 1);
}

#[test]
fn test_get_sessions_after_filter() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, Some("2025-03-01")).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "h1");
}

#[test]
fn test_get_sessions_limit() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, Some(1), None).unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "h1");
}

#[test]
fn test_get_sessions_duration() {
    let conn = create_test_db();
    let sessions = get_sessions(&conn, None, None).unwrap();
    let h1 = &sessions[0];
    assert_eq!(h1.duration_seconds, Some(900));
    let h2 = &sessions[1];
    assert_eq!(h2.duration_seconds, None);
}

#[test]
fn test_get_session_tracks() {
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
fn test_get_session_tracks_not_found() {
    let conn = create_test_db();
    let tracks = get_session_tracks(&conn, "nonexistent").unwrap();
    assert!(tracks.is_empty());
}

#[test]
fn test_get_play_stats_counts() {
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
fn test_get_play_stats_with_genre_filter() {
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
fn test_get_play_stats_include_unplayed() {
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
fn test_get_play_stats_session_ids() {
    let conn = create_test_db();
    let params = SearchParams::default();
    let stats = get_play_stats(&conn, &params, false, None).unwrap();
    let t1 = stats.iter().find(|s| s.track_id == "t1").unwrap();
    assert_eq!(t1.session_ids.len(), 2);
    assert!(t1.session_ids.contains(&"h1".to_string()));
    assert!(t1.session_ids.contains(&"h2".to_string()));
}

// ==================== Library health query tests ====================

#[test]
fn test_tracks_not_in_any_playlist() {
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
fn test_tracks_not_in_any_playlist_with_samples() {
    let conn = create_test_db();
    let search = SearchParams::default();
    let result = tracks_not_in_any_playlist(&conn, &search).unwrap();
    assert_eq!(result.total_tracks, 7);
    assert_eq!(result.uncovered_count, 5);
}

#[test]
fn test_tracks_not_in_any_playlist_excludes_deleted() {
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
fn test_all_track_paths() {
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
fn test_find_metadata_duplicates_none() {
    let conn = create_test_db();
    let groups = find_metadata_duplicates(&conn, None, None).unwrap();
    assert!(groups.is_empty());
}

#[test]
fn test_find_metadata_duplicates_with_dup() {
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
fn test_find_metadata_duplicates_case_insensitive() {
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
fn scan_duplicates_page_metadata_reports_total_and_stable_adjacent_pages() {
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
fn test_playlist_membership_counts() {
    let conn = create_test_db();
    let ids = vec!["t1".to_string(), "t2".to_string(), "t3".to_string()];
    let counts = playlist_membership_counts(&conn, &ids).unwrap();
    assert_eq!(counts.get("t1"), Some(&1));
    assert_eq!(counts.get("t3"), Some(&1));
    assert_eq!(counts.get("t2"), None);
}

// ==================== Integration tests (real DB) — history ====================

#[test]
#[ignore]
fn test_real_db_get_sessions() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_get_session_tracks() {
    let conn = open_real_db().expect("backup tarball not found");
    let sessions = get_sessions(&conn, Some(1), None).unwrap();
    assert!(!sessions.is_empty(), "need at least one session");
    let tracks = get_session_tracks(&conn, &sessions[0].id).unwrap();
    assert!(
        !tracks.is_empty(),
        "session {} has no tracks",
        sessions[0].id
    );
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
fn test_real_db_opens() {
    let conn = open_real_db().expect("backup tarball not found — set REKORDBOX_TEST_BACKUP");
    let count: i32 = conn
        .query_row("SELECT count(*) FROM djmdContent", [], |r| r.get(0))
        .unwrap();
    assert!(count > 0, "DB opened but djmdContent is empty");
}

#[test]
#[ignore]
fn test_real_db_schema_tables() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_schema_columns() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_track_count() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_search_returns_results() {
    let conn = open_real_db().expect("backup tarball not found");

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
    for t in &tracks {
        assert!(
            t.bpm >= 120.0 && t.bpm <= 130.0,
            "track {} BPM {} outside range",
            t.title,
            t.bpm
        );
    }
}

#[test]
#[ignore]
fn test_real_db_field_encoding() {
    let conn = open_real_db().expect("backup tarball not found");
    let params = SearchParams {
        limit: Some(200),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();

    for t in &tracks {
        assert!(
            t.bpm == 0.0 || (t.bpm >= 30.0 && t.bpm <= 300.0),
            "track '{}' has unreasonable BPM: {}",
            t.title,
            t.bpm
        );
        assert!(
            t.rating <= 5,
            "track '{}' has invalid rating: {}",
            t.title,
            t.rating
        );
        assert!(
            t.length > 0,
            "track '{}' has non-positive length: {}",
            t.title,
            t.length
        );
        assert!(
            !t.file_path.is_empty(),
            "track '{}' has empty file_path",
            t.title
        );
    }
}

#[test]
#[ignore]
fn test_real_db_null_handling() {
    let conn = open_real_db().expect("backup tarball not found");

    let params = SearchParams {
        has_genre: Some(false),
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for t in &tracks {
        assert!(
            t.genre.is_empty(),
            "track '{}' has genre '{}' but expected none",
            t.title,
            t.genre
        );
    }

    let params = SearchParams {
        has_genre: Some(true),
        limit: Some(50),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for t in &tracks {
        assert!(
            !t.genre.is_empty(),
            "track '{}' has empty genre but expected one",
            t.title
        );
    }
}

#[test]
#[ignore]
fn test_real_db_unicode() {
    let conn = open_real_db().expect("backup tarball not found");
    let all = load_all_tracks(&conn);

    let unicode_tracks: Vec<_> = all
        .iter()
        .filter(|t| !t.title.is_ascii() || !t.artist.is_ascii())
        .collect();

    for t in &unicode_tracks {
        let json = serde_json::to_string(t).unwrap();
        let back: crate::types::Track = serde_json::from_str(&json).unwrap();
        assert_eq!(t.title, back.title, "unicode title round-trip failed");
        assert_eq!(t.artist, back.artist, "unicode artist round-trip failed");
    }
}

#[test]
#[ignore]
fn test_real_db_playlists() {
    let conn = open_real_db().expect("backup tarball not found");
    let playlists = get_playlists(&conn).unwrap();
    assert!(!playlists.is_empty(), "no playlists found");

    let has_folder = playlists.iter().any(|p| p.is_folder);
    let has_regular = playlists.iter().any(|p| !p.is_folder && !p.is_smart);
    assert!(
        has_folder || has_regular,
        "no folders or regular playlists found"
    );

    for p in playlists
        .iter()
        .filter(|p| !p.is_folder && p.track_count > 0)
        .take(3)
    {
        let tracks = get_playlist_tracks(&conn, &p.id, Some(10)).unwrap();
        assert!(
            !tracks.is_empty(),
            "playlist '{}' claims {} tracks but returned none",
            p.name,
            p.track_count
        );
    }
}

#[test]
#[ignore]
fn test_real_db_get_track_by_id() {
    let conn = open_real_db().expect("backup tarball not found");

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
fn test_real_db_library_stats_consistency() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_rated_count_matches_rating_filtered_search() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_all_tracks_load() {
    let conn = open_real_db().expect("backup tarball not found");
    let all = load_all_tracks(&conn);
    assert!(all.len() > 2000, "expected >2000 tracks, got {}", all.len());

    for t in &all {
        assert!(!t.id.is_empty(), "track has empty ID");
    }
}

#[test]
#[ignore]
fn test_real_db_special_characters_in_search() {
    let conn = open_real_db().expect("backup tarball not found");
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
fn test_real_db_sample_exclusion() {
    let conn = open_real_db().expect("backup tarball not found");

    let stats_filtered = get_library_stats(&conn).unwrap();
    let stats_all = get_library_stats_filtered(&conn, false).unwrap();

    assert!(
        stats_filtered.total_tracks <= stats_all.total_tracks,
        "filtered {} > unfiltered {}",
        stats_filtered.total_tracks,
        stats_all.total_tracks
    );

    let diff = stats_all.total_tracks - stats_filtered.total_tracks;
    eprintln!("[integration] Sample exclusion: {diff} sampler tracks filtered out");

    let params = SearchParams {
        exclude_samples: true,
        limit: Some(200),
        ..Default::default()
    };
    let tracks = search_tracks(&conn, &params).unwrap();
    for t in &tracks {
        assert!(
            !is_sampler_path(&t.file_path),
            "sampler track not excluded: {}",
            t.file_path
        );
    }
}

#[test]
#[ignore]
fn test_real_db_genre_normalization_coverage() {
    let conn = open_real_db().expect("backup tarball not found");
    let stats = get_library_stats(&conn).unwrap();

    let mut alias_count = 0;
    let mut canonical_count = 0;
    let mut unknown_count = 0;
    let mut unknown_genres = Vec::new();

    for gc in &stats.genres {
        if gc.name == "(none)" || gc.name.is_empty() {
            continue;
        }
        if crate::genre::canonical_genre_from_alias(&gc.name).is_some() {
            alias_count += gc.count;
        } else if crate::genre::is_known_genre(&gc.name) {
            canonical_count += gc.count;
        } else {
            unknown_count += gc.count;
            unknown_genres.push(format!("{}: {} tracks", gc.name, gc.count));
        }
    }

    eprintln!("[integration] Canonical: {canonical_count} tracks, Alias: {alias_count} tracks");
    eprintln!("[integration] Unknown genres: {}", unknown_genres.len());
    for g in &unknown_genres {
        eprintln!("  {g}");
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

#[test]
fn decode_rating_stars_passthrough_0_to_5() {
    for star in 0..=5 {
        assert_eq!(decode_rating_stars(star), star as u8, "star value {star}");
    }
}

#[test]
fn decode_rating_stars_negative_maps_to_zero() {
    assert_eq!(decode_rating_stars(-1), 0);
    assert_eq!(decode_rating_stars(-100), 0);
    assert_eq!(decode_rating_stars(i32::MIN), 0);
}

#[test]
fn decode_rating_stars_encoded_boundaries() {
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
fn decode_rating_stars_above_255_clamps_to_5() {
    // rating_to_stars treats >255 as 5 stars (catch-all arm)
    assert_eq!(decode_rating_stars(256), 5);
    assert_eq!(decode_rating_stars(1000), 5);
}

#[test]
fn next_day_normal() {
    assert_eq!(next_day("2023-06-15"), "2023-06-16");
}

#[test]
fn next_day_month_boundary_31() {
    assert_eq!(next_day("2023-01-31"), "2023-02-01");
}

#[test]
fn next_day_month_boundary_30() {
    assert_eq!(next_day("2023-04-30"), "2023-05-01");
}

#[test]
fn next_day_feb_non_leap() {
    assert_eq!(next_day("2023-02-28"), "2023-03-01");
}

#[test]
fn next_day_feb_leap_28() {
    assert_eq!(next_day("2024-02-28"), "2024-02-29");
}

#[test]
fn next_day_feb_leap_29() {
    assert_eq!(next_day("2024-02-29"), "2024-03-01");
}

#[test]
fn next_day_year_rollover() {
    assert_eq!(next_day("2023-12-31"), "2024-01-01");
}

#[test]
fn next_day_century_non_leap() {
    assert_eq!(next_day("1900-02-28"), "1900-03-01");
}

#[test]
fn next_day_century_leap() {
    assert_eq!(next_day("2000-02-29"), "2000-03-01");
}

#[test]
fn added_before_includes_boundary_date_with_datetime() {
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
fn added_after_includes_boundary_date_with_datetime() {
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
fn search_has_unknown_genre() {
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
                && !crate::genre::is_known_genre(&t.genre)
                && crate::genre::canonical_genre_from_alias(&t.genre).is_none()
        })
        .collect();
    assert_eq!(unknown.len(), 1);
    assert_eq!(unknown[0].id, "t7");
    assert_eq!(unknown[0].genre, "Wonky Bass");
}
