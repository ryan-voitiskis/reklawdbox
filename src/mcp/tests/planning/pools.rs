use rmcp::handler::server::wrapper::Parameters;

use crate::adapters::state as store;
use crate::mcp::library::SearchFilterParams;
use crate::mcp::planning::{DiscoverPoolsParams, ScorePoolCompatibilityParams};

use super::super::common::{
    create_server_with_connections, default_http_client_for_tests, extract_json,
};
use super::support::{create_build_set_test_db, seed_build_set_cache};

#[tokio::test]
async fn mcp_planning_contract_pool_cohesion_returns_representative_json() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .score_pool_compatibility(Parameters(ScorePoolCompatibilityParams {
            track_a: None,
            track_b: None,
            track_id: None,
            pool_track_ids: Some(track_ids[..3].to_vec()),
            master_tempo: Some(true),
            reference_bpm: Some(124.0),
            preset: None,
        }))
        .await
        .expect("pool cohesion should succeed for fixture tracks");

    let payload = extract_json(&result);
    assert_eq!(payload["mode"], "cohesion");
    assert_eq!(payload["reference_bpm"], 124.0);
    assert_eq!(payload["master_tempo"], true);
    assert!(payload["mean_pairwise"].is_number());
    assert!(payload["min_pairwise"].is_number());
    assert!(payload["weakest_member_id"].is_string());
    assert!(payload["medoid_id"].is_string());
    let pairs = payload["per_pair"]
        .as_array()
        .expect("cohesion per_pair should be an array");
    assert_eq!(pairs.len(), 3, "three tracks produce three unordered pairs");
    for pair in pairs {
        assert!(pair["track_a"].is_string());
        assert!(pair["track_b"].is_string());
        assert!(pair["scores"]["composite"].is_number());
    }
}

#[tokio::test]
async fn mcp_planning_contract_pool_discovery_returns_bounded_json() {
    let (db_conn, track_ids, audio_dir) = create_build_set_test_db();
    let store_dir = tempfile::tempdir().expect("temp store dir");
    let store_path = store_dir.path().join("internal.sqlite3");
    let store_conn = store::open(store_path.to_str().unwrap()).expect("store open");
    seed_build_set_cache(&store_conn, audio_dir.path());

    let server =
        create_server_with_connections(db_conn, store_conn, default_http_client_for_tests());
    let result = server
        .discover_pools(Parameters(DiscoverPoolsParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(track_ids.clone()),
            playlist_id: None,
            max_tracks: None,
            threshold: Some(0.3),
            min_pool_size: Some(2),
            max_pool_size: Some(4),
            max_pools: Some(2),
            master_tempo: Some(true),
            reference_bpm: Some(126.0),
            preset: None,
        }))
        .await
        .expect("pool discovery should succeed for fixture tracks");

    let payload = extract_json(&result);
    assert_eq!(payload["threshold"], 0.3);
    assert_eq!(payload["master_tempo"], true);
    assert_eq!(payload["reference_bpm"], 126.0);
    assert_eq!(payload["tracks_analyzed"], track_ids.len());
    assert!(payload["bridge_tracks"].is_array());
    let pools = payload["pools"]
        .as_array()
        .expect("discovery pools should be an array");
    assert!(
        !pools.is_empty(),
        "fixture should produce a compatible pool"
    );
    assert!(pools.len() <= 2, "max_pools should cap discovery output");
    for pool in pools {
        let size = pool["size"].as_u64().expect("pool size should be numeric");
        assert!((2..=4).contains(&size));
        assert!(pool["mean_compatibility"].is_number());
        assert!(pool["min_compatibility"].is_number());
        assert!(pool["score"].is_number());
        assert_eq!(pool["tracks"].as_array().unwrap().len() as u64, size);
    }
}
