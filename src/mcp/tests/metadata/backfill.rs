use super::*;

#[tokio::test]
async fn backfill_labels_conflict_page_later_dry_run_does_not_repeat_staging() {
    let db_conn = create_selector_pagination_test_db();
    db_conn
        .execute("UPDATE djmdContent SET LabelID = NULL WHERE ID = 't3'", [])
        .expect("unlabeled staging fixture should update");
    let tracks = db::get_tracks_by_ids(
        &db_conn,
        &["t1".to_string(), "t2".to_string(), "t3".to_string()],
    )
    .expect("label fixture tracks should load");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    {
        let store_conn = server.cache_store_conn().expect("test store should open");
        for track in &tracks {
            let label = if track.id == "t3" {
                "Filled Label"
            } else {
                "Conflicting Label"
            };
            let response = serde_json::json!({"label": label}).to_string();
            store::set_enrichment(
                &store_conn,
                "discogs",
                &crate::domain::metadata::normalize_for_matching(&track.artist),
                &crate::domain::metadata::normalize_for_matching(&track.title),
                Some(&crate::domain::metadata::normalize_for_matching(
                    &track.album,
                )),
                Some("exact"),
                Some(&response),
            )
            .expect("label cache fixture should write");
        }
    }

    let first = server
        .backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(false),
            max_conflicts: Some(1),
            conflict_offset: Some(0),
        }))
        .await
        .expect("mutating label pass should succeed");
    let first_payload = extract_json(&first);
    assert_eq!(first_payload["staged"], 1);
    assert_eq!(first_payload["conflict_page"]["returned"], 1);
    assert_eq!(first_payload["conflict_page"]["next_offset"], 1);
    assert_eq!(first_payload["conflicts_truncated"], true);
    let pending_after_first = server.context.mutation.changes.pending_ids();
    assert_eq!(pending_after_first, vec!["t3".to_string()]);

    let second = server
        .backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(false),
            max_conflicts: Some(1),
            conflict_offset: Some(1),
        }))
        .await
        .expect("later dry-run conflict page should succeed");
    let second_payload = extract_json(&second);
    assert_eq!(second_payload["staged"], 0);
    assert_eq!(second_payload["conflict_page"]["offset"], 1);
    assert_eq!(second_payload["conflict_page"]["returned"], 1);
    assert_eq!(
        second_payload["conflict_page"]["next_offset"],
        serde_json::Value::Null
    );
    assert!(second_payload.get("conflicts_truncated").is_none());
    assert_eq!(
        server.context.mutation.changes.pending_ids(),
        pending_after_first
    );
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_reports_acknowledged_match() {
    let db_conn = create_single_track_test_db(
        "album-cache-persistence-match",
        "/tmp/album-cache-persistence-match.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Persistence Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Persistence Title', AlbumID = NULL
             WHERE ID = 'album-cache-persistence-match';",
        )
        .expect("album persistence fixture should need enrichment");

    let store_dir = tempfile::tempdir().expect("album persistence store should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_string = store_path.to_string_lossy().to_string();
    let store_conn = store::open(&store_path_string).expect("album persistence store should open");
    let http = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all("http://127.0.0.1:9").expect("closed local proxy URL should parse"),
        )
        .timeout(Duration::from_millis(250))
        .build()
        .expect("album persistence HTTP client should build");
    let server =
        create_server_with_store_path(db_conn, store_conn, http, Some(store_path_string.clone()));
    set_test_bandcamp_lookup_override(
        "Album Persistence Artist",
        "Album Persistence Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Album Persistence Title".into(),
            artist_name: "Album Persistence Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Album Persistence Label".into()),
            tags: vec![],
            album: Some("Durable Album".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/durable".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album persistence handler should finish within five seconds")
    .expect("album persistence handler should return partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enrichment"]["requested"], 1);
    assert_eq!(payload["auto_enrichment"]["matched"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enriched"], 1);
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_distinguishes_no_match_from_lookup_failure() {
    let db_conn = create_single_track_test_db("album-outcome-none", "/tmp/album-outcome-none.flac");
    insert_test_track(
        &db_conn,
        "album-outcome-error",
        "Album Outcome Error",
        "g1",
        "/tmp/album-outcome-error.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Outcome Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Outcome None', AlbumID = NULL
             WHERE ID = 'album-outcome-none';
             UPDATE djmdContent SET AlbumID = NULL WHERE ID = 'album-outcome-error';",
        )
        .expect("album outcome fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override("Album Outcome Artist", "Album Outcome None", Ok(None));
    set_test_bandcamp_lookup_override(
        "Album Outcome Artist",
        "Album Outcome Error",
        Err("synthetic Bandcamp album failure".into()),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album outcome handler should finish within five seconds")
    .expect("album outcome handler should return structured partial success");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 0);
    assert_eq!(report["no_match"], 1);
    assert_eq!(report["lookup_failed"], 1);
    assert_eq!(report["cache_writes_succeeded"], 1);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], true);
    assert_eq!(report["by_provider"]["bandcamp"]["requested"], 2);
    assert_eq!(report["by_provider"]["bandcamp"]["no_match"], 1);
    assert_eq!(report["by_provider"]["bandcamp"]["lookup_failed"], 1);
    assert_eq!(report["failures"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["failures"][0]["provider"], "bandcamp");
    assert_eq!(
        report["failures"][0]["normalized_title"],
        "album outcome error"
    );
    assert_eq!(report["failures"][0]["kind"], "lookup_failed");
    assert_eq!(payload["staged"], 0);

    let connection = server
        .cache_store_conn()
        .expect("album outcome store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Album Outcome Artist");
    let none_title = crate::domain::metadata::normalize_for_matching("Album Outcome None");
    let error_title = crate::domain::metadata::normalize_for_matching("Album Outcome Error");
    let no_match = store::get_enrichment(
        &connection,
        "bandcamp",
        &norm_artist,
        &none_title,
        None,
        false,
    )
    .expect("album no-match cache should read")
    .expect("album no-match should persist");
    assert_eq!(no_match.match_quality.as_deref(), Some("none"));
    assert!(no_match.response_json.is_none());
    assert!(
        store::get_enrichment(
            &connection,
            "bandcamp",
            &norm_artist,
            &error_title,
            None,
            false,
        )
        .expect("album lookup-failure cache should read")
        .is_none(),
        "album lookup failures must remain retryable"
    );
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_reports_acknowledged_no_matches() {
    let db_conn = create_single_track_test_db(
        "year-cache-persistence-none",
        "/tmp/year-cache-persistence-none.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Persistence Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Persistence Title', ReleaseYear = 0
             WHERE ID = 'year-cache-persistence-none';",
        )
        .expect("year persistence fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override(
        "Year Persistence Artist",
        "Year Persistence Title",
        Ok(None),
    );
    set_test_musicbrainz_lookup_override(
        "Year Persistence Artist",
        "Year Persistence Title",
        Ok(None),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year persistence handler should finish within five seconds")
    .expect("year persistence handler should return partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 2);
    assert_eq!(payload["auto_enriched"], 0);
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_rescans_both_positive_provider_rows() {
    let db_conn = create_single_track_test_db("year-positive-both", "/tmp/year-positive-both.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Positive Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Positive Title', ReleaseYear = 0
             WHERE ID = 'year-positive-both';",
        )
        .expect("year positive fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_musicbrainz_lookup_override(
        "Year Positive Artist",
        "Year Positive Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Year Positive Title".into(),
                artist: "Year Positive Artist".into(),
                first_release_date: Some("2022-03-04".into()),
                label: Some("Year Positive MusicBrainz".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Year Positive Artist",
        "Year Positive Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Year Positive Title".into(),
            artist_name: "Year Positive Artist".into(),
            release_date: Some("2023-05-06".into()),
            label: Some("Year Positive Bandcamp".into()),
            tags: vec![],
            album: Some("Year Positive Album".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/year-positive".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year positive handler should finish within five seconds")
    .expect("year positive handler should succeed");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 2);
    assert_eq!(report["no_match"], 0);
    assert_eq!(report["lookup_failed"], 0);
    assert_eq!(report["cache_writes_succeeded"], 2);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], false);
    for provider in ["bandcamp", "musicbrainz"] {
        assert_eq!(report["by_provider"][provider]["requested"], 1);
        assert_eq!(report["by_provider"][provider]["matched"], 1);
        assert_eq!(report["by_provider"][provider]["cache_writes_succeeded"], 1);
    }
    assert_eq!(payload["summary"]["filled_by_source"]["musicbrainz"], 1);
    assert_eq!(payload["summary"]["filled_by_source"]["bandcamp"], 0);
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("year-positive-both")
            .expect("positive year should be staged")
            .year,
        Some(2022),
        "MusicBrainz remains ahead of Bandcamp in the year cascade"
    );

    let connection = server
        .cache_store_conn()
        .expect("year positive store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Year Positive Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Year Positive Title");
    for (provider, date_field, expected_date) in [
        ("bandcamp", "release_date", "2023-05-06"),
        ("musicbrainz", "first_release_date", "2022-03-04"),
    ] {
        let cached = store::get_enrichment(
            &connection,
            provider,
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("year positive cache should read")
        .expect("each positive provider row should persist");
        assert_eq!(cached.match_quality.as_deref(), Some("exact"));
        let response: serde_json::Value = serde_json::from_str(
            cached
                .response_json
                .as_deref()
                .expect("positive year row should retain provider JSON"),
        )
        .expect("positive year row JSON should parse");
        assert_eq!(response[date_field], expected_date);
    }
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_keeps_lookup_failure_retryable() {
    let db_conn = create_single_track_test_db("year-outcome-error", "/tmp/year-outcome-error.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Outcome Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Outcome Title', ReleaseYear = 0
             WHERE ID = 'year-outcome-error';",
        )
        .expect("year outcome fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_musicbrainz_lookup_override(
        "Year Outcome Artist",
        "Year Outcome Title",
        Err("synthetic MusicBrainz year failure".into()),
    );
    set_test_bandcamp_lookup_override("Year Outcome Artist", "Year Outcome Title", Ok(None));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year outcome handler should finish within five seconds")
    .expect("year outcome handler should return structured partial success");
    let payload = extract_json(&result);
    let report = &payload["auto_enrichment"];
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(report["requested"], 2);
    assert_eq!(report["matched"], 0);
    assert_eq!(report["no_match"], 1);
    assert_eq!(report["lookup_failed"], 1);
    assert_eq!(report["cache_writes_succeeded"], 1);
    assert_eq!(report["cache_writes_failed"], 0);
    assert_eq!(report["operation_failed"], true);
    assert_eq!(report["by_provider"]["musicbrainz"]["lookup_failed"], 1);
    assert_eq!(report["by_provider"]["bandcamp"]["no_match"], 1);
    assert_eq!(report["failures"].as_array().map(Vec::len), Some(1));
    assert_eq!(report["failures"][0]["provider"], "musicbrainz");
    assert_eq!(
        report["failures"][0]["normalized_title"],
        "year outcome title"
    );
    assert_eq!(report["failures"][0]["kind"], "lookup_failed");
    assert_eq!(payload["staged"], 0);

    let connection = server
        .cache_store_conn()
        .expect("year outcome store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Year Outcome Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Year Outcome Title");
    assert!(
        store::get_enrichment(
            &connection,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("MusicBrainz year cache should read")
        .is_none(),
        "year lookup failures must remain retryable"
    );
    let no_match = store::get_enrichment(
        &connection,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        false,
    )
    .expect("Bandcamp year cache should read")
    .expect("year no-match should persist");
    assert_eq!(no_match.match_quality.as_deref(), Some("none"));
    assert!(no_match.response_json.is_none());
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_auto_enriches_both_providers_and_preserves_precedence()
 {
    let db_conn = create_single_track_test_db(
        "labels-auto-enrich-match",
        "/music/labels-auto-enrich-match.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Auto Match Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Auto Match Title', LabelID = NULL
             WHERE ID = 'labels-auto-enrich-match';",
        )
        .expect("label fixture should become unlabeled");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    set_test_musicbrainz_lookup_override(
        "Auto Match Artist",
        "Auto Match Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Auto Match Title".into(),
                artist: "Auto Match Artist".into(),
                first_release_date: Some("2025-01-01".into()),
                label: Some("MusicBrainz Label".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Auto Match Artist",
        "Auto Match Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Auto Match Title".into(),
            artist_name: "Auto Match Artist".into(),
            release_date: Some("2025-01-02".into()),
            label: Some("Bandcamp Label".into()),
            tags: vec![],
            album: Some("Encoded Paths".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/senorita".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("dual-provider label handler should finish within five seconds")
    .expect("dual-provider label hydration should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 1);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 1);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["matched"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], false);
    assert_eq!(
        payload["auto_enrichment"]["failures"],
        serde_json::json!([])
    );
    for provider in ["musicbrainz", "bandcamp"] {
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["requested"],
            1
        );
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["matched"],
            1
        );
        assert_eq!(
            payload["auto_enrichment"]["by_provider"][provider]["cache_writes_succeeded"],
            1
        );
    }
    assert_eq!(payload["staged"], 1);
    let pending = server
        .context
        .mutation
        .changes
        .get("labels-auto-enrich-match")
        .expect("label fill should be staged");
    assert_eq!(pending.label.as_deref(), Some("MusicBrainz Label"));

    let store_conn = server.cache_store_conn().expect("test store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Auto Match Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Auto Match Title");
    for (provider, expected_label) in [
        ("musicbrainz", "MusicBrainz Label"),
        ("bandcamp", "Bandcamp Label"),
    ] {
        let cached = store::get_enrichment(
            &store_conn,
            provider,
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("provider cache should be readable")
        .expect("provider result should be cached");
        assert_eq!(cached.match_quality.as_deref(), Some("exact"));
        let response: serde_json::Value = serde_json::from_str(
            cached
                .response_json
                .as_deref()
                .expect("positive label row should retain provider JSON"),
        )
        .expect("positive label row JSON should parse");
        assert_eq!(response["label"], expected_label);
    }
}

#[tokio::test]
async fn backfill_albums_backfill_cache_persistence_surfaces_selective_failure_and_rescans_success()
{
    let db_conn = create_single_track_test_db("album-cache-accept", "/tmp/album-cache-accept.flac");
    insert_test_track(
        &db_conn,
        "album-cache-reject",
        "Album Cache Reject",
        "g1",
        "/tmp/album-cache-reject.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Album Partial Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Album Cache Accept', AlbumID = NULL
             WHERE ID = 'album-cache-accept';
             UPDATE djmdContent SET AlbumID = NULL WHERE ID = 'album-cache-reject';",
        )
        .expect("album partial fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    server
        .cache_store_conn()
        .expect("album partial store should open")
        .execute_batch(
            "CREATE TRIGGER fail_selected_album_cache
             BEFORE INSERT ON enrichment_cache
             WHEN NEW.provider = 'bandcamp' AND NEW.query_title = 'album cache reject'
             BEGIN
                 SELECT RAISE(FAIL, 'queue send failed acknowledgement canceled writer open failed');
             END;",
        )
        .expect("album partial failure trigger should install");
    for (title, album) in [
        ("Album Cache Accept", "Persisted Album"),
        ("Album Cache Reject", "Rejected Album"),
    ] {
        set_test_bandcamp_lookup_override(
            "Album Partial Artist",
            title,
            Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
                track_title: title.into(),
                artist_name: "Album Partial Artist".into(),
                release_date: Some("2024-01-01".into()),
                label: Some("Album Partial Label".into()),
                tags: vec![],
                album: Some(album.into()),
                cover_image: None,
                bandcamp_url: "https://example.bandcamp.com/track/partial".into(),
                score: 100,
            })),
        );
    }

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("album partial handler should finish within five seconds")
    .expect("album partial handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["matched"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["writer_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    let failure = &payload["auto_enrichment"]["failures"][0];
    assert_eq!(failure["provider"], "bandcamp");
    assert_eq!(failure["normalized_title"], "album cache reject");
    assert_eq!(failure["kind"], "cache_write_failed");
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("album-cache-accept")
            .expect("persisted album should be re-scanned")
            .album
            .as_deref(),
        Some("Persisted Album")
    );
    assert!(
        server
            .context
            .mutation
            .changes
            .get("album-cache-reject")
            .is_none()
    );
}

#[tokio::test]
async fn backfill_years_backfill_cache_persistence_rejects_every_key_when_writer_open_fails() {
    let db_conn = create_single_track_test_db(
        "year-writer-open-failure",
        "/tmp/year-writer-open-failure.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Year Open Failure Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Year Open Failure Title', ReleaseYear = 0
             WHERE ID = 'year-writer-open-failure';",
        )
        .expect("year writer-open fixture should need enrichment");
    let store_dir = tempfile::tempdir().expect("year writer-open store directory should create");
    let usable_path = store_dir.path().join("usable.sqlite3");
    let usable_path_string = usable_path.to_string_lossy().to_string();
    let store_conn = store::open(&usable_path_string).expect("usable year store should open");
    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_dir.path().to_string_lossy().to_string()),
    );
    set_test_bandcamp_lookup_override(
        "Year Open Failure Artist",
        "Year Open Failure Title",
        Ok(None),
    );
    set_test_musicbrainz_lookup_override(
        "Year Open Failure Artist",
        "Year Open Failure Title",
        Ok(None),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("year writer-open handler should finish within five seconds")
    .expect("year writer-open handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 2);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 0);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 2);
    assert_eq!(payload["auto_enrichment"]["writer_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    let failures = payload["auto_enrichment"]["failures"]
        .as_array()
        .expect("year writer-open failures should be an array");
    assert_eq!(failures.len(), 2);
    assert!(failures.iter().all(|failure| {
        failure["kind"] == "writer_open_failed"
            && failure["normalized_artist"] == "year open failure artist"
            && failure["normalized_title"] == "year open failure title"
    }));
    let connection = store::open(&usable_path_string).expect("usable year store should reopen");
    for provider in ["bandcamp", "musicbrainz"] {
        assert!(
            store::get_enrichment(
                &connection,
                provider,
                "year open failure artist",
                "year open failure title",
                None,
                false,
            )
            .expect("usable year cache should read")
            .is_none()
        );
    }
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_surfaces_selective_provider_failure() {
    let db_conn =
        create_single_track_test_db("label-cache-selective", "/tmp/label-cache-selective.flac");
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Label Selective Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Label Selective Title', LabelID = NULL
             WHERE ID = 'label-cache-selective';",
        )
        .expect("label selective fixture should need enrichment");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    server
        .cache_store_conn()
        .expect("label selective store should open")
        .execute_batch(
            "CREATE TRIGGER fail_selected_label_cache
             BEFORE INSERT ON enrichment_cache
             WHEN NEW.provider = 'bandcamp' AND NEW.query_title = 'label selective title'
             BEGIN
                 SELECT RAISE(FAIL, 'selected label cache failure');
             END;",
        )
        .expect("label selective failure trigger should install");
    set_test_musicbrainz_lookup_override(
        "Label Selective Artist",
        "Label Selective Title",
        Ok(Some(
            crate::adapters::providers::musicbrainz::MusicBrainzResult {
                recording_title: "Label Selective Title".into(),
                artist: "Label Selective Artist".into(),
                first_release_date: Some("2024-01-01".into()),
                label: Some("Persisted Label".into()),
                score: 100,
            },
        )),
    );
    set_test_bandcamp_lookup_override(
        "Label Selective Artist",
        "Label Selective Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Label Selective Title".into(),
            artist_name: "Label Selective Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Rejected Label".into()),
            tags: vec![],
            album: Some("Encoded Paths".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/label-selective".into(),
            score: 100,
        })),
    );

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(false),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("label selective handler should finish within five seconds")
    .expect("label selective handler should return structured partial success");
    let payload = extract_json(&result);
    assert_eq!(payload["auto_enriched"], 2);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 1);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["provider"],
        "bandcamp"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["kind"],
        "cache_write_failed"
    );
    assert_eq!(payload["staged"], 1);
    assert_eq!(
        server
            .context
            .mutation
            .changes
            .get("label-cache-selective")
            .expect("persisted label should be re-scanned")
            .label
            .as_deref(),
        Some("Persisted Label")
    );
}

#[tokio::test]
async fn backfill_labels_backfill_cache_persistence_keeps_provider_errors_retryable_and_caches_no_match()
 {
    let db_conn = create_single_track_test_db(
        "labels-auto-enrich-error",
        "/music/labels-auto-enrich-error.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Auto Error Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Auto Error Title', LabelID = NULL
             WHERE ID = 'labels-auto-enrich-error';",
        )
        .expect("label fixture should become unlabeled");
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    set_test_musicbrainz_lookup_override(
        "Auto Error Artist",
        "Auto Error Title",
        Err("synthetic MusicBrainz failure".into()),
    );
    set_test_bandcamp_lookup_override("Auto Error Artist", "Auto Error Title", Ok(None));

    let result = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("label failure handler should finish within five seconds")
    .expect("provider failures should not fail the whole label pass");
    let payload = extract_json(&result);

    assert_eq!(payload["auto_enriched"], 0);
    assert_eq!(payload["auto_enriched_by_provider"]["musicbrainz"], 0);
    assert_eq!(payload["auto_enriched_by_provider"]["bandcamp"], 0);
    assert_eq!(payload["auto_enrichment"]["requested"], 2);
    assert_eq!(payload["auto_enrichment"]["no_match"], 1);
    assert_eq!(payload["auto_enrichment"]["lookup_failed"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_succeeded"], 1);
    assert_eq!(payload["auto_enrichment"]["cache_writes_failed"], 0);
    assert_eq!(payload["auto_enrichment"]["operation_failed"], true);
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["musicbrainz"]["requested"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["musicbrainz"]["lookup_failed"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["bandcamp"]["requested"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["by_provider"]["bandcamp"]["no_match"],
        1
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["provider"],
        "musicbrainz"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["normalized_title"],
        "auto error title"
    );
    assert_eq!(
        payload["auto_enrichment"]["failures"][0]["kind"],
        "lookup_failed"
    );
    assert_eq!(payload["staged"], 0);

    let store_conn = server.cache_store_conn().expect("test store should open");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Auto Error Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Auto Error Title");
    assert!(
        store::get_enrichment(
            &store_conn,
            "musicbrainz",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("MusicBrainz cache should be readable")
        .is_none(),
        "provider errors must remain retryable"
    );
    let bandcamp = store::get_enrichment(
        &store_conn,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        false,
    )
    .expect("Bandcamp cache should be readable")
    .expect("completed no-match should be durable");
    assert_eq!(bandcamp.match_quality.as_deref(), Some("none"));
    assert!(bandcamp.response_json.is_none());
}

#[tokio::test]
async fn metadata_backfill_cancellation_aborts_provider_work_and_quiesces_writer() {
    use crate::application::metadata::enrichment::{
        MetadataEnrichmentProvider, install_test_lookup_pause, metadata_writer_active_for_test,
    };

    let db_conn = create_single_track_test_db(
        "metadata-cancellation-track",
        "/tmp/metadata-cancellation-track.flac",
    );
    db_conn
        .execute_batch(
            "UPDATE djmdArtist SET Name = 'Cancellation Artist' WHERE ID = 'a1';
             UPDATE djmdContent
             SET Title = 'Cancellation Title', AlbumID = NULL
             WHERE ID = 'metadata-cancellation-track';",
        )
        .expect("metadata cancellation fixture should need enrichment");
    let (server, _store_dir, store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_bandcamp_lookup_override(
        "Cancellation Artist",
        "Cancellation Title",
        Ok(Some(crate::adapters::providers::bandcamp::BandcampResult {
            track_title: "Cancellation Title".into(),
            artist_name: "Cancellation Artist".into(),
            release_date: Some("2024-01-01".into()),
            label: Some("Cancellation Label".into()),
            tags: vec![],
            album: Some("Must Not Persist".into()),
            cover_image: None,
            bandcamp_url: "https://example.bandcamp.com/track/cancel".into(),
            score: 100,
        })),
    );
    let norm_artist = crate::domain::metadata::normalize_for_matching("Cancellation Artist");
    let norm_title = crate::domain::metadata::normalize_for_matching("Cancellation Title");
    let (pause_guard, reached, _release) = install_test_lookup_pause(
        MetadataEnrichmentProvider::Bandcamp,
        norm_artist.clone(),
        norm_title.clone(),
    );

    let task_server = server.clone();
    let mut handler = tokio::spawn(async move {
        task_server
            .backfill_albums(Parameters(BackfillAlbumsParams {
                dry_run: Some(false),
                auto_enrich: Some(true),
            }))
            .await
    });
    if tokio::time::timeout(Duration::from_secs(5), reached.notified())
        .await
        .is_err()
    {
        handler.abort();
        drop(pause_guard);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut handler)
                .await
                .is_ok(),
            "timed-out metadata handler cleanup should join within five seconds"
        );
        panic!("metadata provider did not reach the cancellation barrier");
    }
    if tokio::time::timeout(Duration::from_secs(5), async {
        while !metadata_writer_active_for_test(&store_path) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .is_err()
    {
        handler.abort();
        drop(pause_guard);
        assert!(
            tokio::time::timeout(Duration::from_secs(5), &mut handler)
                .await
                .is_ok(),
            "inactive metadata writer cleanup should join within five seconds"
        );
        panic!("metadata writer did not become active before cancellation");
    }

    handler.abort();
    let cancelled = match tokio::time::timeout(Duration::from_secs(5), &mut handler).await {
        Ok(result) => result.expect_err("metadata handler should be cancelled"),
        Err(_) => {
            // Only release paused provider work as bounded cleanup after the
            // handler failed to observe cancellation on its own.
            drop(pause_guard);
            assert!(
                tokio::time::timeout(Duration::from_secs(5), &mut handler)
                    .await
                    .is_ok(),
                "timed-out cancelled metadata handler cleanup should join within five seconds"
            );
            panic!("metadata handler cancellation did not join within five seconds");
        }
    };
    assert!(cancelled.is_cancelled());
    // Cancellation is now confirmed, so releasing the test barrier cannot let
    // provider work race past the cancellation boundary.
    drop(pause_guard);

    tokio::time::timeout(Duration::from_secs(5), async {
        while metadata_writer_active_for_test(&store_path) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("metadata writer should quiesce within five seconds");

    let connection = server
        .cache_store_conn()
        .expect("metadata cancellation store should remain usable");
    assert!(
        store::get_enrichment(
            &connection,
            "bandcamp",
            &norm_artist,
            &norm_title,
            None,
            false,
        )
        .expect("metadata cancellation cache should read")
        .is_none(),
        "cancelled provider must not persist a cache row after writer quiescence"
    );
}

#[tokio::test]
async fn metadata_auto_enrichment_output_is_conditional_and_reports_zero_work() {
    let db_conn = create_single_track_test_db(
        "metadata-output-zero-work",
        "/tmp/metadata-output-zero-work.flac",
    );
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    let without_auto_enrich = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(false),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("non-auto label handler should finish within five seconds")
    .expect("label output without auto enrichment should succeed");
    let without_auto_enrich = extract_json(&without_auto_enrich);
    assert!(without_auto_enrich.get("auto_enrichment").is_none());
    assert!(without_auto_enrich.get("auto_enriched").is_none());

    let labels = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
            max_conflicts: None,
            conflict_offset: None,
        })),
    )
    .await
    .expect("zero-work label handler should finish within five seconds")
    .expect("zero-work label auto enrichment should succeed");
    let years = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_years(Parameters(BackfillYearsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("zero-work year handler should finish within five seconds")
    .expect("zero-work year auto enrichment should succeed");
    let albums = tokio::time::timeout(
        Duration::from_secs(5),
        server.backfill_albums(Parameters(BackfillAlbumsParams {
            dry_run: Some(true),
            auto_enrich: Some(true),
        })),
    )
    .await
    .expect("zero-work album handler should finish within five seconds")
    .expect("zero-work album auto enrichment should succeed");

    for (name, result) in [("labels", labels), ("years", years), ("albums", albums)] {
        let payload = extract_json(&result);
        let report = &payload["auto_enrichment"];
        assert_eq!(report["requested"], 0, "{name} requested count");
        assert_eq!(report["matched"], 0, "{name} matched count");
        assert_eq!(report["no_match"], 0, "{name} no-match count");
        assert_eq!(report["cache_writes_succeeded"], 0, "{name} write count");
        assert_eq!(report["operation_failed"], false, "{name} failure flag");
        assert_eq!(report["failures"], serde_json::json!([]));
        assert_eq!(report["failures_truncated"], false);
        assert!(report["by_provider"].get("bandcamp").is_some());
        assert!(report["by_provider"].get("musicbrainz").is_some());
        assert_eq!(payload["auto_enriched"], 0);
    }
}

#[test]
fn metadata_auto_enrichment_output_schema_exposes_typed_label_report() {
    fn contains_fields(
        root: &serde_json::Value,
        value: &serde_json::Value,
        fields: &[&str],
    ) -> bool {
        if let Some(reference) = value.get("$ref").and_then(serde_json::Value::as_str)
            && let Some(target) = root.pointer(reference.trim_start_matches('#'))
        {
            return contains_fields(root, target, fields);
        }
        if let Some(properties) = value
            .get("properties")
            .and_then(serde_json::Value::as_object)
            && fields.iter().all(|field| properties.contains_key(*field))
        {
            return true;
        }
        match value {
            serde_json::Value::Array(values) => values
                .iter()
                .any(|value| contains_fields(root, value, fields)),
            serde_json::Value::Object(values) => values
                .values()
                .any(|value| contains_fields(root, value, fields)),
            _ => false,
        }
    }

    let tool = ReklawdboxServer::build_tool_router()
        .list_all()
        .into_iter()
        .find(|tool| tool.name.as_ref() == "backfill_labels")
        .expect("backfill_labels should exist in the live router");
    let schema = serde_json::to_value(
        tool.output_schema
            .as_ref()
            .expect("backfill_labels should advertise outputSchema"),
    )
    .expect("backfill_labels output schema should serialize");
    let root_properties = schema["properties"]
        .as_object()
        .expect("backfill_labels output schema should expose root properties");
    let auto_enrichment = root_properties
        .get("auto_enrichment")
        .expect("backfill_labels output schema should expose auto_enrichment");
    assert!(
        !schema["required"]
            .as_array()
            .expect("backfill_labels required fields should be an array")
            .iter()
            .any(|field| field == "auto_enrichment"),
        "auto_enrichment is conditional on auto_enrich=true"
    );
    assert!(
        contains_fields(
            &schema,
            auto_enrichment,
            &[
                "operation_failed",
                "requested",
                "matched",
                "no_match",
                "lookup_failed",
                "cache_writes_succeeded",
                "cache_writes_failed",
                "serialization_failed",
                "worker_failed",
                "writer_failed",
                "by_provider",
                "failures",
                "failures_truncated",
            ],
        ),
        "unexpected auto_enrichment schema: {auto_enrichment:#}"
    );
}
