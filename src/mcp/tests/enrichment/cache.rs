use super::support::{
    assert_cache_write_summary, discogs_match, install_enrichment_insert_failure,
    run_discogs_batch_with_timeout, set_enrich_test_track_title,
};
use crate::mcp::enrichment::{
    EnrichTracksParams, LookupDiscogsParams, set_test_discogs_lookup_override,
};
use crate::mcp::library::SearchFilterParams;

use rmcp::handler::server::wrapper::Parameters;
use rusqlite::params;

use crate::adapters::state as store;

use super::super::common::{
    create_enrich_cache_writer_test_server, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track,
};

fn assert_cache_write_failure_context(
    payload: &serde_json::Value,
    track_id: &str,
    title: &str,
    error_prefix: &str,
) {
    let failure = payload["failures"]
        .as_array()
        .expect("failures should be an array")
        .iter()
        .find(|failure| failure["track_id"] == track_id)
        .unwrap_or_else(|| panic!("failure for {track_id} should be present"));
    assert_eq!(failure["artist"], "Aníbal");
    assert_eq!(failure["title"], title);
    assert_eq!(failure["provider"], "discogs");
    assert!(
        failure["error"]
            .as_str()
            .expect("cache-write failure should include an error string")
            .starts_with(error_prefix),
        "cache-write error should start with {error_prefix}: {failure:?}"
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_persists_no_match_before_skipped() {
    let db_conn = create_single_track_test_db("ack-no-match", "/tmp/ack-no-match.flac");
    let title = "Ack No Match";
    set_enrich_test_track_title(&db_conn, "ack-no-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-no-match"],
        "acknowledged no-match enrichment",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 1);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 1, 1, 0);

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry = store::get_enrichment(
        &conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("skipped no-match should have a durable negative cache row");
    assert_eq!(entry.match_quality.as_deref(), Some("none"));
    assert!(entry.response_json.is_none());
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_failed_no_match_counts_only_failed() {
    let db_conn = create_single_track_test_db("ack-failed-no-match", "/tmp/ack-failed.flac");
    let title = "Ack Failed No Match";
    set_enrich_test_track_title(&db_conn, "ack-failed-no-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    install_enrichment_insert_failure(&server, title);
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-failed-no-match"],
        "failed no-match cache write",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 1, 0, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-failed-no-match",
        title,
        "cache write failed:",
    );

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &norm_title,
            Some(&norm_album),
            false,
        )
        .expect("cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_open_failure_is_per_attempt() {
    let db_conn = create_single_track_test_db("ack-open-failure", "/tmp/ack-open-failure.flac");
    let title = "Ack Writer Open Failure";
    set_enrich_test_track_title(&db_conn, "ack-open-failure", title);
    let initialized_store_dir = tempfile::tempdir().expect("initialized store dir should create");
    let initialized_store_path = initialized_store_dir.path().join("internal.sqlite3");
    let initialized_store_path_str = initialized_store_path.to_string_lossy().to_string();
    let store_conn =
        store::open(&initialized_store_path_str).expect("initialized store should open");
    let writer_directory = tempfile::tempdir().expect("writer directory should create");
    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(writer_directory.path().to_string_lossy().to_string()),
    );
    set_test_discogs_lookup_override("Aníbal", title, Some("Encoded Paths"), Ok(None));

    let result =
        run_discogs_batch_with_timeout(&server, &["ack-open-failure"], "writer-open cache failure")
            .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 1, 0, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-open-failure",
        title,
        "cache writer open failed:",
    );

    let conn =
        store::open(&initialized_store_path_str).expect("initialized store should remain readable");
    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &norm_title,
            Some(&norm_album),
            false,
        )
        .expect("cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_mixed_success_and_failure_are_exact() {
    let db_conn = create_single_track_test_db("ack-mixed-success", "/tmp/ack-mixed-success.flac");
    let successful_raw_title = "Ack Mixed Success";
    set_enrich_test_track_title(&db_conn, "ack-mixed-success", successful_raw_title);
    insert_test_track(
        &db_conn,
        "ack-mixed-failure",
        "Matched But Uncached",
        "g1",
        "/tmp/ack-mixed-failure.flac",
    );
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    install_enrichment_insert_failure(&server, "Matched But Uncached");
    set_test_discogs_lookup_override(
        "Aníbal",
        successful_raw_title,
        Some("Encoded Paths"),
        Ok(None),
    );
    set_test_discogs_lookup_override(
        "Aníbal",
        "Matched But Uncached",
        Some("Encoded Paths"),
        Ok(Some(discogs_match("Matched But Uncached"))),
    );

    let result = run_discogs_batch_with_timeout(
        &server,
        &["ack-mixed-success", "ack-mixed-failure"],
        "mixed cache-write outcomes",
    )
    .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 1);
    assert_eq!(payload["summary"]["failed"], 1);
    assert_cache_write_summary(&payload, 2, 1, 1);
    assert_cache_write_failure_context(
        &payload,
        "ack-mixed-failure",
        "Matched But Uncached",
        "cache write failed:",
    );

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let success_title = crate::domain::metadata::normalize_for_matching(successful_raw_title);
    let failed_title = crate::domain::metadata::normalize_for_matching("Matched But Uncached");
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &success_title,
            Some(&norm_album),
            false,
        )
        .expect("successful cache read should succeed")
        .is_some()
    );
    assert!(
        store::get_enrichment(
            &conn,
            "discogs",
            &norm_artist,
            &failed_title,
            Some(&norm_album),
            false,
        )
        .expect("failed cache read should succeed")
        .is_none()
    );
}

#[tokio::test]
async fn enrich_tracks_enrich_cache_writer_persists_match_before_enriched() {
    let db_conn = create_single_track_test_db("ack-match", "/tmp/ack-match.flac");
    let title = "Ack Persisted Match";
    set_enrich_test_track_title(&db_conn, "ack-match", title);
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);
    set_test_discogs_lookup_override(
        "Aníbal",
        title,
        Some("Encoded Paths"),
        Ok(Some(discogs_match(title))),
    );

    let result =
        run_discogs_batch_with_timeout(&server, &["ack-match"], "acknowledged match enrichment")
            .await;
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["enriched"], 1);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 1, 1, 0);

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    let conn = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry = store::get_enrichment(
        &conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("enriched match should have a durable cache row");
    assert_eq!(entry.match_quality.as_deref(), Some("exact"));
    assert!(entry.response_json.is_some());
}

#[tokio::test]
async fn lookup_discogs_no_match_payload_is_consistent_across_live_and_cache_paths() {
    let db_conn =
        create_single_track_test_db("discogs-no-match-track", "/tmp/discogs-no-match.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");
    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());

    let artist = "Discogs NoMatch Artist";
    let title = "Discogs NoMatch Title";
    set_test_discogs_lookup_override(artist, title, None, Ok(None));

    let live_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(true),
        }))
        .await
        .expect("live discogs no-match should succeed");
    let live_payload = extract_json(&live_result);
    assert_eq!(live_payload["result"], serde_json::Value::Null);
    assert_eq!(live_payload["cache_hit"], false);
    assert!(
        live_payload.get("cached_at").is_none(),
        "live payload should omit cached_at"
    );

    let cache_result = server
        .lookup_discogs(Parameters(LookupDiscogsParams {
            track_id: None,
            artist: Some(artist.to_string()),
            title: Some(title.to_string()),
            album: None,
            force_refresh: Some(false),
        }))
        .await
        .expect("cached discogs no-match should succeed");
    let cache_payload = extract_json(&cache_result);
    assert_eq!(cache_payload["result"], serde_json::Value::Null);
    assert_eq!(cache_payload["cache_hit"], true);

    let cache_hit_timestamp = cache_payload
        .get("cached_at")
        .and_then(serde_json::Value::as_str)
        .expect("cached no-match payload should include cached_at");
    let norm_artist = crate::domain::metadata::normalize_for_matching(artist);
    let norm_title = crate::domain::metadata::normalize_for_matching(title);
    let cache_entry = {
        let store = server
            .cache_store_conn()
            .expect("internal store should be available");
        store::get_enrichment(&store, "discogs", &norm_artist, &norm_title, None, false)
            .expect("cache read should succeed")
            .expect("discogs no-match lookup should create cache entry")
    };
    assert!(
        cache_entry.response_json.is_none(),
        "discogs no-match cache entry should store null response as no payload"
    );
    assert_eq!(
        cache_hit_timestamp,
        cache_entry.created_at.as_str(),
        "cached_at should match persisted cache timestamp"
    );
}

#[tokio::test]
async fn enrich_tracks_discogs_skip_cached_reports_cached_counts() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    db_conn
        .execute(
            "INSERT INTO djmdContent (
                    ID, Title, ArtistID, AlbumID, GenreID, KeyID, ColorID, LabelID, RemixerID,
                    BPM, Rating, Commnt, ReleaseYear, Length, FolderPath, DJPlayCount, BitRate,
                    SampleRate, FileType, created_at, rb_local_deleted
                ) VALUES (
                    ?1, ?2, 'a1', 'al1', 'g1', 'k1', 'c1', 'l1', '',
                    12700, 153, 'cached batch test', 2025, 230, ?3, '0', 1411,
                    44100, 5, '2025-01-01', 0
                )",
            params![
                "cached-track-2",
                "Corazón Cached",
                "/tmp/cached-track-2.flac"
            ],
        )
        .expect("second test track should insert");

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let artist = "Aníbal";
    let title_one = "Señorita";
    let title_two = "Corazón Cached";
    let norm_artist = crate::domain::metadata::normalize_for_matching(artist);
    let norm_title_one = crate::domain::metadata::normalize_for_matching(title_one);
    let norm_title_two = crate::domain::metadata::normalize_for_matching(title_two);
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");

    let cached_one = serde_json::json!({
        "title": "Anibal - Senorita",
        "genres": ["Electronic"],
        "styles": ["Deep House"],
        "fuzzy_match": false
    })
    .to_string();
    let cached_two = serde_json::json!({
        "title": "Anibal - Corazon Cached",
        "genres": ["Electronic"],
        "styles": ["House"],
        "fuzzy_match": false
    })
    .to_string();

    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_one),
    )
    .expect("first sentinel discogs cache entry should write");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        Some("exact"),
        Some(&cached_two),
    )
    .expect("second sentinel discogs cache entry should write");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );

    let params = EnrichTracksParams {
        filters: SearchFilterParams::default(),
        track_ids: Some(vec![
            "cached-track-1".to_string(),
            "cached-track-2".to_string(),
        ]),
        playlist_id: None,
        max_tracks: Some(10),
        offset: None,
        providers: Some(vec![
            crate::application::enrichment::model::EnrichmentProvider::Discogs,
        ]),
        skip_cached: Some(true),
        force_refresh: Some(false),
        concurrency: None,
    };

    let first_result = server
        .enrich_tracks(Parameters(params))
        .await
        .expect("enrich_tracks should succeed when everything is cached");
    let first_payload = extract_json(&first_result);
    assert_eq!(first_payload["summary"]["tracks_total"], 2);
    assert_eq!(first_payload["summary"]["total"], 2);
    assert_eq!(first_payload["summary"]["enriched"], 0);
    assert_eq!(first_payload["summary"]["cached"], 2);
    assert_eq!(first_payload["summary"]["skipped"], 0);
    assert_eq!(first_payload["summary"]["failed"], 0);
    assert_cache_write_summary(&first_payload, 0, 0, 0);
    assert_eq!(first_payload["page"]["matched_tracks"], 2);
    assert_eq!(first_payload["page"]["examined_tracks"], 2);
    assert_eq!(first_payload["page"]["selected_tracks"], 0);
    assert_eq!(first_payload["page"]["fully_cached_skipped"], 2);
    assert_eq!(
        first_payload["page"]["next_offset"],
        serde_json::Value::Null
    );
    assert_eq!(first_payload["page"]["has_more"], false);
    assert_eq!(
        first_payload["failures"]
            .as_array()
            .expect("failures should be an array")
            .len(),
        0
    );

    let second_result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "cached-track-1".to_string(),
                "cached-track-2".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(10),
            offset: None,
            providers: Some(vec![
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
            ]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("second enrich_tracks run should also be fully cached");
    let second_payload = extract_json(&second_result);
    assert_eq!(second_payload["summary"]["tracks_total"], 2);
    assert_eq!(second_payload["summary"]["total"], 2);
    assert_eq!(second_payload["summary"]["enriched"], 0);
    assert_eq!(second_payload["summary"]["cached"], 2);
    assert_eq!(second_payload["summary"]["skipped"], 0);
    assert_eq!(second_payload["summary"]["failed"], 0);
    assert_cache_write_summary(&second_payload, 0, 0, 0);
    assert_eq!(second_payload["page"]["fully_cached_skipped"], 2);

    let store = server
        .cache_store_conn()
        .expect("internal store should be available");
    let entry_one = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("first cache entry should still exist");
    let entry_two = store::get_enrichment(
        &store,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some(&norm_album),
        false,
    )
    .expect("cache read should succeed")
    .expect("second cache entry should still exist");
    assert_eq!(
        entry_one.response_json.as_deref(),
        Some(cached_one.as_str())
    );
    assert_eq!(
        entry_two.response_json.as_deref(),
        Some(cached_two.as_str())
    );
}

#[tokio::test]
async fn enrich_tracks_summary_uses_provider_attempt_totals() {
    let db_conn = create_single_track_test_db("cached-track-1", "/tmp/cached-track-1.flac");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title = crate::domain::metadata::normalize_for_matching("Señorita");
    let norm_album = crate::domain::metadata::normalize_for_matching("Encoded Paths");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title,
        Some(&norm_album),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .expect("discogs cache should seed");
    store::set_enrichment(
        &store_conn,
        "bandcamp",
        &norm_artist,
        &norm_title,
        None,
        Some("exact"),
        Some(r#"{"genre":"Deep House"}"#),
    )
    .expect("bandcamp cache should seed");

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );
    let result = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["cached-track-1".to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            offset: None,
            providers: Some(vec![
                crate::application::enrichment::model::EnrichmentProvider::Discogs,
                crate::application::enrichment::model::EnrichmentProvider::Bandcamp,
            ]),
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: None,
        }))
        .await
        .expect("enrich_tracks should resolve from cache for both providers");
    let payload = extract_json(&result);

    assert_eq!(payload["summary"]["tracks_total"], 1);
    assert_eq!(payload["summary"]["total"], 2);
    assert_eq!(payload["summary"]["cached"], 2);
    assert_eq!(payload["summary"]["enriched"], 0);
    assert_eq!(payload["summary"]["skipped"], 0);
    assert_eq!(payload["summary"]["failed"], 0);
    assert_cache_write_summary(&payload, 0, 0, 0);
    assert_eq!(payload["page"]["matched_tracks"], 1);
    assert_eq!(payload["page"]["fully_cached_skipped"], 1);
}
