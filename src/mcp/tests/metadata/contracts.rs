use crate::mcp::metadata::{TrackChangeInput, UpdateTracksParams, handle_update_tracks};

use crate::domain::metadata::ChangeManager;

use super::super::common::make_test_track;

#[test]
fn color_input_public_contract() {
    let changes = ChangeManager::new();
    handle_update_tracks(
        &changes,
        UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "color-name".to_owned(),
                genre: None,
                comments: None,
                rating: None,
                color: Some("turquoise".to_owned()),
                label: None,
                year: None,
                album: None,
            }],
        },
    )
    .expect("a canonical color name should validate without a database");
    assert_eq!(
        changes
            .get("color-name")
            .expect("accepted color should be staged")
            .color
            .as_deref(),
        Some("Turquoise"),
        "accepted color names should be canonicalized"
    );

    let error = handle_update_tracks(
        &ChangeManager::new(),
        UpdateTracksParams {
            changes: vec![TrackChangeInput {
                track_id: "color-hex".to_owned(),
                genre: None,
                comments: None,
                rating: None,
                color: Some("0x25FDE9".to_owned()),
                label: None,
                year: None,
                album: None,
            }],
        },
    )
    .expect_err("serialized XML hex must not be accepted as a tool input");
    let message = format!("{error:?}");
    for name in [
        "Blue",
        "Green",
        "Lemon",
        "Orange",
        "Red",
        "Rose",
        "Turquoise",
        "Violet",
    ] {
        assert!(
            message.contains(name),
            "invalid color guidance should include {name}: {message}"
        );
    }

    let mut track = make_test_track("color-xml", "House", 124.0, "8A");
    track.color_code = crate::domain::metadata::color_name_to_code("Turquoise")
        .expect("canonical color should have an XML code");
    let xml = crate::adapters::rekordbox::xml::generate_xml(&[track]);
    assert!(
        xml.contains("Colour=\"0x25FDE9\""),
        "canonical color integers should serialize as uppercase 0xRRGGBB"
    );
}
