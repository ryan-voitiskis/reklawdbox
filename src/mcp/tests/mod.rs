mod analysis;
mod audit;
mod classification;
mod common;
mod enrichment;
mod files;
mod help;
mod library;
mod metadata;
mod planning;

use crate::mcp::analysis::AnalyzeAudioBatchParams;
use crate::mcp::audit::ScanDuplicatesParams;
use crate::mcp::enrichment::{EnrichTracksParams, ResolveTracksDataParams};
use crate::mcp::library::{SearchFilterParams, SearchTracksParams};
use crate::mcp::metadata::BackfillLabelsParams;
use crate::mcp::server::ReklawdboxServer;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use schemars::JsonSchema;

use self::common::{
    create_enrich_cache_writer_test_server, create_selector_pagination_test_db, extract_json,
};

fn assert_structured_matches_text(result: &CallToolResult, tool_name: &str) {
    let structured = result
        .structured_content
        .as_ref()
        .unwrap_or_else(|| panic!("{tool_name} should return structured content"));
    assert_eq!(
        structured,
        &extract_json(result),
        "{tool_name} structured content and compatibility JSON text should match"
    );
}

fn resolve_local_schema<'a>(
    root: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> &'a serde_json::Value {
    schema
        .get("$ref")
        .and_then(serde_json::Value::as_str)
        .and_then(|reference| reference.strip_prefix('#'))
        .and_then(|pointer| root.pointer(pointer))
        .unwrap_or(schema)
}

fn schema_allows_type(
    root: &serde_json::Value,
    schema: &serde_json::Value,
    expected: &str,
) -> bool {
    let schema = resolve_local_schema(root, schema);
    schema["type"].as_str() == Some(expected)
        || schema["type"]
            .as_array()
            .is_some_and(|types| types.iter().any(|value| value.as_str() == Some(expected)))
        || ["anyOf", "oneOf"].iter().any(|keyword| {
            schema[*keyword].as_array().is_some_and(|alternatives| {
                alternatives
                    .iter()
                    .any(|alternative| schema_allows_type(root, alternative, expected))
            })
        })
}

fn assert_terminal_cursor_contract(
    tool_name: &str,
    continuation_field: &str,
    result: &CallToolResult,
) {
    let router = ReklawdboxServer::build_tool_router();
    let tool = router
        .get(tool_name)
        .unwrap_or_else(|| panic!("{tool_name} should be registered"));
    let tool_schema = serde_json::to_value(tool).expect("tool metadata should serialize");
    let output_schema = &tool_schema["outputSchema"];
    assert!(
        output_schema["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == continuation_field)),
        "{tool_name} should require {continuation_field}"
    );
    let continuation_schema = resolve_local_schema(
        output_schema,
        &output_schema["properties"][continuation_field],
    );
    assert!(
        continuation_schema["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "next_offset")),
        "{tool_name} should require {continuation_field}.next_offset"
    );
    let cursor_schema = &continuation_schema["properties"]["next_offset"];
    assert!(
        schema_allows_type(output_schema, cursor_schema, "integer"),
        "{tool_name} next_offset should allow integers"
    );
    assert!(
        schema_allows_type(output_schema, cursor_schema, "null"),
        "{tool_name} next_offset should allow terminal nulls; schema was {cursor_schema}"
    );

    let structured = result
        .structured_content
        .as_ref()
        .unwrap_or_else(|| panic!("{tool_name} should return structured content"));
    let continuation = structured[continuation_field]
        .as_object()
        .unwrap_or_else(|| panic!("{tool_name} should return {continuation_field} as an object"));
    assert!(
        continuation.contains_key("next_offset"),
        "{tool_name} terminal payload should retain required next_offset"
    );
    assert_eq!(
        continuation["next_offset"],
        serde_json::Value::Null,
        "{tool_name} terminal payload should encode next_offset as null"
    );
}

#[tokio::test]
async fn batch_output_schema_contract_advertises_and_returns_typed_payloads() {
    let router = ReklawdboxServer::build_tool_router();
    for (tool_name, continuation_field) in [
        ("enrich_tracks", "page"),
        ("analyze_audio_batch", "page"),
        ("backfill_labels", "conflict_page"),
        ("scan_duplicates", "page"),
    ] {
        let tool = router
            .get(tool_name)
            .unwrap_or_else(|| panic!("{tool_name} should be registered"));
        let schema = serde_json::to_value(tool).expect("tool metadata should serialize");
        assert!(
            schema["outputSchema"].is_object(),
            "{tool_name} should advertise outputSchema"
        );
        assert!(
            schema["outputSchema"]["properties"][continuation_field].is_object(),
            "{tool_name} outputSchema should require {continuation_field}"
        );
    }

    let db_conn = create_selector_pagination_test_db();
    let (server, _store_dir, _store_path) = create_enrich_cache_writer_test_server(db_conn);

    let enrich = server
        .enrich_tracks(Parameters(EnrichTracksParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["t1".to_string()]),
            playlist_id: None,
            max_tracks: Some(0),
            offset: Some(0),
            providers: None,
            skip_cached: Some(true),
            force_refresh: Some(false),
            concurrency: Some(1),
        }))
        .await
        .expect("zero-work enrichment should succeed");
    assert_structured_matches_text(&enrich, "enrich_tracks");
    assert_terminal_cursor_contract("enrich_tracks", "page", &enrich);
    assert_eq!(extract_json(&enrich)["summary"]["tracks_total"], 0);
    assert_eq!(extract_json(&enrich)["summary"]["total"], 0);
    assert_eq!(extract_json(&enrich)["summary"]["cached"], 0);

    let audio = server
        .analyze_audio_batch(Parameters(AnalyzeAudioBatchParams {
            filters: SearchFilterParams::default(),
            track_ids: Some(vec!["t1".to_string()]),
            playlist_id: None,
            max_tracks: Some(0),
            offset: Some(0),
            skip_cached: Some(true),
            concurrency: Some(1),
        }))
        .await
        .expect("zero-work audio batch should succeed");
    assert_structured_matches_text(&audio, "analyze_audio_batch");
    assert_terminal_cursor_contract("analyze_audio_batch", "page", &audio);
    assert_eq!(extract_json(&audio)["summary"]["total"], 0);
    assert_eq!(extract_json(&audio)["summary"]["cached"], 0);
    assert_eq!(extract_json(&audio)["summary"]["essentia_cached"], 0);

    let labels = server
        .backfill_labels(Parameters(BackfillLabelsParams {
            dry_run: Some(true),
            auto_enrich: Some(false),
            max_conflicts: Some(0),
            conflict_offset: Some(0),
        }))
        .await
        .expect("dry-run label backfill should succeed");
    assert_structured_matches_text(&labels, "backfill_labels");
    assert_terminal_cursor_contract("backfill_labels", "conflict_page", &labels);

    let duplicates = server
        .scan_duplicates(Parameters(ScanDuplicatesParams::default()))
        .await
        .expect("empty metadata duplicate scan should succeed");
    assert_structured_matches_text(&duplicates, "scan_duplicates");
    assert_terminal_cursor_contract("scan_duplicates", "page", &duplicates);
}

#[test]
fn flatten_json_round_trip_search_tracks_params() {
    let json = serde_json::json!({
        "query": "burial",
        "artist": "Burial",
        "genre": "Dubstep",
        "rating_min": 3,
        "bpm_min": 130.0,
        "bpm_max": 145.0,
        "key": "Am",
        "has_genre": true,
        "label": "Hyperdub",
        "path": "/Music",
        "added_after": "2026-01-01",
        "added_before": "2026-12-31",
        "playlist": "p1",
        "include_samples": true,
        "limit": 50,
        "offset": 10,
    });
    let p: SearchTracksParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.query.as_deref(), Some("burial"));
    assert_eq!(p.filters.artist.as_deref(), Some("Burial"));
    assert_eq!(p.filters.genre.as_deref(), Some("Dubstep"));
    assert_eq!(p.filters.rating_min, Some(3));
    assert_eq!(p.filters.bpm_min, Some(130.0));
    assert_eq!(p.filters.bpm_max, Some(145.0));
    assert_eq!(p.filters.key.as_deref(), Some("Am"));
    assert_eq!(p.filters.has_genre, Some(true));
    assert_eq!(p.filters.label.as_deref(), Some("Hyperdub"));
    assert_eq!(p.filters.path.as_deref(), Some("/Music"));
    assert_eq!(p.filters.added_after.as_deref(), Some("2026-01-01"));
    assert_eq!(p.filters.added_before.as_deref(), Some("2026-12-31"));
    assert_eq!(p.playlist.as_deref(), Some("p1"));
    assert_eq!(p.include_samples, Some(true));
    assert_eq!(p.limit, Some(50));
    assert_eq!(p.offset, Some(10));
}

#[test]
fn flatten_json_round_trip_enrich_tracks_params() {
    let json = serde_json::json!({
        "genre": "Techno",
        "bpm_min": 125.0,
        "track_ids": ["t1", "t2"],
        "playlist_id": "p1",
        "max_tracks": 20,
        "providers": ["discogs", "bandcamp"],
        "skip_cached": false,
        "force_refresh": true,
    });
    let p: EnrichTracksParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.genre.as_deref(), Some("Techno"));
    assert_eq!(p.filters.bpm_min, Some(125.0));
    assert_eq!(p.filters.query, None);
    assert_eq!(p.track_ids.as_ref().unwrap().len(), 2);
    assert_eq!(p.playlist_id.as_deref(), Some("p1"));
    assert_eq!(p.max_tracks, Some(20));
    assert_eq!(p.skip_cached, Some(false));
    assert_eq!(p.force_refresh, Some(true));
}

#[test]
fn flatten_json_round_trip_analyze_audio_batch_params() {
    let json = serde_json::json!({
        "artist": "Aphex Twin",
        "rating_min": 4,
        "track_ids": ["t1"],
        "max_tracks": 10,
        "skip_cached": true,
    });
    let p: AnalyzeAudioBatchParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.artist.as_deref(), Some("Aphex Twin"));
    assert_eq!(p.filters.rating_min, Some(4));
    assert_eq!(p.track_ids.as_ref().unwrap(), &["t1"]);
    assert_eq!(p.max_tracks, Some(10));
    assert_eq!(p.skip_cached, Some(true));
}

#[test]
fn flatten_json_round_trip_resolve_tracks_data_params() {
    let json = serde_json::json!({
        "key": "Cm",
        "has_genre": false,
        "added_after": "2025-06-01",
        "playlist_id": "p2",
        "max_tracks": 100,
    });
    let p: ResolveTracksDataParams = serde_json::from_value(json).expect("should deserialize");
    assert_eq!(p.filters.key.as_deref(), Some("Cm"));
    assert_eq!(p.filters.has_genre, Some(false));
    assert_eq!(p.filters.added_after.as_deref(), Some("2025-06-01"));
    assert_eq!(p.playlist_id.as_deref(), Some("p2"));
    assert_eq!(p.max_tracks, Some(100));
}

#[test]
fn flatten_json_empty_payload_deserializes_to_all_none() {
    let json = serde_json::json!({});
    let p: SearchTracksParams = serde_json::from_value(json.clone()).expect("SearchTracksParams");
    assert!(p.filters.query.is_none());
    assert!(p.playlist.is_none());
    assert!(p.limit.is_none());

    let p: EnrichTracksParams = serde_json::from_value(json.clone()).expect("EnrichTracksParams");
    assert!(p.filters.genre.is_none());
    assert!(p.track_ids.is_none());

    let p: AnalyzeAudioBatchParams =
        serde_json::from_value(json.clone()).expect("AnalyzeAudioBatchParams");
    assert!(p.filters.artist.is_none());
    assert!(p.track_ids.is_none());

    let p: ResolveTracksDataParams = serde_json::from_value(json).expect("ResolveTracksDataParams");
    assert!(p.filters.key.is_none());
    assert!(p.track_ids.is_none());
}

/// MCP clients expect filter fields at schema top level, not nested under `filters`.
#[test]
fn flatten_schema_has_top_level_filter_properties() {
    let filter_fields = [
        "query",
        "artist",
        "genre",
        "rating_min",
        "bpm_min",
        "bpm_max",
        "key",
        "has_genre",
        "has_label",
        "label",
        "path",
        "added_after",
        "added_before",
    ];

    fn assert_schema_properties<T: JsonSchema>(
        type_name: &str,
        expected: &[&str],
        forbidden: &[&str],
    ) {
        let schema = schemars::schema_for!(T);
        let root = schema.as_value();
        let props = root
            .get("properties")
            .unwrap_or_else(|| panic!("{type_name} schema should have properties"));
        for field in expected {
            assert!(
                props.get(*field).is_some(),
                "{type_name} schema missing top-level property '{field}'"
            );
        }
        for field in forbidden {
            assert!(
                props.get(*field).is_none(),
                "{type_name} schema should NOT have property '{field}'"
            );
        }
    }

    assert_schema_properties::<SearchTracksParams>(
        "SearchTracksParams",
        &[
            &filter_fields[..],
            &["playlist", "include_samples", "limit", "offset"],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<EnrichTracksParams>(
        "EnrichTracksParams",
        &[
            &filter_fields[..],
            &[
                "track_ids",
                "playlist_id",
                "max_tracks",
                "providers",
                "skip_cached",
                "force_refresh",
            ],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<AnalyzeAudioBatchParams>(
        "AnalyzeAudioBatchParams",
        &[
            &filter_fields[..],
            &["track_ids", "playlist_id", "max_tracks", "skip_cached"],
        ]
        .concat(),
        &["filters"],
    );

    assert_schema_properties::<ResolveTracksDataParams>(
        "ResolveTracksDataParams",
        &[
            &filter_fields[..],
            &["track_ids", "playlist_id", "max_tracks"],
        ]
        .concat(),
        &["filters"],
    );
}

#[derive(Debug, PartialEq, Eq)]
struct ClaudeSchemaViolation {
    tool_name: String,
    keyword: &'static str,
    path: String,
}

impl std::fmt::Display for ClaudeSchemaViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "tool `{}` input schema contains prohibited `{}` at `{}`",
            self.tool_name, self.keyword, self.path
        )
    }
}

fn json_pointer_child(path: &str, segment: &str) -> String {
    let escaped = segment.replace('~', "~0").replace('/', "~1");
    format!("{path}/{escaped}")
}

fn validate_claude_input_schema(
    tool_name: &str,
    input_schema: &serde_json::Value,
) -> Result<(), Vec<ClaudeSchemaViolation>> {
    fn visit(
        tool_name: &str,
        value: &serde_json::Value,
        path: &str,
        is_root: bool,
        violations: &mut Vec<ClaudeSchemaViolation>,
    ) {
        match value {
            serde_json::Value::Object(object) => {
                for keyword in ["$ref", "$defs"] {
                    if object.contains_key(keyword) {
                        violations.push(ClaudeSchemaViolation {
                            tool_name: tool_name.to_string(),
                            keyword,
                            path: json_pointer_child(path, keyword),
                        });
                    }
                }
                if is_root {
                    for keyword in ["oneOf", "anyOf"] {
                        if object.contains_key(keyword) {
                            violations.push(ClaudeSchemaViolation {
                                tool_name: tool_name.to_string(),
                                keyword,
                                path: json_pointer_child(path, keyword),
                            });
                        }
                    }
                }
                for (key, child) in object {
                    visit(
                        tool_name,
                        child,
                        &json_pointer_child(path, key),
                        false,
                        violations,
                    );
                }
            }
            serde_json::Value::Array(array) => {
                for (index, child) in array.iter().enumerate() {
                    visit(
                        tool_name,
                        child,
                        &json_pointer_child(path, &index.to_string()),
                        false,
                        violations,
                    );
                }
            }
            _ => {}
        }
    }

    let mut violations = Vec::new();
    visit(tool_name, input_schema, "", true, &mut violations);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

#[test]
fn claude_schema_contract_rejects_forbidden_shapes() {
    fn assert_violation(
        schema: serde_json::Value,
        expected_keyword: &'static str,
        expected_path: &str,
    ) {
        let violations = validate_claude_input_schema("fixture_tool", &schema)
            .expect_err("fixture should violate the Claude input-schema contract");
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].tool_name, "fixture_tool");
        assert_eq!(violations[0].keyword, expected_keyword);
        assert_eq!(violations[0].path, expected_path);
        assert_eq!(
            violations[0].to_string(),
            format!(
                "tool `fixture_tool` input schema contains prohibited `{expected_keyword}` at `{expected_path}`"
            )
        );
    }

    assert_violation(
        serde_json::json!({
            "type": "object",
            "properties": {
                "nested": {
                    "type": "object",
                    "properties": { "value": { "$ref": "#/components/value" } }
                }
            }
        }),
        "$ref",
        "/properties/nested/properties/value/$ref",
    );
    assert_violation(
        serde_json::json!({
            "type": "object",
            "properties": { "nested": { "$defs": {} } }
        }),
        "$defs",
        "/properties/nested/$defs",
    );
    assert_violation(
        serde_json::json!({ "oneOf": [{ "type": "object" }] }),
        "oneOf",
        "/oneOf",
    );
    assert_violation(
        serde_json::json!({ "anyOf": [{ "type": "object" }] }),
        "anyOf",
        "/anyOf",
    );

    let allowed = serde_json::json!({
        "type": "object",
        "properties": {
            "nested": {
                "type": "object",
                "properties": {
                    "value": { "anyOf": [{ "type": "string" }, { "type": "null" }] }
                }
            }
        }
    });
    assert_eq!(
        validate_claude_input_schema("allowed_tool", &allowed),
        Ok(())
    );
}

/// Tool schemas must be free of $ref/$defs and top-level oneOf/anyOf for Claude API compatibility.
#[test]
fn claude_schema_contract_covers_live_router() {
    const EXPECTED_TOOL_COUNT: usize = 52;

    let tools = ReklawdboxServer::build_tool_router().list_all();
    assert_eq!(
        tools.len(),
        EXPECTED_TOOL_COUNT,
        "live MCP router tool count changed: observed {} tools",
        tools.len()
    );

    let mut violations = Vec::new();
    for tool in tools {
        let input_schema = serde_json::to_value(tool.input_schema.as_ref())
            .unwrap_or_else(|error| panic!("{} input schema should serialize: {error}", tool.name));
        if let Err(mut tool_violations) =
            validate_claude_input_schema(tool.name.as_ref(), &input_schema)
        {
            violations.append(&mut tool_violations);
        }
    }

    let failures = violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        violations.is_empty(),
        "Claude input-schema contract violations:\n{failures}"
    );
}
