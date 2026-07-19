use crate::mcp::enrichment::{
    BatchPage, EnrichTracksParams, auth_remediation_message, lookup_output_with_cache_metadata,
    resolve_pending_tracks,
};
use crate::mcp::library::SearchFilterParams;

use super::super::common::{create_selector_pagination_test_db, track_ids};

#[test]
fn pending_batch_page_explicit_ids_keep_caller_order_and_apply_cap() {
    let conn = create_selector_pagination_test_db();
    let ids = vec![
        "t3".to_string(),
        "t1".to_string(),
        "t1".to_string(),
        "t2".to_string(),
    ];
    let selection = resolve_pending_tracks(
        &conn,
        Some(&ids),
        None,
        SearchFilterParams::default(),
        Some(10),
        Some(0),
        50,
        2,
        false,
        |tracks| Ok(vec![false; tracks.len()]),
    )
    .expect("pending explicit-ID selector should resolve");

    assert_eq!(track_ids(&selection.selected), ["t3", "t1"]);
    assert_eq!(
        selection.page,
        BatchPage {
            matched_tracks: 3,
            start_offset: 0,
            examined_tracks: 2,
            selected_tracks: 2,
            fully_cached_skipped: 0,
            next_offset: Some(2),
            has_more: true,
        }
    );
}

#[test]
fn lookup_output_wraps_non_object_in_result_envelope() {
    let output = lookup_output_with_cache_metadata(serde_json::Value::Null, false, None);
    assert_eq!(output["result"], serde_json::Value::Null);
    assert_eq!(output["cache_hit"], false);
    assert!(
        output.get("cached_at").is_none(),
        "live payload should not include cached_at"
    );
}

#[test]
fn lookup_output_with_cache_metadata_keeps_object_payload_shape() {
    let output = lookup_output_with_cache_metadata(
        serde_json::json!({
            "genre": "Techno"
        }),
        true,
        Some("2026-02-20T10:00:00Z"),
    );
    assert_eq!(output["genre"], "Techno");
    assert_eq!(output["cache_hit"], true);
    assert_eq!(output["cached_at"], "2026-02-20T10:00:00Z");
    assert!(
        output.get("result").is_none(),
        "object payloads should not be wrapped in a result envelope"
    );
}

#[test]
fn auth_remediation_no_shell_keeps_url_as_human_confirmed_data() {
    let remediation = crate::adapters::providers::discogs::AuthRemediation {
        message: "Discogs auth required (not a lookup miss).".to_string(),
        auth_url: Some("https://discogs.example/auth/device/o'hare".to_string()),
        poll_interval_seconds: Some(5),
        expires_at: Some(1_777_000_000),
    };

    let message = auth_remediation_message(&remediation);

    assert!(message.contains("not a lookup miss"));
    assert!(message.contains("Auth URL: https://discogs.example/auth/device/o'hare"));
    assert!(message.contains("human confirmation"));
    assert!(
        message.contains("Never pass a broker-supplied URL through a shell or terminal command")
    );
    assert!(!message.contains("open '"));
    assert!(!message.contains("sh -c"));
    assert!(!message.contains('`'));
    assert!(message.contains("Poll interval if polling instead of browser: 5s"));
    assert!(message.contains("Auth session expires_at (unix): 1777000000"));
}

#[test]
fn enrich_tracks_invalid_provider_rejected_by_serde() {
    let json = serde_json::json!({
        "providers": ["spotify"],
    });
    let result = serde_json::from_value::<EnrichTracksParams>(json);
    assert!(
        result.is_err(),
        "serde should reject unknown provider variant"
    );
}
