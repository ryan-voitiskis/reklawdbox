use crate::mcp::help::{HelpParams, handle_help};

use super::common::{call_tool_via_router, extract_json};

#[test]
fn help_public_contract() {
    let menu = handle_help(HelpParams::default()).expect("DB-free help menu should succeed");
    let payload = extract_json(&menu);
    let workflows = payload["workflows"]
        .as_array()
        .expect("help menu should expose workflow records");
    assert_eq!(
        workflows.len(),
        9,
        "runtime help menu should contain nine SOPs"
    );
    for workflow in workflows {
        assert!(workflow["name"].is_string());
        assert!(workflow["summary"].is_string());
        assert!(workflow["key_tools"].is_array());
    }

    assert_eq!(
        payload["reference"], "https://reklawdbox.com/mcp-tools/",
        "runtime help should link to the built MCP reference"
    );
    assert!(
        !payload
            .to_string()
            .contains(&["/reference", "tools/"].join("/")),
        "runtime help must not retain the retired tool-reference route"
    );

    let expected_topics = [
        "genre",
        "genre audit",
        "set",
        "pool",
        "chapter",
        "audit",
        "import",
        "metadata",
        "label",
        "year",
        "album",
        "health",
    ];
    let help_schema = schemars::schema_for!(HelpParams);
    let topic_description = help_schema.as_value()["properties"]["topic"]["description"]
        .as_str()
        .expect("HelpParams.topic should advertise its public vocabulary");
    let schema_topics = topic_description
        .split('\'')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value))
        .collect::<Vec<_>>();
    assert_eq!(
        schema_topics, expected_topics,
        "HelpParams should advertise exactly the twelve public topics"
    );
    let expected_tip = format!(
        "Call help(topic={}) for the full step-by-step SOP.",
        expected_topics
            .iter()
            .map(|topic| format!("'{topic}'"))
            .collect::<Vec<_>>()
            .join("|")
    );
    assert_eq!(
        payload["tip"].as_str(),
        Some(expected_tip.as_str()),
        "the visible topic tip should match the schema vocabulary and order"
    );

    let recommended = payload["recommended_order"]
        .as_str()
        .expect("help menu should expose a recommended sequence");
    let numbered = recommended
        .lines()
        .filter(|line| line.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .collect::<Vec<_>>();
    assert_eq!(
        numbered.len(),
        7,
        "recommended sequence should contain seven steps"
    );
    for (index, line) in numbered.iter().enumerate() {
        assert!(
            line.starts_with(&format!("{}. ", index + 1)),
            "recommended sequence should be consecutively numbered"
        );
    }
    assert!(
        recommended.contains("workflow-specific")
            && recommended.contains("scoped cache_coverage")
            && recommended.contains("provider access is conditional"),
        "help should explain scoped, conditional readiness"
    );
    assert!(
        !recommended.contains("run `reklawdbox hydrate`"),
        "help must not impose universal hydration"
    );
    assert!(
        recommended.contains("Reload Tag") && recommended.contains("metadata or playlist XML"),
        "help should expose the real Rekordbox checkpoints"
    );

    let topic_routes = [
        ("genre", "Genre Classification"),
        ("genre audit", "Genre Audit"),
        ("set", "Set Building"),
        ("pool", "Pool Building"),
        ("chapter", "Chapter Set Planning"),
        ("audit", "Collection Audit"),
        ("import", "Batch Import"),
        ("metadata", "Metadata Backfill"),
        ("label", "Metadata Backfill"),
        ("year", "Metadata Backfill"),
        ("album", "Metadata Backfill"),
        ("health", "Library Health"),
    ];
    for (topic, expected_workflow) in topic_routes {
        let response = handle_help(HelpParams {
            topic: Some(topic.to_owned()),
        })
        .unwrap_or_else(|error| panic!("DB-free help topic {topic:?} failed: {error:?}"));
        let topic_payload = extract_json(&response);
        assert_eq!(
            topic_payload["workflow"], expected_workflow,
            "topic {topic}"
        );
        assert!(topic_payload["key_tools"].is_array(), "topic {topic}");
        assert!(
            topic_payload["sop"]
                .as_str()
                .is_some_and(|sop| !sop.is_empty()),
            "topic {topic}"
        );
        if topic == "audit" {
            assert!(
                topic_payload["sop"]
                    .as_str()
                    .is_some_and(|sop| sop.contains("Reload Tag")),
                "collection audit should retain its Reload Tag checkpoint"
            );
        }
        if topic == "album" {
            assert!(
                topic_payload["sop"]
                    .as_str()
                    .is_some_and(|sop| sop.contains("Step 1c")),
                "metadata help should retain its label-research checkpoint"
            );
        }
    }
    let unknown = extract_json(
        &handle_help(HelpParams {
            topic: Some("not-a-public-topic".to_owned()),
        })
        .expect("unknown topic should return a DB-free guidance payload"),
    );
    assert_eq!(
        unknown["error"],
        format!(
            "No workflow matching 'not-a-public-topic'. Try: {}.",
            expected_topics.join(", ")
        )
    );
}

#[tokio::test]
async fn playlist_import_help_contract() {
    for topic in ["set", "pool", "chapter"] {
        let arguments = serde_json::json!({ "topic": topic }).as_object().cloned();
        let result = call_tool_via_router("help", arguments).await;
        let payload = extract_json(&result);
        let sop = payload["sop"]
            .as_str()
            .unwrap_or_else(|| panic!("help topic '{topic}' should return SOP text"));
        let lower = sop.to_lowercase();

        for required in [
            "rekordbox xml",
            "playlists",
            "drag",
            "track count",
            "first and last",
            "track order",
        ] {
            assert!(
                lower.contains(required),
                "help topic '{topic}' should include playlist import guidance containing '{required}'"
            );
        }
        assert!(
            !sop.contains("import XmlPlaylistImportSteps"),
            "help topic '{topic}' must not expose the MDX component import"
        );
        assert!(
            !sop.contains("<XmlPlaylistImportSteps />"),
            "help topic '{topic}' must not expose an unresolved MDX component tag"
        );
    }
}
