use crate::mcp::planning::{
    BuildSetParams, DescribePoolParams, DiscoverPoolsParams, ExpandPoolParams,
    QueryTransitionCandidatesParams, ScorePoolCompatibilityParams, ScoreTransitionParams,
};
use crate::mcp::server::ReklawdboxServer;

#[test]
fn mcp_planning_contract_live_schemas_equal_transport_types() {
    let router = ReklawdboxServer::build_tool_router();
    let expected = [
        (
            "score_transition",
            schemars::schema_for!(ScoreTransitionParams),
        ),
        (
            "query_transition_candidates",
            schemars::schema_for!(QueryTransitionCandidatesParams),
        ),
        ("build_set", schemars::schema_for!(BuildSetParams)),
        (
            "score_pool_compatibility",
            schemars::schema_for!(ScorePoolCompatibilityParams),
        ),
        ("expand_pool", schemars::schema_for!(ExpandPoolParams)),
        ("describe_pool", schemars::schema_for!(DescribePoolParams)),
        ("discover_pools", schemars::schema_for!(DiscoverPoolsParams)),
    ];

    for (tool_name, schema) in expected {
        let tool = router
            .get(tool_name)
            .unwrap_or_else(|| panic!("{tool_name} should be registered"));
        let mut live = serde_json::to_value(tool.input_schema.as_ref())
            .unwrap_or_else(|error| panic!("{tool_name} schema should serialize: {error}"));
        let mut generated = serde_json::to_value(schema).unwrap_or_else(|error| {
            panic!("{tool_name} generated schema should serialize: {error}")
        });
        let live_object = live
            .as_object_mut()
            .expect("live schema should be an object");
        live_object.remove("$schema");
        let generated_object = generated
            .as_object_mut()
            .expect("generated schema should be an object");
        generated_object.remove("$schema");
        generated_object.remove("title");
        assert_eq!(live, generated, "{tool_name} input schema drifted");
    }
}

#[test]
fn mcp_planning_contract_build_set_bpm_range_deserializes_from_json_array() {
    let json = serde_json::json!({
        "track_ids": ["a", "b"],
        "target_tracks": 4,
        "beam_width": 3,
        "bpm_range": [124.0, 131.0],
    });
    let p: BuildSetParams =
        serde_json::from_value(json).expect("bpm_range should deserialize from JSON array");
    assert_eq!(p.bpm_range, Some((124.0, 131.0)));
    assert_eq!(p.beam_width, Some(3));
    assert!(p.candidates.is_none());
}

#[test]
fn mcp_planning_contract_build_set_without_new_fields_deserializes() {
    let json = serde_json::json!({
        "track_ids": ["a"],
        "target_tracks": 2,
        "candidates": 2,
    });
    let p: BuildSetParams = serde_json::from_value(json).expect("legacy fields should still work");
    assert_eq!(p.candidates, Some(2));
    assert!(p.beam_width.is_none());
    assert!(p.bpm_range.is_none());
}

#[test]
fn mcp_planning_contract_query_transition_candidates_deserializes_from_json() {
    let json = serde_json::json!({
        "from_track_id": "t1",
        "pool_track_ids": ["t2", "t3"],
        "target_bpm": 130.0,
        "limit": 5,
    });
    let p: QueryTransitionCandidatesParams =
        serde_json::from_value(json).expect("QueryTransitionCandidatesParams should deserialize");
    assert_eq!(p.source_track_id, "t1");
    assert_eq!(p.candidate_track_ids.as_ref().unwrap().len(), 2);
    assert_eq!(p.target_bpm, Some(130.0));
    assert_eq!(p.limit, Some(5));
    assert!(p.playlist_id.is_none());
}
