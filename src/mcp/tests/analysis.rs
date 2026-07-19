use crate::adapters::audio::{
    probe_essentia_python_from_sources, validate_essentia_python_with_timeout,
};
use crate::application::analysis::identity::{
    audio_cache_identities_with_fingerprint_loader,
    audio_cache_identity_with_stratum_input_fingerprint, check_analysis_cache,
    check_analysis_cache_for_identity,
};
use crate::application::planning::build_track_profiles;
use crate::mcp::analysis::{
    AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, CacheCoverageParams, resolve_file_path,
};
use crate::mcp::classification::{
    CalibrateAudioProfilesParams, ClassifyFormat, ClassifyTracksParams,
};
use crate::mcp::library::SearchFilterParams;
use crate::mcp::server::ReklawdboxServer;
use std::collections::HashSet;
use std::time::Duration;

use rmcp::handler::server::wrapper::Parameters;
use rusqlite::params;
use tempfile::TempDir;

use crate::adapters::{rekordbox as db, state as store};

use super::common::{
    create_real_server_with_temp_store, create_server_with_connections,
    create_server_with_store_path, create_single_track_test_db, default_http_client_for_tests,
    extract_json, insert_test_track, make_test_track, sample_real_tracks, set_test_audio_analysis,
    valid_test_essentia_payload, write_test_audio_file,
};

fn create_fully_current_audio_batch_server(
    essentia_available: bool,
) -> (ReklawdboxServer, TempDir, TempDir) {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let first_path = audio_dir.path().join("current-one.flac");
    let second_path = audio_dir.path().join("current-two.flac");
    let (first_size, first_mtime) = write_test_audio_file(&first_path, 64);
    let (second_size, second_mtime) = write_test_audio_file(&second_path, 96);
    let first_path = first_path.to_string_lossy().to_string();
    let second_path = second_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("current-one", &first_path);
    insert_test_track(&db_conn, "current-two", "Current Two", "g1", &second_path);

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_path_str = store_path
        .to_str()
        .expect("temp store path should be UTF-8")
        .to_string();
    let store_conn = store::open(&store_path_str).expect("temp internal store should open");
    for (path, size, mtime) in [
        (&first_path, first_size, first_mtime),
        (&second_path, second_size, second_mtime),
    ] {
        set_test_audio_analysis(
            &store_conn,
            path,
            crate::adapters::audio::ANALYZER_STRATUM,
            size,
            mtime,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            r#"{"bpm":128.0}"#,
        )
        .expect("current Stratum cache should seed");
        if essentia_available {
            set_test_audio_analysis(
                &store_conn,
                path,
                crate::adapters::audio::ANALYZER_ESSENTIA,
                size,
                mtime,
                crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
                r#"{"energy":0.5}"#,
            )
            .expect("current Essentia cache should seed");
        }
    }

    let server = create_server_with_store_path(
        db_conn,
        store_conn,
        default_http_client_for_tests(),
        Some(store_path_str),
    );
    server
        .context
        .analysis
        .essentia_python
        .set(essentia_available.then(|| "/unused/test-essentia-python".to_string()))
        .expect("Essentia availability should initialize exactly once");
    (server, audio_dir, store_dir)
}

#[test]
fn cache_coverage_public_schema() {
    let router = ReklawdboxServer::build_tool_router();
    let tool_schema = |tool_name: &str| {
        let tool = router
            .get(tool_name)
            .unwrap_or_else(|| panic!("{tool_name} should be registered"));
        serde_json::to_value(tool).expect("tool metadata should serialize")
    };
    let resolve_schema = tool_schema("resolve_tracks_data");
    let coverage_schema = tool_schema("cache_coverage");
    let properties = |tool_name: &str, schema: &serde_json::Value| {
        schema["inputSchema"]["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{tool_name} should expose input properties"))
            .clone()
    };

    let resolve_properties = properties("resolve_tracks_data", &resolve_schema);
    let coverage_properties = properties("cache_coverage", &coverage_schema);

    let resolve_names = resolve_properties.keys().cloned().collect::<HashSet<_>>();
    let coverage_names = coverage_properties.keys().cloned().collect::<HashSet<_>>();

    assert_eq!(resolve_names.len(), 20);
    assert_eq!(coverage_names.len(), 19);
    for (name, schema) in [
        ("resolve_tracks_data", &resolve_schema),
        ("cache_coverage", &coverage_schema),
    ] {
        assert!(
            schema["inputSchema"]["required"]
                .as_array()
                .is_none_or(Vec::is_empty),
            "{name} selectors should all remain optional"
        );
    }

    assert!(
        !coverage_names.contains("format"),
        "cache_coverage must not advertise the ignored format parameter"
    );
    assert_eq!(
        resolve_names
            .difference(&coverage_names)
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from(["format".to_owned()]),
        "resolve_tracks_data should differ from cache_coverage only by format"
    );
    assert!(
        coverage_names.is_subset(&resolve_names),
        "cache_coverage should retain every shared selector and filter"
    );
    for (name, mut coverage_schema) in coverage_properties {
        let mut resolve_schema = resolve_properties
            .get(&name)
            .cloned()
            .unwrap_or_else(|| panic!("resolve_tracks_data should retain shared property {name}"));
        coverage_schema
            .as_object_mut()
            .expect("property schemas should be objects")
            .remove("description");
        resolve_schema
            .as_object_mut()
            .expect("property schemas should be objects")
            .remove("description");
        assert_eq!(
            resolve_schema, coverage_schema,
            "shared property {name} should have the same public type contract"
        );
    }
    assert!(
        coverage_schema["inputSchema"]["properties"]["track_ids"]["description"]
            .as_str()
            .is_some_and(
                |description| description.contains("check") && !description.contains("resolve")
            )
    );
    assert!(
        coverage_schema["inputSchema"]["properties"]["max_tracks"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("unbounded"))
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_prefers_env_override_when_valid() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python");
    std::fs::write(&fake_python, "#!/bin/sh\necho '{\"python\":\"3.14.6\",\"implementation\":\"cpython\",\"essentia\":\"2.1b6.dev1438\",\"essentia_module\":\"2.1-beta6-dev\",\"numpy\":\"2.5.1\",\"pyyaml\":\"6.0.3\",\"six\":\"1.17.0\"}'\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let resolved = probe_essentia_python_from_sources(
        fake_python.to_str(),
        Some(dir.path().join("missing-default-python")),
    );

    assert_eq!(
        resolved.as_deref(),
        fake_python.to_str(),
        "valid env override should win over default candidate"
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_fails_when_no_version_string() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python-empty");
    std::fs::write(&fake_python, "#!/bin/sh\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let resolved =
        probe_essentia_python_from_sources(fake_python.to_str(), Some(dir.path().join("other")));
    assert!(
        resolved.is_none(),
        "probe should reject candidates that do not emit version output"
    );
}

#[test]
#[cfg(unix)]
fn probe_essentia_python_returns_error_on_timeout() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python-slow");
    std::fs::write(&fake_python, "#!/bin/sh\nexec sleep 2\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("fake python metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python script should be executable");

    let start = std::time::Instant::now();
    let is_valid = validate_essentia_python_with_timeout(
        fake_python.to_str().unwrap(),
        Duration::from_millis(100),
    );
    assert!(
        !is_valid,
        "slow candidate should be rejected when probe timeout elapses"
    );
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "probe timeout should fail fast"
    );
}

#[tokio::test]
async fn analyze_track_audio_reports_essentia_unavailable_when_probe_is_none() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("cached-track.flac");
    std::fs::write(&audio_path, b"fake-audio-data").expect("temp audio file should be created");
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let metadata = std::fs::metadata(&audio_path).expect("temp audio metadata should load");
    let file_size = metadata.len() as i64;
    let file_mtime = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);

    let db_conn = create_single_track_test_db("essentia-missing-1", &audio_path_str);
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
        "stratum-dsp",
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"stratum-dsp-1.0.0"}"#,
    )
    .expect("stratum cache should be seeded");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server
        .context
        .analysis
        .essentia_python
        .set(None)
        .expect("essentia probe state should be seeded once");

    let result = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: "essentia-missing-1".to_string(),
            skip_cached: Some(true),
        }))
        .await
        .expect("analyze_track_audio should succeed with cached stratum data");
    let payload = extract_json(&result);

    assert_eq!(payload["essentia_available"], false);
    assert!(
        payload["essentia"].is_null(),
        "essentia payload should be null when probe is unavailable"
    );
    assert_eq!(
        payload["stratum_cache_hit"], true,
        "stratum cache should still be used when Essentia is unavailable"
    );
    assert!(
        payload["stratum_dsp"].is_object(),
        "stratum_dsp should still be returned"
    );
    let hint = payload["essentia_setup_hint"]
        .as_str()
        .expect("essentia_setup_hint should be present when unavailable");
    assert!(
        hint.contains("setup_essentia"),
        "hint should mention setup_essentia tool"
    );
    assert!(
        hint.contains("CRATE_DIG_ESSENTIA_PYTHON"),
        "hint should mention the env var that was checked"
    );
}

#[test]
fn analyze_track_audio_cache_miss_when_file_unstatable() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let missing_path = audio_dir.path().join("missing-track.flac");
    let missing_path_str = missing_path.to_string_lossy().to_string();
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");

    set_test_audio_analysis(
        &store_conn,
        &missing_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        123,
        456,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0}"#,
    )
    .expect("stale stratum cache should seed");

    let cached = check_analysis_cache(
        &store_conn,
        &missing_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
    )
    .expect("cache read should succeed");
    assert!(cached.is_none(), "unstatable files must be cache misses");
}

#[test]
fn audio_cache_grid_fingerprint_mcp_reads_are_analyzer_specific() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("identity.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1000);
    let audio_path = audio_path.to_string_lossy().to_string();
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    store::set_audio_analysis_with_fingerprint(
        &store_conn,
        &audio_path,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        "grid:v1:same",
        r#"{"bpm":128.0}"#,
    )
    .unwrap();
    store::set_audio_analysis_with_fingerprint(
        &store_conn,
        &audio_path,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        file_size,
        file_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        "",
        r#"{"danceability":0.8}"#,
    )
    .unwrap();
    let same =
        audio_cache_identity_with_stratum_input_fingerprint(&audio_path, "grid:v1:same").unwrap();
    let changed =
        audio_cache_identity_with_stratum_input_fingerprint(&audio_path, "grid:v1:changed")
            .unwrap();

    assert!(
        check_analysis_cache_for_identity(
            &store_conn,
            &same,
            crate::adapters::audio::ANALYZER_STRATUM,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        )
        .unwrap()
        .is_some()
    );
    assert!(
        check_analysis_cache_for_identity(
            &store_conn,
            &changed,
            crate::adapters::audio::ANALYZER_STRATUM,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        )
        .unwrap()
        .is_none()
    );
    assert!(
        check_analysis_cache_for_identity(
            &store_conn,
            &changed,
            crate::adapters::audio::ANALYZER_ESSENTIA,
            crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        )
        .unwrap()
        .is_some(),
        "Rekordbox grid edits must not invalidate Essentia"
    );
    store::set_audio_analysis_with_fingerprint(
        &store_conn,
        &audio_path,
        crate::adapters::audio::ANALYZER_STRATUM,
        file_size,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        "grid:v1:changed",
        r#"{"bpm":129.0}"#,
    )
    .unwrap();
    assert!(
        check_analysis_cache_for_identity(
            &store_conn,
            &changed,
            crate::adapters::audio::ANALYZER_STRATUM,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        )
        .unwrap()
        .is_some(),
        "writing the newly analyzed fingerprint must make Stratum fresh"
    );
}

#[test]
fn audio_cache_grid_fingerprint_batch_deduplicates_resolved_paths_and_keeps_mixed_sources() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let grid_path = audio_dir.path().join("grid track.flac");
    let hmm_path = audio_dir.path().join("hmm.flac");
    write_test_audio_file(&grid_path, 1000);
    write_test_audio_file(&hmm_path, 1001);
    let grid_path = grid_path.to_string_lossy().to_string();
    let encoded_grid_path = grid_path.replace(' ', "%20");
    let hmm_path = hmm_path.to_string_lossy().to_string();
    let mut lookup_keys = Vec::new();

    let identities = audio_cache_identities_with_fingerprint_loader(
        [
            grid_path.as_str(),
            encoded_grid_path.as_str(),
            hmm_path.as_str(),
        ],
        |cache_key| {
            lookup_keys.push(cache_key.to_string());
            if cache_key.ends_with("hmm.flac") {
                "hmm:v1".to_string()
            } else {
                "grid:v1:synthetic".to_string()
            }
        },
    );
    let identities: Vec<_> = identities.into_iter().map(Option::unwrap).collect();

    assert_eq!(lookup_keys, vec![grid_path.clone(), hmm_path.clone()]);
    assert_eq!(identities[0].cache_key, grid_path);
    assert_eq!(identities[1].cache_key, identities[0].cache_key);
    assert_eq!(
        identities[0].stratum_input_fingerprint,
        identities[1].stratum_input_fingerprint
    );
    assert_eq!(
        identities[2].stratum_input_fingerprint.as_deref(),
        Some("hmm:v1")
    );
}

#[test]
fn track_profile_batch_preserves_order_and_uses_fresh_cached_features() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store = crate::adapters::state::open(store_path.to_str().unwrap()).unwrap();
    let mut tracks = Vec::new();

    for (index, fallback_bpm) in [(0, 0.0), (1, 126.0)] {
        let audio_path = audio_dir.path().join(format!("profile-{index}.wav"));
        let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1_000 + index);
        let mut track =
            make_test_track(&format!("profile-{index}"), "Deep House", fallback_bpm, "");
        track.file_path = audio_path.to_string_lossy().to_string();
        set_test_audio_analysis(
            &store,
            &track.file_path,
            crate::adapters::audio::ANALYZER_STRATUM,
            file_size,
            file_mtime,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            &format!(
                r#"{{"bpm":{},"key_camelot":"{}A"}}"#,
                124 + index,
                8 + index
            ),
        )
        .unwrap();
        set_test_audio_analysis(
            &store,
            &track.file_path,
            crate::adapters::audio::ANALYZER_ESSENTIA,
            file_size,
            file_mtime,
            crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
            r#"{"danceability":0.8,"spectral_centroid_mean":1200.0}"#,
        )
        .unwrap();
        tracks.push(track);
    }

    let profiles = build_track_profiles(tracks, &store).unwrap();

    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].track.id, "profile-0");
    assert_eq!(profiles[1].track.id, "profile-1");
    assert_eq!(profiles[0].bpm, 124.0, "missing Rekordbox BPM uses Stratum");
    assert_eq!(
        profiles[1].bpm, 126.0,
        "plausible Rekordbox BPM remains authoritative"
    );
    assert_eq!(profiles[0].key_display, "8A");
    assert_eq!(profiles[1].key_display, "9A");
    assert_eq!(profiles[0].brightness, Some(1200.0));
    assert!(profiles.iter().all(|profile| profile.energy > 0.0));
}

#[tokio::test]
async fn analyze_track_audio_audio_cache_ignores_existing_file_stale_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let audio_path = audio_dir.path().join("stale-existing-track.flac");
    let (file_size, file_mtime) = write_test_audio_file(&audio_path, 1000);
    let audio_path_str = audio_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("stale-existing-1", &audio_path_str);
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
        file_size + 1,
        file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":128.0,"key":"Am","analyzer_version":"stratum-dsp-1.0.0"}"#,
    )
    .expect("stale stratum cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server
        .context
        .analysis
        .essentia_python
        .set(None)
        .expect("essentia probe state should be seeded once");

    server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: "stale-existing-1".to_string(),
            skip_cached: Some(true),
        }))
        .await
        .expect_err("stale current-schema cache must not bypass decode for an existing file");
}

#[tokio::test]
#[ignore]
async fn private_rekordbox_analyze_track_audio_essentia_cache_round_trip() {
    let server_fixture = create_real_server_with_temp_store(default_http_client_for_tests())
        .expect("private Rekordbox fixture should be configured and readable");
    let server = server_fixture.server();

    if server.essentia_python_path().is_none() {
        eprintln!("Skipping: Essentia Python not available");
        return;
    }

    let track = sample_real_tracks(server, 40)
        .into_iter()
        .filter(|t| (120.0..=145.0).contains(&t.bpm))
        .find(|t| resolve_file_path(&t.file_path).is_ok())
        .expect("integration test needs at least one track with accessible audio file");

    let first = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: track.id.clone(),
            skip_cached: Some(false),
        }))
        .await
        .unwrap_or_else(|_| panic!("initial private fixture analysis should succeed"));
    let first_payload = extract_json(&first);
    assert!(
        first_payload["essentia_available"] == true,
        "Essentia should be available"
    );
    assert!(
        first_payload["essentia"].is_object(),
        "real Essentia run should produce feature JSON"
    );
    assert!(
        first_payload["essentia_cache_hit"] == false,
        "initial analysis should miss the Essentia cache"
    );
    let onset_rate = first_payload["essentia"]["onset_rate"]
        .as_f64()
        .expect("onset_rate should be present in Essentia output");
    let danceability = first_payload["essentia"]["danceability"]
        .as_f64()
        .expect("danceability should be present in Essentia output");
    let loudness_integrated = first_payload["essentia"]["loudness_integrated"]
        .as_f64()
        .expect("loudness_integrated should be present in Essentia output");
    assert!(onset_rate > 1.0, "onset_rate should be rate-like");
    assert!(
        (0.0..=3.5).contains(&danceability),
        "danceability should stay in the plausible Essentia range"
    );
    assert!(
        (-30.0..=0.0).contains(&loudness_integrated),
        "loudness_integrated should stay in the plausible LUFS range"
    );

    let second = server
        .analyze_track_audio(Parameters(AnalyzeTrackAudioParams {
            track_id: track.id,
            skip_cached: Some(true),
        }))
        .await
        .unwrap_or_else(|_| panic!("cached private fixture analysis should succeed"));
    let second_payload = extract_json(&second);
    assert!(
        second_payload["essentia_available"] == true,
        "Essentia should remain available"
    );
    assert!(
        second_payload["stratum_cache_hit"] == true,
        "cached analysis should hit the Stratum cache"
    );
    assert!(
        second_payload["essentia_cache_hit"] == true,
        "cached analysis should hit the Essentia cache"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn setup_essentia_returns_already_installed_when_override_is_valid() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir should create");
    let fake_python = dir.path().join("fake-python");
    std::fs::write(&fake_python, "#!/bin/sh\necho '{\"python\":\"3.14.6\",\"implementation\":\"cpython\",\"essentia\":\"2.1b6.dev1438\",\"essentia_module\":\"2.1-beta6-dev\",\"numpy\":\"2.5.1\",\"pyyaml\":\"6.0.3\",\"six\":\"1.17.0\"}'\nexit 0\n")
        .expect("fake python script should be written");
    let mut perms = std::fs::metadata(&fake_python)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&fake_python, perms).expect("fake python should be executable");
    let fake_path = fake_python.to_string_lossy().to_string();

    let server = ReklawdboxServer::new(None);
    {
        let mut guard = server
            .context
            .analysis
            .essentia_python_override
            .lock()
            .unwrap();
        *guard = Some(fake_path.clone());
    }

    let result = server
        .setup_essentia()
        .await
        .expect("setup_essentia should succeed when already installed");
    let payload = extract_json(&result);

    assert_eq!(payload["status"], "already_installed");
    assert_eq!(payload["python_path"], fake_path.as_str());
    assert_eq!(payload["python_version"], "3.14.6");
    assert_eq!(payload["essentia_version"], "2.1b6.dev1438");
    assert_eq!(payload["essentia_module_version"], "2.1-beta6-dev");
    assert_eq!(payload["numpy_version"], "2.5.1");
    assert_eq!(payload["pyyaml_version"], "6.0.3");
    assert_eq!(payload["six_version"], "1.17.0");
    assert_eq!(
        payload["analyzer_contract"],
        "essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14"
    );
}

#[test]
fn setup_essentia_installed_and_reused_shapes_share_runtime_manifest() {
    use crate::adapters::audio::EssentiaRuntime;
    use crate::application::analysis::setup::{EssentiaSetupResult, EssentiaSetupStatus};

    let runtime = EssentiaRuntime {
        python_path: "/managed/essentia-venv/bin/python".into(),
        python_version: "3.14.6".into(),
        essentia_version: "2.1b6.dev1438".into(),
        essentia_module_version: "2.1-beta6-dev".into(),
        numpy_version: "2.5.1".into(),
        pyyaml_version: "6.0.3".into(),
        six_version: "1.17.0".into(),
        analyzer_contract:
            "essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14".into(),
    };
    let installed = crate::mcp::analysis::setup_essentia_payload(&EssentiaSetupResult {
        status: EssentiaSetupStatus::Installed,
        runtime: runtime.clone(),
        python_bin_used: Some("python3.14".into()),
    });
    let reused = crate::mcp::analysis::setup_essentia_payload(&EssentiaSetupResult {
        status: EssentiaSetupStatus::AlreadyInstalled,
        runtime,
        python_bin_used: None,
    });

    assert_eq!(installed["status"], "installed");
    assert_eq!(reused["status"], "already_installed");
    assert_eq!(installed["python_version"], "3.14.6");
    assert_eq!(installed["essentia_module_version"], "2.1-beta6-dev");
    assert_eq!(installed["six_version"], "1.17.0");
    assert_eq!(installed["python_bin_used"], "python3.14");
    assert_eq!(installed["venv_dir"], "/managed/essentia-venv");
    assert!(reused.get("python_bin_used").is_none());
    assert!(reused.get("venv_dir").is_none());
}

#[tokio::test]
async fn setup_essentia_failure_invalidates_stale_memoized_runtime() {
    use crate::adapters::audio::{EssentiaSetupError, EssentiaSetupErrorKind};
    use crate::application::analysis::setup::EssentiaSetupResult;

    let server = ReklawdboxServer::new(None);
    server
        .context
        .analysis
        .essentia_python
        .set(Some("/stale/essentia/bin/python".into()))
        .unwrap();
    assert!(server.essentia_python_path().is_some());

    let error = crate::mcp::analysis::handle_setup_essentia_with(&server, |_| {
        Err::<EssentiaSetupResult, _>(EssentiaSetupError::new(
            EssentiaSetupErrorKind::PipFailure,
            "forced offline install failure",
        ))
    })
    .await
    .expect_err("forced setup failure should surface");

    assert!(error.message.contains("forced offline install failure"));
    assert_eq!(error.data.as_ref().unwrap()["kind"], "pip_failure");
    assert!(server.essentia_python_path().is_none());
    server
        .activate_essentia_python_path("/fresh/essentia-venv/bin/python".into())
        .unwrap();
    assert_eq!(
        server.essentia_python_path().as_deref(),
        Some("/fresh/essentia-venv/bin/python")
    );

    let override_server = ReklawdboxServer::new(None);
    *override_server
        .context
        .analysis
        .essentia_python_override
        .lock()
        .unwrap() = Some("/stale/override/bin/python".into());
    crate::mcp::analysis::handle_setup_essentia_with(&override_server, |_| {
        Err::<EssentiaSetupResult, _>(EssentiaSetupError::new(
            EssentiaSetupErrorKind::ManifestMismatch,
            "forced override install failure",
        ))
    })
    .await
    .expect_err("forced override setup failure should surface");
    assert!(override_server.essentia_python_path().is_none());
}

#[tokio::test]
async fn setup_essentia_concurrent_mcp_calls_remain_serialized() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::adapters::audio::EssentiaRuntime;
    use crate::application::analysis::setup::{EssentiaSetupResult, EssentiaSetupStatus};

    let server = ReklawdboxServer::new(None);
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let setup = |active: Arc<AtomicUsize>, max_active: Arc<AtomicUsize>| {
        move |_existing: Option<String>| {
            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(30));
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(EssentiaSetupResult {
                status: EssentiaSetupStatus::AlreadyInstalled,
                runtime: EssentiaRuntime {
                    python_path: "/managed/essentia-venv/bin/python".into(),
                    python_version: "3.14.6".into(),
                    essentia_version: "2.1b6.dev1438".into(),
                    essentia_module_version: "2.1-beta6-dev".into(),
                    numpy_version: "2.5.1".into(),
                    pyyaml_version: "6.0.3".into(),
                    six_version: "1.17.0".into(),
                    analyzer_contract:
                        "essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14"
                            .into(),
                },
                python_bin_used: None,
            })
        }
    };

    let first = crate::mcp::analysis::handle_setup_essentia_with(
        &server,
        setup(active.clone(), max_active.clone()),
    );
    let second = crate::mcp::analysis::handle_setup_essentia_with(
        &server,
        setup(active.clone(), max_active.clone()),
    );
    let (first, second) = tokio::join!(first, second);

    first.unwrap();
    second.unwrap();
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
}

#[tokio::test]
#[cfg(unix)]
async fn setup_essentia_cancelled_future_leaves_blocking_transaction_sound() {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd as _;
    use std::sync::Arc;

    use crate::adapters::audio::{
        ESSENTIA_CONTRACT_ID, EssentiaRuntime, EssentiaSetupError, EssentiaSetupErrorKind,
    };
    use crate::application::analysis::setup::{EssentiaSetupResult, EssentiaSetupStatus};

    fn try_lock(file: &std::fs::File) -> std::io::Result<()> {
        // SAFETY: `file` stays open for the call and flock only touches its
        // owned descriptor.
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    struct GenerationFixture(std::path::PathBuf);

    impl Drop for GenerationFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let root = tempfile::tempdir().expect("temp root should create");
    let lock_path = root.path().join("essentia-venv.lock");
    let generation = root
        .path()
        .join("essentia-venv.generations/runtime-cancelled");
    let server = Arc::new(ReklawdboxServer::new(None));
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let first_server = Arc::clone(&server);
    let first_lock_path = lock_path.clone();
    let first_generation = generation.clone();
    let first = tokio::spawn(async move {
        crate::mcp::analysis::handle_setup_essentia_with(&first_server, move |_| {
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&first_lock_path)
                .expect("fixture lock should open");
            // SAFETY: the descriptor remains owned until the fake transaction
            // finishes, including after the outer future is cancelled.
            assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
            std::fs::create_dir_all(&first_generation).expect("fixture generation should create");
            let generation_guard = GenerationFixture(first_generation.clone());
            std::fs::write(first_generation.join("incomplete"), b"fixture")
                .expect("fixture marker should write");
            started_tx
                .send(())
                .expect("test should still await transaction start");
            release_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("test should release fake transaction");
            drop(generation_guard);
            drop(lock);
            Err::<EssentiaSetupResult, _>(EssentiaSetupError::new(
                EssentiaSetupErrorKind::ImportFailure,
                "cancelled fixture completed",
            ))
        })
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("blocking transaction should start before deadline")
        .expect("blocking transaction start sender should remain live");
    first.abort();
    let cancelled = first
        .await
        .expect_err("outer setup task should be cancelled");
    assert!(cancelled.is_cancelled());

    let competing_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .expect("competing fixture lock should open");
    let lock_error = try_lock(&competing_lock).expect_err(
        "blocking transaction must retain its interprocess lock after outer cancellation",
    );
    assert_eq!(lock_error.kind(), std::io::ErrorKind::WouldBlock);
    assert!(generation.join("incomplete").is_file());

    release_tx
        .send(())
        .expect("fake transaction should receive release");
    tokio::time::timeout(Duration::from_secs(2), async {
        while generation.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("fake transaction should clean before deadline");
    try_lock(&competing_lock).expect("lock should be reacquirable after transaction cleanup");

    let result = crate::mcp::analysis::handle_setup_essentia_with(&server, |_| {
        Ok(EssentiaSetupResult {
            status: EssentiaSetupStatus::AlreadyInstalled,
            runtime: EssentiaRuntime {
                python_path: "/managed/essentia-venv/bin/python".into(),
                python_version: "3.14.6".into(),
                essentia_version: "2.1b6.dev1438".into(),
                essentia_module_version: "2.1-beta6-dev".into(),
                numpy_version: "2.5.1".into(),
                pyyaml_version: "6.0.3".into(),
                six_version: "1.17.0".into(),
                analyzer_contract: ESSENTIA_CONTRACT_ID.into(),
            },
            python_bin_used: None,
        })
    })
    .await
    .expect("subsequent setup should remain sound");
    let payload = extract_json(&result);
    assert_eq!(payload["status"], "already_installed");
    assert_eq!(
        server.essentia_python_path().as_deref(),
        Some("/managed/essentia-venv/bin/python")
    );
}

#[tokio::test]
async fn essentia_python_override_takes_precedence() {
    let server = ReklawdboxServer::new(None);
    server
        .context
        .analysis
        .essentia_python
        .set(None)
        .expect("essentia probe should be seeded once");
    assert!(
        server.essentia_python_path().is_none(),
        "should be None before override"
    );

    {
        let mut guard = server
            .context
            .analysis
            .essentia_python_override
            .lock()
            .unwrap();
        *guard = Some("/override/python".to_string());
    }
    assert_eq!(
        server.essentia_python_path().as_deref(),
        Some("/override/python"),
        "override should take precedence over OnceLock probe"
    );
}

#[tokio::test]
async fn analyze_audio_batch_fully_current_reports_page_scoped_cache_counts() {
    for essentia_available in [false, true] {
        let (server, _audio_dir, _store_dir) =
            create_fully_current_audio_batch_server(essentia_available);
        let result = server
            .analyze_audio_batch(Parameters(AnalyzeAudioBatchParams {
                filters: SearchFilterParams::default(),
                track_ids: Some(vec!["current-one".to_string(), "current-two".to_string()]),
                playlist_id: None,
                max_tracks: Some(10),
                offset: Some(0),
                skip_cached: Some(true),
                concurrency: Some(1),
            }))
            .await
            .expect("fully current audio page should succeed without analyzer work");
        let payload = extract_json(&result);

        assert_eq!(payload["summary"]["total"], 2);
        assert_eq!(payload["summary"]["analyzed"], 0);
        assert_eq!(payload["summary"]["cached"], 2);
        assert_eq!(payload["summary"]["failed"], 0);
        assert_eq!(payload["summary"]["essentia_available"], essentia_available);
        assert_eq!(payload["summary"]["essentia_analyzed"], 0);
        assert_eq!(
            payload["summary"]["essentia_cached"],
            if essentia_available { 2 } else { 0 }
        );
        assert_eq!(payload["summary"]["essentia_failed"], 0);
        assert_eq!(payload["page"]["examined_tracks"], 2);
        assert_eq!(payload["page"]["selected_tracks"], 0);
        assert_eq!(payload["page"]["fully_cached_skipped"], 2);
        assert_eq!(payload["page"]["next_offset"], serde_json::Value::Null);
        assert_eq!(payload["page"]["has_more"], false);
        assert!(
            payload["results"]
                .as_array()
                .expect("audio results should be an array")
                .is_empty(),
            "fully current candidates must not inflate bounded result payloads"
        );
    }
}

#[tokio::test]
async fn classify_tracks_audio_cache_ignores_stale_file_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let stale_size_path = audio_dir.path().join("classify-stale-size.flac");
    let stale_mtime_path = audio_dir.path().join("classify-stale-mtime.flac");
    let (stale_size_file_size, stale_size_file_mtime) =
        write_test_audio_file(&stale_size_path, 1000);
    let (stale_mtime_file_size, stale_mtime_file_mtime) =
        write_test_audio_file(&stale_mtime_path, 1001);
    let stale_size_path_str = stale_size_path.to_string_lossy().to_string();
    let stale_mtime_path_str = stale_mtime_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("classify-stale-size", &stale_size_path_str);
    insert_test_track(
        &db_conn,
        "classify-stale-mtime",
        "Classify Stale Mtime",
        "g1",
        &stale_mtime_path_str,
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let stale_audio_json = r#"{
        "bpm": 128.0,
        "duration_seconds": 240.0,
        "decay_mid_tau": 240.0,
        "key_clarity": 0.72,
        "key_confidence": 0.88,
        "kick_pattern": "four_on_floor"
    }"#;
    set_test_audio_analysis(
        &store_conn,
        &stale_size_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        stale_size_file_size + 1,
        stale_size_file_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        stale_audio_json,
    )
    .expect("stale-size stratum cache should seed");
    set_test_audio_analysis(
        &store_conn,
        &stale_mtime_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        stale_mtime_file_size,
        stale_mtime_file_mtime + 1,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        stale_audio_json,
    )
    .expect("stale-mtime stratum cache should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .classify_tracks(Parameters(ClassifyTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "classify-stale-size".to_string(),
                "classify-stale-mtime".to_string(),
            ]),
            playlist_id: None,
            max_tracks: Some(2),
            offset: None,
            genre_overrides: None,
            format: Some(ClassifyFormat::Full),
            auto_stage: None,
        }))
        .await
        .expect("classify_tracks should succeed");
    let payload = extract_json(&result);
    let results = payload["results"]
        .as_array()
        .expect("classification results should be an array");
    assert_eq!(results.len(), 2);

    for track_id in ["classify-stale-size", "classify-stale-mtime"] {
        let item = results
            .iter()
            .find(|item| item["track_id"] == track_id)
            .unwrap_or_else(|| panic!("{track_id} should be present"));
        let flags = item["flags"]
            .as_array()
            .expect("classification flags should be an array");
        assert!(
            flags.iter().any(|flag| flag == "no-audio"),
            "current-schema stale identity row for {track_id} must be excluded from classification evidence"
        );
    }
}

#[tokio::test]
async fn cache_coverage_reports_provider_coverage_and_gap_counts() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let with_genre_path = audio_dir.path().join("coverage-1.flac");
    let fresh_path = audio_dir.path().join("coverage-2.flac");
    let stale_schema_path = audio_dir.path().join("coverage-3.flac");
    let stale_identity_path = audio_dir.path().join("coverage-4.flac");
    write_test_audio_file(&with_genre_path, 1000);
    let (fresh_size, fresh_mtime) = write_test_audio_file(&fresh_path, 1001);
    let (stale_schema_size, stale_schema_mtime) = write_test_audio_file(&stale_schema_path, 1002);
    let (stale_identity_size, stale_identity_mtime) =
        write_test_audio_file(&stale_identity_path, 1003);
    let with_genre_path_str = with_genre_path.to_string_lossy().to_string();
    let fresh_path_str = fresh_path.to_string_lossy().to_string();
    let stale_schema_path_str = stale_schema_path.to_string_lossy().to_string();
    let stale_identity_path_str = stale_identity_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("coverage-with-genre", &with_genre_path_str);
    insert_test_track(
        &db_conn,
        "coverage-no-genre-1",
        "No Genre One",
        "",
        &fresh_path_str,
    );
    insert_test_track(
        &db_conn,
        "coverage-no-genre-2",
        "No Genre Two",
        "",
        &stale_schema_path_str,
    );
    insert_test_track(
        &db_conn,
        "coverage-no-genre-3",
        "No Genre Three",
        "",
        &stale_identity_path_str,
    );
    insert_test_track(
        &db_conn,
        "coverage-no-genre-4",
        "No Genre Four",
        "",
        "/missing/coverage-no-genre-4.flac",
    );

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    let norm_artist = crate::domain::metadata::normalize_for_matching("Aníbal");
    let norm_title_one = crate::domain::metadata::normalize_for_matching("No Genre One");
    let norm_title_two = crate::domain::metadata::normalize_for_matching("No Genre Two");
    let norm_title_three = crate::domain::metadata::normalize_for_matching("No Genre Three");
    let norm_title_four = crate::domain::metadata::normalize_for_matching("No Genre Four");

    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        "stratum-dsp",
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":127.1,"key":"Am"}"#,
    )
    .expect("stratum cache should be seeded");
    let fresh_essentia_payload = valid_test_essentia_payload(serde_json::json!({
        "danceability": 0.81,
    }));
    set_test_audio_analysis(
        &store_conn,
        &fresh_path_str,
        "essentia",
        fresh_size,
        fresh_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &fresh_essentia_payload,
    )
    .expect("essentia cache should be seeded");
    set_test_audio_analysis(
        &store_conn,
        &stale_schema_path_str,
        "stratum-dsp",
        stale_schema_size,
        stale_schema_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":"invalid"}"#,
    )
    .expect("invalid current stratum cache should be seeded");
    set_test_audio_analysis(
        &store_conn,
        &stale_schema_path_str,
        "essentia",
        stale_schema_size,
        stale_schema_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        "not-json",
    )
    .expect("invalid current Essentia cache should be seeded");
    set_test_audio_analysis(
        &store_conn,
        &stale_identity_path_str,
        "stratum-dsp",
        stale_identity_size + 1,
        stale_identity_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":129.0,"key":"Am"}"#,
    )
    .expect("stale-size stratum cache should be seeded");
    set_test_audio_analysis(
        &store_conn,
        &stale_identity_path_str,
        "essentia",
        stale_identity_size,
        stale_identity_mtime + 1,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        r#"{"danceability":0.75}"#,
    )
    .expect("stale-mtime essentia cache should be seeded");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_one,
        Some("encoded paths"),
        Some("exact"),
        Some(r#"{"styles":["Deep House"]}"#),
    )
    .expect("discogs cache should be seeded for first ungenred track");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_two,
        Some("encoded paths"),
        Some("none"),
        None,
    )
    .expect("discogs no-match should be seeded for second ungenred track");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_three,
        Some("encoded paths"),
        Some("exact"),
        Some(r#"{"styles":["Unmapped Test Style"]}"#),
    )
    .expect("unmapped Discogs result should be seeded for third ungenred track");
    store::set_enrichment(
        &store_conn,
        "discogs",
        &norm_artist,
        &norm_title_four,
        Some("different album"),
        Some("exact"),
        Some(r#"{"styles":["Tech House"]}"#),
    )
    .expect("wrong-album Discogs result should be seeded for fourth ungenred track");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    server
        .context
        .analysis
        .essentia_python
        .set(Some("/tmp/fake-essentia-python".to_string()))
        .expect("essentia probe cache should be set exactly once");

    let result = server
        .cache_coverage(Parameters(CacheCoverageParams {
            filters: SearchFilterParams {
                has_genre: Some(false),
                ..Default::default()
            },
            track_ids: None,
            playlist_id: None,
            max_tracks: None,
        }))
        .await
        .expect("cache_coverage should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["scope"]["total_tracks"], 5);
    assert_eq!(payload["scope"]["matched_tracks"], 4);
    assert_eq!(payload["scope"]["filter_description"], "has_genre = false");

    assert_eq!(payload["coverage"]["stratum_dsp"]["cached"], 2);
    assert_eq!(payload["coverage"]["stratum_dsp"]["percent"], 50.0);

    assert_eq!(payload["coverage"]["essentia"]["cached"], 2);
    assert_eq!(payload["coverage"]["essentia"]["percent"], 50.0);
    assert_eq!(payload["coverage"]["essentia"]["installed"], true);

    assert_eq!(payload["classification_readiness"]["full"], 1);
    assert_eq!(payload["classification_readiness"]["degraded"], 3);
    assert_eq!(
        payload["classification_readiness"]["degraded_reasons"]["missing_stratum"],
        2
    );
    assert_eq!(
        payload["classification_readiness"]["degraded_reasons"]["invalid_stratum"],
        1
    );
    assert_eq!(
        payload["classification_readiness"]["degraded_reasons"]["missing_essentia"],
        2
    );
    assert_eq!(
        payload["classification_readiness"]["degraded_reasons"]["invalid_essentia"],
        1
    );
    assert_eq!(
        payload["classification_readiness"]["essentia_runtime_available"],
        true
    );

    assert_eq!(payload["coverage"]["discogs"]["searched"], 3);
    assert_eq!(payload["coverage"]["discogs"]["searched_percent"], 75.0);
    assert_eq!(payload["coverage"]["discogs"]["has_result"], 2);
    assert_eq!(payload["coverage"]["discogs"]["has_result_percent"], 50.0);
    assert_eq!(payload["coverage"]["discogs"]["usable_genre"], 1);
    assert_eq!(payload["coverage"]["discogs"]["matched_unmapped"], 1);

    assert_eq!(payload["gaps"]["no_audio_analysis"], 2);
    assert_eq!(payload["gaps"]["no_enrichment"], 3);
    assert_eq!(payload["gaps"]["no_data_at_all"], 2);
    assert_eq!(payload["gaps"]["discogs"]["not_searched"], 1);
    assert_eq!(payload["gaps"]["discogs"]["searched_no_match"], 1);
    assert_eq!(payload["gaps"]["discogs"]["matched_unmapped"], 1);
}

#[tokio::test]
async fn cache_coverage_excludes_sampler_tracks_for_id_and_playlist_scopes() {
    let db_conn = create_single_track_test_db("coverage-base", "/music/coverage-base.flac");
    insert_test_track(
        &db_conn,
        "coverage-nonsample",
        "Coverage Non Sample",
        "",
        "/music/coverage-nonsample.flac",
    );
    let sampler_path = format!("/music{}CoverageSampler.wav", db::SAMPLER_PATH_FRAGMENT);
    insert_test_track(
        &db_conn,
        "coverage-sampler",
        "Coverage Sampler",
        "",
        &sampler_path,
    );

    db_conn
        .execute_batch(
            "CREATE TABLE djmdSongPlaylist (
                    PlaylistID VARCHAR(255),
                    ContentID VARCHAR(255),
                    TrackNo INTEGER
                );",
        )
        .expect("playlist table should be created for test");
    db_conn
        .execute(
            "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
            params!["pl-cache", "coverage-nonsample", 1],
        )
        .expect("non-sampler playlist entry should insert");
    db_conn
        .execute(
            "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
            params!["pl-cache", "coverage-sampler", 2],
        )
        .expect("sampler playlist entry should insert");

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

    let id_scope = server
        .cache_coverage(Parameters(CacheCoverageParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec![
                "coverage-nonsample".to_string(),
                "coverage-sampler".to_string(),
            ]),
            playlist_id: None,
            max_tracks: None,
        }))
        .await
        .expect("cache_coverage track_ids scope should succeed");
    let id_payload = extract_json(&id_scope);
    assert!(id_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
    assert_eq!(id_payload["scope"]["matched_tracks"], 1);
    assert_eq!(id_payload["gaps"]["no_data_at_all"], 1);

    let playlist_scope = server
        .cache_coverage(Parameters(CacheCoverageParams {
            filters: SearchFilterParams::default(),
            track_ids: None,
            playlist_id: Some("pl-cache".to_string()),
            max_tracks: None,
        }))
        .await
        .expect("cache_coverage playlist scope should succeed");
    let playlist_payload = extract_json(&playlist_scope);
    assert!(playlist_payload["scope"]["total_tracks"].as_u64().unwrap() >= 2);
    assert_eq!(playlist_payload["scope"]["matched_tracks"], 1);
    assert_eq!(playlist_payload["gaps"]["no_data_at_all"], 1);
}

#[tokio::test]
async fn classification_calibration_ignores_stale_and_partial_audio_identity() {
    let audio_dir = tempfile::tempdir().expect("temp audio dir should create");
    let mut deep_paths = Vec::new();
    for i in 1..=5 {
        let path = audio_dir.path().join(format!("calibrate-deep-{i}.flac"));
        let (file_size, file_mtime) = write_test_audio_file(&path, 1000 + i);
        deep_paths.push((path.to_string_lossy().to_string(), file_size, file_mtime));
    }
    let techno_path = audio_dir.path().join("calibrate-techno-stale.flac");
    let (techno_size, techno_mtime) = write_test_audio_file(&techno_path, 1200);
    let techno_path_str = techno_path.to_string_lossy().to_string();
    let techno_fresh_path = audio_dir.path().join("calibrate-techno-fresh.flac");
    let (techno_fresh_size, techno_fresh_mtime) = write_test_audio_file(&techno_fresh_path, 1201);
    let techno_fresh_path_str = techno_fresh_path.to_string_lossy().to_string();
    let deep_invalid_path = audio_dir.path().join("calibrate-deep-invalid.flac");
    let (deep_invalid_size, deep_invalid_mtime) = write_test_audio_file(&deep_invalid_path, 1202);
    let deep_invalid_path_str = deep_invalid_path.to_string_lossy().to_string();

    let db_conn = create_single_track_test_db("calibrate-deep-1", &deep_paths[0].0);
    db_conn
        .execute_batch(
            "
            INSERT INTO djmdGenre (ID, Name) VALUES ('g2', 'Techno');
            CREATE TABLE djmdPlaylist (
                ID VARCHAR(255) PRIMARY KEY,
                Name VARCHAR(255),
                ParentID VARCHAR(255) DEFAULT '',
                Attribute INTEGER DEFAULT 0,
                Seq INTEGER DEFAULT 0,
                rb_local_deleted INTEGER DEFAULT 0
            );
            CREATE TABLE djmdSongPlaylist (
                PlaylistID VARCHAR(255),
                ContentID VARCHAR(255),
                TrackNo INTEGER
            );
            INSERT INTO djmdPlaylist (ID, Name, Seq) VALUES ('pl-verified', 'genre_verified', 1);
            ",
        )
        .expect("calibration schema should initialize");

    for i in 2..=5 {
        insert_test_track(
            &db_conn,
            &format!("calibrate-deep-{i}"),
            &format!("Calibrate Deep {i}"),
            "g1",
            &deep_paths[i as usize - 1].0,
        );
    }
    insert_test_track(
        &db_conn,
        "calibrate-techno-stale",
        "Calibrate Techno Stale",
        "g2",
        &techno_path_str,
    );
    insert_test_track(
        &db_conn,
        "calibrate-techno-fresh",
        "Calibrate Techno Fresh",
        "g2",
        &techno_fresh_path_str,
    );
    insert_test_track(
        &db_conn,
        "calibrate-deep-invalid",
        "Calibrate Deep Invalid",
        "g1",
        &deep_invalid_path_str,
    );

    for (track_no, track_id) in [
        "calibrate-deep-1",
        "calibrate-deep-2",
        "calibrate-deep-3",
        "calibrate-deep-4",
        "calibrate-deep-5",
        "calibrate-techno-stale",
        "calibrate-techno-fresh",
        "calibrate-deep-invalid",
    ]
    .iter()
    .enumerate()
    {
        db_conn
            .execute(
                "INSERT INTO djmdSongPlaylist (PlaylistID, ContentID, TrackNo) VALUES (?1, ?2, ?3)",
                params!["pl-verified", track_id, track_no as i64 + 1],
            )
            .expect("playlist entry should insert");
    }

    let store_dir = tempfile::tempdir().expect("temp store dir should create");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(
        store_path
            .to_str()
            .expect("temp store path should be UTF-8"),
    )
    .expect("temp internal store should open");

    for (path, file_size, file_mtime) in &deep_paths {
        let essentia_payload = if path == &deep_paths[0].0 {
            valid_test_essentia_payload(serde_json::json!({
                "danceability": null,
                "onset_rate": null,
            }))
        } else {
            valid_test_essentia_payload(serde_json::json!({
                "danceability": 0.72,
                "onset_rate": 4.1,
            }))
        };
        set_test_audio_analysis(
            &store_conn,
            path,
            crate::adapters::audio::ANALYZER_STRATUM,
            *file_size,
            *file_mtime,
            crate::adapters::audio::STRATUM_SCHEMA_VERSION,
            r#"{"bpm":127.0,"decay_mid_tau":0.21,"key_clarity":0.72}"#,
        )
        .expect("fresh stratum analysis should seed");
        set_test_audio_analysis(
            &store_conn,
            path,
            crate::adapters::audio::ANALYZER_ESSENTIA,
            *file_size,
            *file_mtime,
            crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
            &essentia_payload,
        )
        .expect("fresh Essentia analysis should seed");
    }
    set_test_audio_analysis(
        &store_conn,
        &deep_invalid_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        deep_invalid_size,
        deep_invalid_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":"invalid"}"#,
    )
    .expect("current malformed Stratum analysis should seed");
    let invalid_pair_essentia_payload = valid_test_essentia_payload(serde_json::json!({
        "danceability": 0.70,
        "onset_rate": 4.0,
    }));
    set_test_audio_analysis(
        &store_conn,
        &deep_invalid_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        deep_invalid_size,
        deep_invalid_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &invalid_pair_essentia_payload,
    )
    .expect("valid Essentia pair for malformed Stratum should seed");
    let partial_essentia_payload = valid_test_essentia_payload(serde_json::json!({
        "danceability": 0.61,
        "onset_rate": 4.8,
    }));
    set_test_audio_analysis(
        &store_conn,
        &techno_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        techno_size + 1,
        techno_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":132.0,"decay_mid_tau":240.0,"key_clarity":0.40}"#,
    )
    .expect("stale-identity stratum analysis should seed");
    set_test_audio_analysis(
        &store_conn,
        &techno_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        techno_size,
        techno_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &partial_essentia_payload,
    )
    .expect("fresh Essentia analysis should seed for partial sample");
    let contrast_essentia_payload = valid_test_essentia_payload(serde_json::json!({
        "danceability": 0.62,
        "onset_rate": 4.9,
    }));
    set_test_audio_analysis(
        &store_conn,
        &techno_fresh_path_str,
        crate::adapters::audio::ANALYZER_STRATUM,
        techno_fresh_size,
        techno_fresh_mtime,
        crate::adapters::audio::STRATUM_SCHEMA_VERSION,
        r#"{"bpm":132.0,"decay_mid_tau":240.0,"key_clarity":0.40}"#,
    )
    .expect("fresh contrast sample should seed");
    set_test_audio_analysis(
        &store_conn,
        &techno_fresh_path_str,
        crate::adapters::audio::ANALYZER_ESSENTIA,
        techno_fresh_size,
        techno_fresh_mtime,
        crate::adapters::audio::ESSENTIA_SCHEMA_VERSION,
        &contrast_essentia_payload,
    )
    .expect("fresh Essentia contrast sample should seed");

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .calibrate_audio_profiles(Parameters(CalibrateAudioProfilesParams {
            playlist: Some("genre_verified".to_string()),
        }))
        .await
        .expect("calibrate_audio_profiles should succeed");
    let payload = extract_json(&result);

    assert_eq!(payload["status"], "calibrated");
    assert_eq!(payload["total_tracks"], 8);
    assert_eq!(payload["tracks_with_features"], 8);
    assert_eq!(payload["tracks_with_complete_classification_audio"], 6);
    assert_eq!(payload["tracks_with_scorable_features"], 6);
    assert_eq!(payload["missing_required_stratum"], 1);
    assert_eq!(payload["invalid_required_stratum"], 1);
    assert_eq!(payload["missing_required_essentia"], 0);
    assert_eq!(payload["skipped_no_audio"], 0);
    assert_eq!(payload["skipped_incomplete_classification_audio"], 2);
}
