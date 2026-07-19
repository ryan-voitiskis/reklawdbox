use super::*;

#[tokio::test]
async fn update_tracks_stages_changes() {
    let server = ReklawdboxServer::new(None);
    let known_genre = genre::GENRES
        .first()
        .copied()
        .unwrap_or("House")
        .to_string();

    let result = server
        .update_tracks(Parameters(UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "test-track-1".to_string(),
                genre: Some(known_genre),
                comments: Some("staged by test".to_string()),
                rating: Some(4),
                color: None,
                label: None,
                year: None,
                album: None,
            }],
        }))
        .await
        .expect("update_tracks should succeed");

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    assert_eq!(
        payload
            .get("total_pending")
            .and_then(serde_json::Value::as_u64)
            .expect("total_pending should be present"),
        1
    );
    assert!(
        payload.get("changes").is_none(),
        "update_tracks should not echo changes back"
    );
}

#[tokio::test]
async fn update_tracks_via_router_warns_non_taxonomy_genre() {
    let result = call_tool_via_router(
        "update_tracks",
        serde_json::json!({
            "changes": [{
                "track_id": "router-test-track-1",
                "genre": "NotInTaxonomy"
            }]
        })
        .as_object()
        .cloned(),
    )
    .await;

    let payload = extract_json(&result);
    assert_eq!(
        payload
            .get("staged")
            .and_then(serde_json::Value::as_u64)
            .expect("staged should be present"),
        1
    );
    let warnings = payload
        .get("warnings")
        .and_then(serde_json::Value::as_array)
        .expect("warnings should be present for non-taxonomy genre");
    assert!(
        !warnings.is_empty(),
        "warnings should include at least one non-taxonomy genre warning"
    );
}
