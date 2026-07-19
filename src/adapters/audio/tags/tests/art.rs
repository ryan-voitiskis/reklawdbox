use lofty::picture::PictureType;

use super::super::art::*;
use super::super::model::{FileEmbedResult, TagError};
use super::support::{cover_art_test_png, write_tag_test_wav};

#[test]
fn parse_picture_type_accepts_exact_documented_values() {
    let cases = [
        ("other", PictureType::Other),
        ("icon", PictureType::Icon),
        ("other_icon", PictureType::OtherIcon),
        ("front_cover", PictureType::CoverFront),
        ("cover_front", PictureType::CoverFront),
        ("back_cover", PictureType::CoverBack),
        ("cover_back", PictureType::CoverBack),
        ("leaflet", PictureType::Leaflet),
        ("media", PictureType::Media),
        ("lead_artist", PictureType::LeadArtist),
        ("artist", PictureType::Artist),
        ("conductor", PictureType::Conductor),
        ("band", PictureType::Band),
        ("composer", PictureType::Composer),
        ("lyricist", PictureType::Lyricist),
        ("recording_location", PictureType::RecordingLocation),
        ("during_recording", PictureType::DuringRecording),
        ("during_performance", PictureType::DuringPerformance),
        ("screen_capture", PictureType::ScreenCapture),
        ("bright_fish", PictureType::BrightFish),
        ("illustration", PictureType::Illustration),
        ("band_logo", PictureType::BandLogo),
        ("publisher_logo", PictureType::PublisherLogo),
    ];

    assert_eq!(
        cases.map(|(name, _)| name).as_slice(),
        ACCEPTED_PICTURE_TYPES
    );
    for (name, expected) in cases {
        assert_eq!(parse_picture_type(name).unwrap(), expected, "{name}");
    }
    assert_eq!(
        parse_picture_type("front_cover").unwrap(),
        parse_picture_type("cover_front").unwrap()
    );
    assert_eq!(
        parse_picture_type("back_cover").unwrap(),
        parse_picture_type("cover_back").unwrap()
    );
    assert_eq!(
        picture_type_name(parse_picture_type("bright_fish").unwrap()),
        "bright_fish"
    );
}

#[test]
fn parse_picture_type_rejects_unknown_unmodified_values() {
    for invalid in ["garbage", "", "Front_Cover", " front_cover "] {
        let error = parse_picture_type(invalid).unwrap_err();
        let TagError::Validation(message) = error else {
            panic!("invalid picture type should return validation: {error:?}");
        };
        assert!(message.contains(&format!("{invalid:?}")));
        assert!(message.contains("front_cover"));
        assert!(message.contains("back_cover"));
    }
}

#[test]
fn cover_art_invalid_picture_type_extract_precedes_io() {
    let dir = tempfile::tempdir().expect("temp directory should create");
    let missing_audio = dir.path().join("missing.wav");

    for invalid in ["garbage", "", "Front_Cover", " front_cover "] {
        let result = extract_cover_art(&missing_audio, None, invalid);
        assert!(
            matches!(result, Err(TagError::Validation(_))),
            "invalid picture type {invalid:?} should fail validation before audio I/O"
        );
    }
}

#[test]
fn cover_art_invalid_picture_type_embed_precedes_io() {
    let dir = tempfile::tempdir().expect("temp directory should create");
    let missing_image = dir.path().join("missing.png");
    let missing_audio = dir.path().join("missing.wav");

    for invalid in ["garbage", "", "Front_Cover", " front_cover "] {
        let result = embed_cover_art_inner(&missing_image, &missing_audio, invalid);
        assert!(
            matches!(result, Err(TagError::Validation(_))),
            "invalid picture type {invalid:?} should fail validation before image/audio I/O, got {result:?}"
        );
    }
}

#[test]
fn cover_art_valid_missing_type_falls_back_to_first_picture() {
    let dir = tempfile::tempdir().expect("temp directory should create");
    let image_path = dir.path().join("cover.png");
    let audio_path = dir.path().join("track.wav");
    let output_path = dir.path().join("extracted.png");
    let image = cover_art_test_png();
    std::fs::write(&image_path, &image).expect("synthetic PNG should write");
    write_tag_test_wav(&audio_path);

    assert!(matches!(
        embed_cover_art(&image_path, &audio_path, "front_cover"),
        FileEmbedResult::Ok { .. }
    ));
    let extracted = extract_cover_art(&audio_path, Some(&output_path), "back_cover")
        .expect("valid missing type should fall back to the first picture");

    assert_eq!(extracted.picture_type, "front_cover");
    assert_eq!(std::fs::read(output_path).unwrap(), image);
}
