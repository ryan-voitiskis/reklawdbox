use super::*;

#[tokio::test]
async fn resolve_track_data_uses_decoded_path_for_audio_cache_lookup() {
    let temp_audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let decoded_path = temp_audio_dir.path().join("Aníbal Track.flac");
    std::fs::write(&decoded_path, b"fake-audio-data")
        .expect("decoded path file should exist for resolve_file_path");
    let metadata = std::fs::metadata(&decoded_path).expect("decoded path metadata should load");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let decoded_path_str = decoded_path.to_string_lossy().to_string();
    let raw_path = decoded_path_str
        .replace("Aníbal", "An%C3%ADbal")
        .replace(' ', "%20");
    assert_ne!(
        raw_path, decoded_path_str,
        "raw path must differ from decoded path for this regression test"
    );
    assert!(
        std::fs::metadata(&decoded_path_str).is_ok(),
        "decoded file path should exist"
    );
    assert!(
        std::fs::metadata(&raw_path).is_err(),
        "raw encoded path should not exist"
    );

    let db_conn = create_single_track_test_db("encoded-track-1", &raw_path);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &decoded_path_str,
        "stratum-dsp",
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"4"}"#,
    )
    .expect("audio cache entry should write with decoded cache key");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: "encoded-track-1".to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let payload = extract_json(&result);

    assert_eq!(
        payload["data_completeness"]["stratum_dsp"], true,
        "decoded path lookup should find stratum cache entry"
    );
    assert!(
        payload["audio_analysis"]["stratum_dsp"].is_object(),
        "audio_analysis.stratum_dsp should be populated from cache"
    );
    assert_eq!(payload["audio_analysis"]["stratum_dsp"]["key"], "Am");
}

#[tokio::test]
async fn resolve_track_data_audio_cache_ignores_existing_file_stale_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("resolve-stale-single.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1000);
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("resolve-stale-single", &audio_path_str);
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &audio_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime + 1,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"18"}"#,
    )
    .expect("stale-mtime stratum cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: "resolve-stale-single".to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let payload = extract_json(&result);

    assert_eq!(
        payload["data_completeness"]["stratum_dsp"], false,
        "current-schema row with stale file mtime must not count"
    );
    assert!(
        payload["audio_analysis"]["stratum_dsp"].is_null(),
        "stale stratum row must be excluded from single-track resolve payload"
    );
}

#[tokio::test]
async fn resolve_tracks_data_audio_cache_ignores_stale_file_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let fresh_path = audio_dir.path().join("resolve-fresh.flac");
    let stale_path = audio_dir.path().join("resolve-stale.flac");
    let (fresh_size, fresh_mtime) = write_test_audio_file(&fresh_path, 1000);
    let (stale_size, stale_mtime) = write_test_audio_file(&stale_path, 1001);
    let fresh_path_str = fresh_path.to_string_lossy().to_string();
    let stale_path_str = stale_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("resolve-fresh", &fresh_path_str);
    insert_test_track(
        &db_conn,
        "resolve-stale",
        "Resolve Stale",
        "g1",
        &stale_path_str,
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"18"}"#,
    )
    .expect("fresh stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":1.5}"#,
    )
    .expect("fresh essentia cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &stale_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        stale_size + 1,
        stale_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":140.0,"key":"Cm","analyzer_version":"18"}"#,
    )
    .expect("stale-size stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &stale_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        stale_size,
        stale_mtime + 1,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":2.5}"#,
    )
    .expect("stale-mtime essentia cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "resolve-fresh".to_string(),
                "resolve-stale".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(2),
            format: None,
        }))
        .await
        .expect("resolve_tracks_data should succeed");
    let payload = extract_json(&result);
    let items = payload
        .as_array()
        .expect("batch resolve should return array");
    let fresh = items
        .iter()
        .find(|item| item["track_id"] == "resolve-fresh")
        .expect("fresh track should be present");
    let stale = items
        .iter()
        .find(|item| item["track_id"] == "resolve-stale")
        .expect("stale track should be present");

    assert_eq!(fresh["data_completeness"]["stratum_dsp"], true);
    assert!(fresh["audio_analysis"]["stratum_dsp"].is_object());
    assert_eq!(fresh["data_completeness"]["essentia"], true);
    assert!(fresh["audio_analysis"]["essentia"].is_object());

    assert_eq!(
        stale["data_completeness"]["stratum_dsp"], false,
        "current-schema stratum row with stale file size must not count"
    );
    assert!(
        stale["audio_analysis"]["stratum_dsp"].is_null(),
        "stale stratum row must be excluded from batch resolve payload"
    );
    assert_eq!(
        stale["data_completeness"]["essentia"], false,
        "current-schema essentia row with stale file mtime must not count"
    );
    assert!(
        stale["audio_analysis"]["essentia"].is_null(),
        "stale essentia row must be excluded from batch resolve payload"
    );
}

#[tokio::test]
#[ignore]
async fn resolve_tracks_data_batch_consistency() {
    let Some((server, _store_dir)) =
        create_real_server_with_temp_store(default_http_client_for_tests())
    else {
        eprintln!("Skipping: backup tarball not found (set REKORDBOX_TEST_BACKUP)");
        return;
    };

    let tracks = sample_real_tracks(&server, 5);
    assert!(
        !tracks.is_empty(),
        "integration test needs tracks from real DB"
    );
    let track_ids: Vec<String> = tracks.iter().map(|t| t.id.clone()).collect();

    let batch_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(track_ids.clone()),
            playlist_id: None,
            max_tracks: Some(track_ids.len() as u32),
            format: None,
        }))
        .await
        .expect("batch resolve should succeed");
    let batch_payload = extract_json(&batch_result);
    let batch_items = batch_payload
        .as_array()
        .expect("batch resolve payload should be an array");
    assert_eq!(
        batch_items.len(),
        track_ids.len(),
        "batch resolve should return one entry per requested track"
    );

    let mut by_track_id: HashMap<String, serde_json::Value> = HashMap::new();
    for item in batch_items {
        let track_id = item
            .get("track_id")
            .and_then(serde_json::Value::as_str)
            .expect("resolved track item should include track_id");
        by_track_id.insert(track_id.to_string(), item.clone());
    }

    for track_id in &track_ids {
        let single_result = server
            .resolve_track_data(Parameters(ResolveTrackDataParams {
                track_id: track_id.clone(),
            }))
            .await
            .expect("single resolve should succeed");
        let single_payload = extract_json(&single_result);
        assert_eq!(
            by_track_id
                .get(track_id)
                .expect("batch output should include every requested track"),
            &single_payload,
            "batch resolve output should match single-track resolve output"
        );
    }
}

#[test]
fn resolve_single_track_rekordbox_only() {
    let track = make_test_track("t1", "Deep House", 126.0, "Am");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let rb = result
        .get("rekordbox")
        .expect("rekordbox section should exist");
    assert_eq!(rb["title"], "Track t1");
    assert_eq!(rb["artist"], "Test Artist");
    assert_eq!(rb["genre"], "Deep House");
    assert_eq!(rb["bpm"], 126.0);
    assert_eq!(rb["key"], "Am");
    assert_eq!(rb["duration_s"], 300);
    assert_eq!(rb["year"], 2023);
    assert_eq!(rb["rating"], 3);
    assert_eq!(rb["label"], "Test Label");

    assert!(
        result["audio_analysis"].is_null(),
        "audio_analysis should be null without cache"
    );
    assert!(
        result["discogs"].is_null(),
        "discogs should be null without cache"
    );
    assert!(
        result["staged_changes"].is_null(),
        "staged_changes should be null without staged"
    );

    let dc = result
        .get("data_completeness")
        .expect("data_completeness should exist");
    assert_eq!(dc["rekordbox"], true);
    assert_eq!(dc["stratum_dsp"], false);
    assert_eq!(dc["essentia"], false);
    assert_eq!(dc["essentia_installed"], false);
    assert_eq!(dc["discogs"], false);

    let gt = result
        .get("genre_taxonomy")
        .expect("genre_taxonomy should exist");
    assert_eq!(gt["current_genre_canonical"], "Deep House");
}

#[test]
fn resolve_single_track_with_staged_changes() {
    let track = make_test_track("t2", "House", 128.0, "Cm");
    let staged = crate::domain::metadata::TrackChange {
        track_id: "t2".to_string(),
        genre: Some("Deep House".to_string()),
        comments: None,
        rating: Some(5),
        color: None,
        label: None,
        year: None,
        album: None,
    };
    let result = resolve_single_track(&track, None, None, None, false, Some(&staged));

    let sc = result
        .get("staged_changes")
        .expect("staged_changes should exist");
    assert!(
        !sc.is_null(),
        "staged_changes should not be null when changes are staged"
    );
    assert_eq!(sc["genre"], "Deep House");
    assert!(sc["comments"].is_null(), "unstaged field should be null");
    assert_eq!(sc["rating"], 5);
    assert!(sc["color"].is_null(), "unstaged field should be null");
    assert!(
        sc.get("year").is_some_and(serde_json::Value::is_null),
        "unstaged year should be present and null"
    );
    assert!(
        sc.get("album").is_some_and(serde_json::Value::is_null),
        "unstaged album should be present and null"
    );
}

#[tokio::test]
async fn resolve_tools_return_all_staged_fields_in_full_format() {
    let track_id = "resolve-all-staged-fields";
    let db_conn = create_single_track_test_db(track_id, "/tmp/resolve-all-staged-fields.flac");
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

    let staged = TrackChange {
        track_id: track_id.to_string(),
        genre: Some("Techno".to_string()),
        comments: Some("staged comments".to_string()),
        rating: Some(5),
        color: Some("Red".to_string()),
        label: Some("Staged Label".to_string()),
        year: Some(1997),
        album: Some("Staged Album".to_string()),
    };
    assert_eq!(server.context.mutation.changes.stage(vec![staged]), (1, 1));

    let expected_staged = serde_json::json!({
        "genre": "Techno",
        "comments": "staged comments",
        "rating": 5,
        "color": "Red",
        "label": "Staged Label",
        "year": 1997,
        "album": "Staged Album",
    });
    let expected_keys: HashSet<&str> = EditableField::ALL
        .iter()
        .map(EditableField::as_str)
        .collect();

    let single_result = server
        .resolve_track_data(Parameters(ResolveTrackDataParams {
            track_id: track_id.to_string(),
        }))
        .await
        .expect("resolve_track_data should succeed");
    let single_payload = extract_json(&single_result);
    let single_staged = single_payload
        .get("staged_changes")
        .expect("resolve_track_data should include staged_changes");
    assert_eq!(single_staged, &expected_staged);
    let single_keys: HashSet<&str> = single_staged
        .as_object()
        .expect("staged_changes should be an object")
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(single_keys, expected_keys);

    let batch_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![track_id.to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            format: Some(ResolveFormat::Full),
        }))
        .await
        .expect("full resolve_tracks_data should succeed");
    let batch_payload = extract_json(&batch_result);
    let batch_item = batch_payload
        .as_array()
        .and_then(|items| items.first())
        .expect("full resolve_tracks_data should return the requested track");
    let batch_staged = batch_item
        .get("staged_changes")
        .expect("full resolve_tracks_data should include staged_changes");
    assert_eq!(batch_staged, &expected_staged);
    assert_eq!(batch_staged, single_staged);

    let compact_result = server
        .resolve_tracks_data(Parameters(ResolveTracksDataParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![track_id.to_string()]),
            playlist_id: None,
            max_tracks: Some(1),
            format: Some(ResolveFormat::Classification),
        }))
        .await
        .expect("classification resolve_tracks_data should succeed");
    let compact_payload = extract_json(&compact_result);
    let compact_item = compact_payload
        .as_array()
        .and_then(|items| items.first())
        .expect("classification resolve_tracks_data should return the requested track");
    assert_eq!(
        compact_item.get("track_id"),
        Some(&serde_json::json!(track_id))
    );
    assert!(compact_item.get("artist").is_some());
    assert!(compact_item.get("audio").is_some());
    assert!(compact_item.get("rekordbox").is_none());
    assert!(compact_item.get("staged_changes").is_none());
}

#[test]
fn resolve_single_track_taxonomy_mappings() {
    let track = make_test_track("t3", "Hip-Hop", 130.0, "Fm");

    let discogs_json = serde_json::json!({
        "title": "Some Release",
        "year": "2020",
        "label": "Some Label",
        "genres": ["Electronic"],
        "styles": ["Deep House", "Garage House", "Some Unknown Style"],
        "fuzzy_match": false,
    });
    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t3".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("exact".to_string()),
        response_json: Some(serde_json::to_string(&discogs_json).unwrap()),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, Some(&discogs_cache), None, None, false, None);

    let dc = &result["data_completeness"];
    assert_eq!(dc["discogs"], true);
    assert_eq!(dc["stratum_dsp"], false);

    let gt = &result["genre_taxonomy"];
    assert_eq!(gt["current_genre_canonical"], "Hip Hop");

    let dsm = gt["discogs_style_mappings"]
        .as_array()
        .expect("should be array");
    assert_eq!(dsm.len(), 3);

    let dh = dsm
        .iter()
        .find(|m| m["style"] == "Deep House")
        .expect("Deep House mapping");
    assert_eq!(dh["mapping_type"], "exact");
    assert_eq!(dh["maps_to"], "Deep House");

    let gh = dsm
        .iter()
        .find(|m| m["style"] == "Garage House")
        .expect("Garage House mapping");
    assert_eq!(gh["mapping_type"], "alias");
    assert_eq!(gh["maps_to"], "House");

    let unknown = dsm
        .iter()
        .find(|m| m["style"] == "Some Unknown Style")
        .expect("unknown mapping");
    assert_eq!(unknown["mapping_type"], "unknown");
    assert!(unknown["maps_to"].is_null());

    assert!(
        result["discogs"].is_object(),
        "discogs should be parsed object"
    );
}

#[test]
fn resolve_single_track_empty_genre_is_null() {
    let track = make_test_track("t4", "", 0.0, "");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "empty genre should map to null canonical"
    );
}

#[test]
fn resolve_single_track_unknown_genre_maps_to_null() {
    let track = make_test_track("t5", "Polka", 120.0, "C");
    let result = resolve_single_track(&track, None, None, None, false, None);

    let gt = &result["genre_taxonomy"];
    assert!(
        gt["current_genre_canonical"].is_null(),
        "unknown genre 'Polka' should map to null"
    );
}

#[test]
fn resolve_single_track_with_stratum_agreement() {
    let track = make_test_track("t6", "Techno", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 128.5,
        "key": "Am",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        input_fingerprint: "hmm:v1".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, Some(&stratum_cache), None, false, None);

    let aa = result
        .get("audio_analysis")
        .expect("audio_analysis should exist");
    assert!(
        !aa.is_null(),
        "audio_analysis should not be null with stratum cache"
    );
    assert_eq!(
        aa["bpm_agreement"], true,
        "BPM 128.0 vs 128.5 should agree (within 2.0)"
    );
    assert_eq!(aa["key_agreement"], true, "Key Am vs Am should agree");
    assert!(
        aa["stratum_dsp"].is_object(),
        "stratum_dsp should be the parsed features"
    );
    assert!(
        aa["essentia"].is_null(),
        "essentia should be null when not cached"
    );

    let dc = &result["data_completeness"];
    assert_eq!(dc["stratum_dsp"], true);
}

#[test]
fn resolve_single_track_with_essentia_cache() {
    let track = make_test_track("t6b", "Techno", 128.0, "Am");
    let essentia_json = serde_json::json!({
        "danceability": 0.82,
        "loudness_integrated": -8.4,
        "rhythm_regularity": 0.91,
        "analyzer_version": "2.1b6.dev1389"
    });
    let essentia_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "essentia".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "2.1b6.dev1389".to_string(),
        input_fingerprint: String::new(),
        features_json: serde_json::to_string(&essentia_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, None, Some(&essentia_cache), true, None);

    let aa = &result["audio_analysis"];
    assert!(
        aa.is_object(),
        "audio_analysis should be populated when essentia cache exists"
    );
    assert!(
        aa["stratum_dsp"].is_null(),
        "stratum_dsp should remain null when not cached"
    );
    assert!(
        aa["essentia"].is_object(),
        "essentia should expose cached analysis JSON"
    );
    assert_eq!(aa["essentia"]["danceability"], 0.82);

    let dc = &result["data_completeness"];
    assert_eq!(dc["essentia"], true);
    assert_eq!(dc["essentia_installed"], true);
}

#[test]
fn resolve_single_track_stratum_disagreement() {
    let track = make_test_track("t7", "House", 128.0, "Am");

    let stratum_json = serde_json::json!({
        "bpm": 64.0,
        "key": "Cm",
        "analyzer_version": "0.1.0",
    });
    let stratum_cache = store::CachedAudioAnalysis {
        file_path: "/music/test.flac".to_string(),
        analyzer: "stratum-dsp".to_string(),
        file_size: 12345,
        file_mtime: 1700000000,
        analysis_version: "0.1.0".to_string(),
        input_fingerprint: "hmm:v1".to_string(),
        features_json: serde_json::to_string(&stratum_json).unwrap(),
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, None, Some(&stratum_cache), None, false, None);

    let aa = &result["audio_analysis"];
    assert_eq!(
        aa["bpm_agreement"], false,
        "BPM 128.0 vs 64.0 should disagree"
    );
    assert_eq!(aa["key_agreement"], false, "Key Am vs Cm should disagree");
}

#[test]
fn resolve_single_track_enrichment_no_match_returns_null() {
    let track = make_test_track("t8", "House", 126.0, "Am");

    let discogs_cache = store::EnrichmentCacheEntry {
        provider: "discogs".to_string(),
        query_artist: "test artist".to_string(),
        query_title: "track t8".to_string(),
        query_album: "test album".to_string(),
        match_quality: Some("none".to_string()),
        response_json: None,
        created_at: "2024-01-01".to_string(),
    };

    let result = resolve_single_track(&track, Some(&discogs_cache), None, None, false, None);

    assert!(
        result["discogs"].is_null(),
        "discogs with no response_json should be null"
    );
    assert_eq!(
        result["data_completeness"]["discogs"], true,
        "cache entry exists so completeness is true"
    );
}
