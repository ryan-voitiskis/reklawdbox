use std::collections::HashMap;
use std::path::Path;

use lofty::file::TaggedFileExt;
use lofty::picture::PictureType;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

use super::art::*;
use super::fields::*;
use super::model::*;
use super::mutation::*;
use super::read::*;

fn write_tag_test_wav(path: &Path) {
    let data_size: u32 = 2;
    let file_size = 36 + data_size;
    let mut header = Vec::new();
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&file_size.to_le_bytes());
    header.extend_from_slice(b"WAVE");
    header.extend_from_slice(b"fmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&44_100u32.to_le_bytes());
    header.extend_from_slice(&88_200u32.to_le_bytes());
    header.extend_from_slice(&2u16.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_size.to_le_bytes());
    header.extend_from_slice(&[0u8; 2]);
    std::fs::write(path, header).expect("synthetic WAV should write");
}

fn write_tag_test_aiff(path: &Path) {
    let mut file = Vec::new();
    file.extend_from_slice(b"FORM");
    file.extend_from_slice(&48u32.to_be_bytes());
    file.extend_from_slice(b"AIFF");
    file.extend_from_slice(b"COMM");
    file.extend_from_slice(&18u32.to_be_bytes());
    file.extend_from_slice(&1u16.to_be_bytes());
    file.extend_from_slice(&1u32.to_be_bytes());
    file.extend_from_slice(&16u16.to_be_bytes());
    file.extend_from_slice(&[0x40, 0x0e, 0xac, 0x44, 0, 0, 0, 0, 0, 0]);
    file.extend_from_slice(b"SSND");
    file.extend_from_slice(&10u32.to_be_bytes());
    file.extend_from_slice(&0u32.to_be_bytes());
    file.extend_from_slice(&0u32.to_be_bytes());
    file.extend_from_slice(&[0u8; 2]);
    std::fs::write(path, file).expect("synthetic AIFF should write");
}

fn write_test_wav_layer(path: &Path, target: WavTarget, tags: HashMap<String, Option<String>>) {
    let result = write_file_tags(&WriteEntry {
        path: path.to_path_buf(),
        tags,
        wav_targets: vec![target],
        comment_mode: CommentMode::Replace,
    });
    assert!(
        matches!(result, FileWriteResult::Ok { .. }),
        "synthetic WAV layer should seed successfully: {result:?}"
    );
}

fn wav_dry_run_json(
    path: &Path,
    tags: HashMap<String, Option<String>>,
    wav_targets: Vec<WavTarget>,
    comment_mode: CommentMode,
) -> serde_json::Value {
    let result = write_file_tags_dry_run(&WriteEntry {
        path: path.to_path_buf(),
        tags,
        wav_targets,
        comment_mode,
    });
    serde_json::to_value(result).expect("dry-run result should serialize")
}

fn dry_run_test_patch(tags: HashMap<String, Option<String>>) -> ValidatedTagPatch {
    ValidatedTagPatch::try_from(&tags).expect("synthetic patch should validate")
}

fn cover_art_test_png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}

fn wav_layer_field(path: &Path, target: WavTarget, field: &str) -> Option<String> {
    let tagged_file = Probe::open(path)
        .expect("synthetic WAV should open")
        .options(parse_options(true))
        .read()
        .expect("synthetic WAV should read");
    tagged_file
        .tag(TagType::from(&target))
        .and_then(|tag| get_field_from_tag(tag, field))
}

fn physical_tag_field(tag: &Tag, field: &str) -> Option<String> {
    get_field_from_tag(tag, field).or_else(|| {
        (field == "publisher")
            .then(|| tag.get_string(ItemKey::Publisher).map(str::to_string))
            .flatten()
    })
}

fn physical_wav_layer_field(path: &Path, target: WavTarget, field: &str) -> Option<String> {
    let tagged_file = Probe::open(path)
        .expect("synthetic WAV should open")
        .options(parse_options(true))
        .read()
        .expect("synthetic WAV should read");
    tagged_file
        .tag(TagType::from(&target))
        .and_then(|tag| physical_tag_field(tag, field))
}

#[test]
fn field_to_key_roundtrip() {
    assert_eq!(
        TagField::ALL.map(TagField::as_str).as_slice(),
        ALL_FIELDS,
        "the exhaustive field enum must retain canonical order"
    );
    for field in TagField::ALL {
        let key = field.primary_item_key();
        let back =
            item_key_to_field(&key).unwrap_or_else(|| panic!("No field for ItemKey {key:?}"));
        assert_eq!(back, field.as_str(), "Roundtrip failed for {field:?}");
    }
}

#[test]
fn validated_patch_preserves_exact_validation_contract() {
    for (field, value, expected) in [
        ("unknown", Some("value"), "Unknown field \"unknown\""),
        ("unknown", None, "Unknown field \"unknown\""),
        ("unknown", Some(""), "Unknown field \"unknown\""),
        (
            "year",
            Some("24"),
            "Invalid year \"24\": must be 4-digit YYYY or null/empty to delete",
        ),
        (
            "track",
            Some("0"),
            "Invalid track \"0\": must be a positive integer or null/empty to delete",
        ),
        (
            "disc",
            Some("-1"),
            "Invalid disc \"-1\": must be a positive integer or null/empty to delete",
        ),
    ] {
        let tags = HashMap::from([(
            field.to_string(),
            value.map(std::string::ToString::to_string),
        )]);
        assert_eq!(
            ValidatedTagPatch::try_from(&tags).unwrap_err().to_string(),
            expected
        );
    }

    for delete in [None, Some(String::new())] {
        let patch = ValidatedTagPatch::try_from(&HashMap::from([("artist".to_string(), delete)]))
            .expect("delete patch should validate");
        assert_eq!(patch.edits[&TagField::Artist], TagEdit::Delete);
    }
    let whitespace = ValidatedTagPatch::try_from(&HashMap::from([(
        "artist".to_string(),
        Some("  untouched  ".to_string()),
    )]))
    .expect("free-form field should validate");
    assert_eq!(
        whitespace.edits[&TagField::Artist],
        TagEdit::Set("  untouched  ".to_string())
    );
}

#[test]
fn tags_all_fields_preview_and_write_share_layer_policy() {
    let values = [
        ("artist", "new artist"),
        ("title", "new title"),
        ("album", "new album"),
        ("album_artist", "new album artist"),
        ("genre", "new genre"),
        ("year", "2024"),
        ("track", "2"),
        ("disc", "2"),
        ("comment", "new comment"),
        ("publisher", "new publisher"),
        ("bpm", "128"),
        ("key", "Am"),
        ("composer", "new composer"),
        ("remixer", "new remixer"),
    ];

    for (target, layer_name) in [
        (WavTarget::Id3v2, "id3v2"),
        (WavTarget::RiffInfo, "riff_info"),
    ] {
        for (field, value) in values {
            let dir = tempfile::tempdir().expect("temp directory should create");
            let path = dir.path().join(format!("{layer_name}-{field}.wav"));
            write_tag_test_wav(&path);
            let entry = WriteEntry {
                path: path.clone(),
                tags: HashMap::from([(field.to_string(), Some(value.to_string()))]),
                wav_targets: vec![target.clone()],
                comment_mode: CommentMode::Replace,
            };

            let preview = write_file_tags_dry_run(&entry);
            let FileDryRunResult::Preview {
                changes_by_layer: Some(changes_by_layer),
                ..
            } = preview
            else {
                panic!("{layer_name} {field} preview should succeed: {preview:?}");
            };
            let previewed = changes_by_layer[layer_name].get(field);
            let supported = target
                .capability()
                .supports(TagField::parse(field).expect("table field should be canonical"));
            assert_eq!(
                previewed.is_some(),
                supported,
                "{layer_name} {field} preview capability drifted"
            );
            if let Some(change) = previewed {
                assert_eq!(change.old, None, "{layer_name} {field}");
                assert_eq!(change.new.as_deref(), Some(value), "{layer_name} {field}");
            }

            let result = write_file_tags(&entry);
            let FileWriteResult::Ok { fields_written, .. } = result else {
                panic!("{layer_name} {field} write should succeed: {result:?}");
            };
            assert_eq!(
                fields_written,
                if supported {
                    vec![field.to_string()]
                } else {
                    Vec::new()
                },
                "{layer_name} {field} write capability drifted"
            );
            assert_eq!(
                physical_wav_layer_field(&path, target.clone(), field).as_deref(),
                supported.then_some(value),
                "{layer_name} {field} write must match preview"
            );
        }
    }

    for (field, value) in values {
        let dir = tempfile::tempdir().expect("temp directory should create");
        let path = dir.path().join(format!("primary-{field}.aiff"));
        write_tag_test_aiff(&path);
        let entry = WriteEntry {
            path: path.clone(),
            tags: HashMap::from([(field.to_string(), Some(value.to_string()))]),
            wav_targets: vec![],
            comment_mode: CommentMode::Replace,
        };

        let preview = write_file_tags_dry_run(&entry);
        let FileDryRunResult::Preview {
            changes,
            changes_by_layer: None,
            ..
        } = preview
        else {
            panic!("primary {field} preview should succeed: {preview:?}");
        };
        assert_eq!(changes[field].new.as_deref(), Some(value), "{field}");

        let result = write_file_tags(&entry);
        assert!(
            matches!(result, FileWriteResult::Ok { .. }),
            "primary {field} write should succeed: {result:?}"
        );
        let tagged_file = Probe::open(&path)
            .expect("synthetic AIFF should open")
            .options(parse_options(true))
            .read()
            .expect("synthetic AIFF should read");
        let tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
            .expect("written AIFF tag should exist");
        assert_eq!(
            physical_tag_field(tag, field).as_deref(),
            Some(value),
            "{field}"
        );
    }
}

#[test]
fn tags_id3_publisher_keeps_legacy_public_read_and_preview_behavior() {
    let dir = tempfile::tempdir().expect("temp directory should create");
    let path = dir.path().join("publisher-contract.wav");
    write_tag_test_wav(&path);
    let entry = WriteEntry {
        path: path.clone(),
        tags: HashMap::from([("publisher".to_string(), Some("new publisher".to_string()))]),
        wav_targets: vec![WavTarget::Id3v2],
        comment_mode: CommentMode::Replace,
    };

    assert!(matches!(
        write_file_tags(&entry),
        FileWriteResult::Ok { .. }
    ));
    assert_eq!(
        physical_wav_layer_field(&path, WavTarget::Id3v2, "publisher").as_deref(),
        Some("new publisher"),
        "the TPUB frame should remain physically present"
    );
    let fields = ["publisher".to_string()];
    let FileReadResult::Wav { id3v2, .. } = read_file_tags(&path, Some(&fields), false) else {
        panic!("synthetic WAV should remain readable");
    };
    assert_eq!(
        id3v2["publisher"], None,
        "Lofty normalizes TPUB from Label to Publisher on read; the public reader historically checks Label"
    );

    let repeated_preview = write_file_tags_dry_run(&entry);
    let FileDryRunResult::Preview {
        changes_by_layer: Some(changes_by_layer),
        ..
    } = repeated_preview
    else {
        panic!("repeated publisher preview should succeed: {repeated_preview:?}");
    };
    assert_eq!(
        changes_by_layer["id3v2"]["publisher"],
        DryRunChange {
            old: None,
            new: Some("new publisher".to_string()),
        },
        "the repeated preview must retain legacy public behavior"
    );
}

#[test]
fn tags_vorbis_plan_applier_avoids_secondary_year_and_bpm_items() {
    let tags = HashMap::from([
        ("year".to_string(), Some("2024".to_string())),
        ("bpm".to_string(), Some("128".to_string())),
    ]);
    let patch = ValidatedTagPatch::try_from(&tags).expect("patch should validate");
    let existing = ExistingLayerValues::read(None, &patch);
    let plan = plan_layer_mutation(
        &patch,
        LayerCapability::Primary,
        &existing,
        CommentMode::Replace,
    );
    let mut tag = Tag::new(TagType::VorbisComments);
    let mut fields_written = Vec::new();
    let mut fields_deleted = Vec::new();

    apply_layer_mutation_plan(
        &mut tag,
        TagType::VorbisComments,
        &plan,
        &mut fields_written,
        &mut fields_deleted,
    );

    assert_eq!(
        tag.get_string(ItemKey::RecordingDate),
        Some("2024"),
        "Vorbis should use DATE"
    );
    assert_eq!(
        tag.get_string(ItemKey::IntegerBpm),
        None,
        "Lofty rejects IntegerBpm for Vorbis; preserve the existing write behavior"
    );
    assert!(!tag.items().any(|item| item.key() == ItemKey::Year));
    assert!(!tag.items().any(|item| item.key() == ItemKey::Bpm));
    assert!(!tag.items().any(|item| item.key() == ItemKey::IntegerBpm));
    assert_eq!(fields_written, ["year", "bpm"]);
    assert!(fields_deleted.is_empty());
}

#[test]
fn tags_preview_matches_write_for_representative_mutations() {
    struct Case {
        name: &'static str,
        field: &'static str,
        initial: &'static str,
        edit: Option<&'static str>,
        comment_mode: CommentMode,
        expected: Option<&'static str>,
        previewed: bool,
    }

    for case in [
        Case {
            name: "set",
            field: "artist",
            initial: "old",
            edit: Some("new"),
            comment_mode: CommentMode::Replace,
            expected: Some("new"),
            previewed: true,
        },
        Case {
            name: "delete",
            field: "artist",
            initial: "old",
            edit: None,
            comment_mode: CommentMode::Replace,
            expected: None,
            previewed: true,
        },
        Case {
            name: "no-op",
            field: "artist",
            initial: "same",
            edit: Some("same"),
            comment_mode: CommentMode::Replace,
            expected: Some("same"),
            previewed: false,
        },
        Case {
            name: "prepend",
            field: "comment",
            initial: "old",
            edit: Some("new"),
            comment_mode: CommentMode::Prepend,
            expected: Some("new | old"),
            previewed: true,
        },
        Case {
            name: "append",
            field: "comment",
            initial: "old",
            edit: Some("new"),
            comment_mode: CommentMode::Append,
            expected: Some("old | new"),
            previewed: true,
        },
    ] {
        let dir = tempfile::tempdir().expect("temp directory should create");
        let path = dir.path().join(format!("{}.wav", case.name));
        write_tag_test_wav(&path);
        write_test_wav_layer(
            &path,
            WavTarget::Id3v2,
            HashMap::from([(case.field.to_string(), Some(case.initial.to_string()))]),
        );
        let entry = WriteEntry {
            path: path.clone(),
            tags: HashMap::from([(case.field.to_string(), case.edit.map(str::to_string))]),
            wav_targets: vec![WavTarget::Id3v2],
            comment_mode: case.comment_mode,
        };

        let preview = write_file_tags_dry_run(&entry);
        let FileDryRunResult::Preview { changes, .. } = preview else {
            panic!("{} preview should succeed: {preview:?}", case.name);
        };
        assert_eq!(
            changes.contains_key(case.field),
            case.previewed,
            "{} preview no-op policy drifted",
            case.name
        );
        if let Some(change) = changes.get(case.field) {
            assert_eq!(change.old.as_deref(), Some(case.initial), "{}", case.name);
            assert_eq!(change.new.as_deref(), case.expected, "{}", case.name);
        }

        let result = write_file_tags(&entry);
        assert!(
            matches!(result, FileWriteResult::Ok { .. }),
            "{} write should succeed: {result:?}",
            case.name
        );
        assert_eq!(
            wav_layer_field(&path, WavTarget::Id3v2, case.field).as_deref(),
            case.expected,
            "{} write must match preview",
            case.name
        );
    }
}

#[test]
fn tags_dual_layer_failure_preserves_original_and_cleans_temp_copy() {
    let dir = tempfile::tempdir().expect("temp directory should create");
    let path = dir.path().join("atomic-failure.wav");
    write_tag_test_wav(&path);
    let original = std::fs::read(&path).expect("synthetic WAV should read");
    let entry = WriteEntry {
        path: path.clone(),
        tags: HashMap::from([("artist".to_string(), Some("new".to_string()))]),
        wav_targets: vec![WavTarget::Id3v2, WavTarget::RiffInfo],
        comment_mode: CommentMode::Replace,
    };

    let result = with_test_tag_layer_write_failure(TagType::RiffInfo, || write_file_tags(&entry));
    let FileWriteResult::Error { error, .. } = result else {
        panic!("injected second-layer failure should be reported: {result:?}");
    };
    assert_eq!(error, "injected tag-layer write failure");
    assert_eq!(
        std::fs::read(&path).expect("original WAV should remain readable"),
        original
    );
    let leftovers = std::fs::read_dir(dir.path())
        .expect("temp directory should read")
        .map(|entry| {
            entry
                .expect("temp directory entry should read")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains(".rklw-"))
        .collect::<Vec<_>>();
    assert!(leftovers.is_empty(), "temp copies leaked: {leftovers:?}");
}

#[test]
fn dry_run_layer_diff_reports_deletion() {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::TrackArtist, "old".to_string());

    for deletion in [None, Some(String::new())] {
        let patch = dry_run_test_patch(HashMap::from([("artist".to_string(), deletion)]));
        assert_eq!(
            dry_run_layer_diff(
                &patch,
                Some(&tag),
                LayerCapability::Id3v2,
                CommentMode::Replace,
            )
            .get("artist"),
            Some(&DryRunChange {
                old: Some("old".to_string()),
                new: None,
            })
        );
    }
}

#[test]
fn dry_run_layer_diff_omits_equal_values() {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::TrackArtist, "same".to_string());
    let patch = dry_run_test_patch(HashMap::from([(
        "artist".to_string(),
        Some("same".to_string()),
    )]));

    assert!(
        dry_run_layer_diff(
            &patch,
            Some(&tag),
            LayerCapability::Id3v2,
            CommentMode::Replace,
        )
        .is_empty()
    );
}

#[test]
fn dry_run_layer_diff_applies_comment_modes() {
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::Comment, "old".to_string());

    for (mode, expected) in [
        (CommentMode::Replace, "new"),
        (CommentMode::Prepend, "new | old"),
        (CommentMode::Append, "old | new"),
    ] {
        let patch = dry_run_test_patch(HashMap::from([(
            "comment".to_string(),
            Some("new".to_string()),
        )]));
        assert_eq!(
            dry_run_layer_diff(&patch, Some(&tag), LayerCapability::Id3v2, mode)["comment"]
                .new
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn dry_run_layer_diff_handles_missing_tag() {
    let patch = dry_run_test_patch(HashMap::from([(
        "artist".to_string(),
        Some("new".to_string()),
    )]));

    assert_eq!(
        dry_run_layer_diff(&patch, None, LayerCapability::Id3v2, CommentMode::Replace,)
            .get("artist"),
        Some(&DryRunChange {
            old: None,
            new: Some("new".to_string()),
        })
    );
}

#[test]
fn dry_run_layer_diff_filters_riff_unsupported_fields() {
    let patch = dry_run_test_patch(HashMap::from([
        ("artist".to_string(), Some("new artist".to_string())),
        ("key".to_string(), Some("Am".to_string())),
    ]));

    let changes = dry_run_layer_diff(
        &patch,
        None,
        LayerCapability::RiffInfo,
        CommentMode::Replace,
    );
    assert!(changes.contains_key("artist"));
    assert!(!changes.contains_key("key"));
}

#[test]
fn riff_info_field_set() {
    assert!(is_riff_info_field("artist"));
    assert!(is_riff_info_field("title"));
    assert!(is_riff_info_field("album"));
    assert!(is_riff_info_field("genre"));
    assert!(is_riff_info_field("year"));
    assert!(is_riff_info_field("comment"));
    assert!(!is_riff_info_field("bpm"));
    assert!(!is_riff_info_field("key"));
    assert!(!is_riff_info_field("remixer"));
    assert!(!is_riff_info_field("track"));
}

#[test]
fn validate_year_valid() {
    let mut tags = HashMap::new();
    tags.insert("year".to_string(), Some("2024".to_string()));
    validate_write_tags(&tags).unwrap();
}

#[test]
fn validate_year_delete() {
    let mut tags = HashMap::new();
    tags.insert("year".to_string(), None);
    validate_write_tags(&tags).unwrap();

    let mut tags2 = HashMap::new();
    tags2.insert("year".to_string(), Some("".to_string()));
    validate_write_tags(&tags2).unwrap();
}

#[test]
fn validate_year_invalid() {
    let mut tags = HashMap::new();
    tags.insert("year".to_string(), Some("20".to_string()));
    assert!(validate_write_tags(&tags).is_err());

    let mut tags2 = HashMap::new();
    tags2.insert("year".to_string(), Some("abcd".to_string()));
    assert!(validate_write_tags(&tags2).is_err());

    let mut tags3 = HashMap::new();
    tags3.insert("year".to_string(), Some("20240".to_string()));
    assert!(validate_write_tags(&tags3).is_err());
}

#[test]
fn validate_track_valid() {
    let mut tags = HashMap::new();
    tags.insert("track".to_string(), Some("1".to_string()));
    validate_write_tags(&tags).unwrap();

    let mut tags2 = HashMap::new();
    tags2.insert("track".to_string(), Some("99".to_string()));
    validate_write_tags(&tags2).unwrap();
}

#[test]
fn validate_track_invalid() {
    let mut tags = HashMap::new();
    tags.insert("track".to_string(), Some("0".to_string()));
    assert!(validate_write_tags(&tags).is_err());

    let mut tags2 = HashMap::new();
    tags2.insert("track".to_string(), Some("-1".to_string()));
    assert!(validate_write_tags(&tags2).is_err());

    let mut tags3 = HashMap::new();
    tags3.insert("track".to_string(), Some("1/12".to_string()));
    assert!(validate_write_tags(&tags3).is_err());
}

#[test]
fn validate_disc_valid() {
    let mut tags = HashMap::new();
    tags.insert("disc".to_string(), Some("1".to_string()));
    validate_write_tags(&tags).unwrap();
}

#[test]
fn validate_unknown_field_rejected() {
    let mut tags = HashMap::new();
    tags.insert("nonexistent".to_string(), Some("value".to_string()));
    assert!(validate_write_tags(&tags).is_err());
}

#[test]
fn validate_unknown_field_null_rejected() {
    let mut tags = HashMap::new();
    tags.insert("bogus_field".to_string(), None);
    assert!(validate_write_tags(&tags).is_err());
}

#[test]
fn validate_unknown_field_empty_rejected() {
    let mut tags = HashMap::new();
    tags.insert("bogus_field".to_string(), Some("".to_string()));
    assert!(validate_write_tags(&tags).is_err());
}

#[test]
fn validate_freeform_fields_accepted() {
    let mut tags = HashMap::new();
    tags.insert("artist".to_string(), Some("Burial".to_string()));
    tags.insert("title".to_string(), Some("Archangel".to_string()));
    tags.insert("bpm".to_string(), Some("130".to_string()));
    tags.insert("key".to_string(), Some("Am".to_string()));
    validate_write_tags(&tags).unwrap();
}

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

#[test]
fn resolve_fields_all() {
    let result = resolve_fields(None);
    assert_eq!(result.len(), ALL_FIELDS.len());
}

#[test]
fn resolve_fields_filtered() {
    let filter = vec!["artist".to_string(), "title".to_string()];
    let result = resolve_fields(Some(&filter));
    assert_eq!(result, vec!["artist", "title"]);
}

#[test]
fn dry_run_riff_only_excludes_unsupported_fields() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("test.wav");
    write_tag_test_wav(&wav_path);

    let entry = WriteEntry {
        path: wav_path,
        tags: HashMap::from([
            ("key".to_string(), Some("Am".to_string())),
            ("bpm".to_string(), Some("128".to_string())),
            ("remixer".to_string(), Some("Someone".to_string())),
        ]),
        wav_targets: vec![WavTarget::RiffInfo],
        comment_mode: CommentMode::default(),
    };

    let result = write_file_tags_dry_run(&entry);
    match result {
        FileDryRunResult::Preview { changes, .. } => {
            // key, bpm, and remixer are NOT RIFF INFO fields, so they
            // should be excluded from the diff entirely.
            assert!(
                !changes.contains_key("key"),
                "key should be excluded from RIFF-only dry-run"
            );
            assert!(
                !changes.contains_key("bpm"),
                "bpm should be excluded from RIFF-only dry-run"
            );
            assert!(
                !changes.contains_key("remixer"),
                "remixer should be excluded from RIFF-only dry-run"
            );
        }
        FileDryRunResult::Error { error, .. } => {
            panic!("dry-run should succeed, got error: {error}");
        }
    }
}

#[test]
fn wav_dry_run_default_both_reports_distinct_layer_values() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("distinct-artist.wav");
    write_tag_test_wav(&wav_path);
    write_test_wav_layer(
        &wav_path,
        WavTarget::Id3v2,
        HashMap::from([("artist".to_string(), Some("ID3 old".to_string()))]),
    );
    write_test_wav_layer(
        &wav_path,
        WavTarget::RiffInfo,
        HashMap::from([("artist".to_string(), Some("RIFF old".to_string()))]),
    );

    let preview = wav_dry_run_json(
        &wav_path,
        HashMap::from([("artist".to_string(), Some("new".to_string()))]),
        vec![],
        CommentMode::Replace,
    );

    assert_eq!(
        preview["changes_by_layer"]["id3v2"]["artist"]["old"],
        "ID3 old"
    );
    assert_eq!(
        preview["changes_by_layer"]["riff_info"]["artist"]["old"],
        "RIFF old"
    );
    assert_eq!(preview["changes"], preview["changes_by_layer"]["id3v2"]);
    assert_eq!(
        preview["wav_targets"],
        serde_json::json!(["id3v2", "riff_info"])
    );
}

#[test]
fn wav_dry_run_comment_modes_merge_each_layer_independently() {
    for (mode, id3_new, riff_new) in [
        (CommentMode::Append, "ID3 old | new", "RIFF old | new"),
        (CommentMode::Prepend, "new | ID3 old", "new | RIFF old"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("distinct-comments.wav");
        write_tag_test_wav(&wav_path);
        write_test_wav_layer(
            &wav_path,
            WavTarget::Id3v2,
            HashMap::from([("comment".to_string(), Some("ID3 old".to_string()))]),
        );
        write_test_wav_layer(
            &wav_path,
            WavTarget::RiffInfo,
            HashMap::from([("comment".to_string(), Some("RIFF old".to_string()))]),
        );

        let preview = wav_dry_run_json(
            &wav_path,
            HashMap::from([("comment".to_string(), Some("new".to_string()))]),
            vec![],
            mode,
        );

        assert_eq!(
            preview["changes_by_layer"]["id3v2"]["comment"]["new"],
            id3_new
        );
        assert_eq!(
            preview["changes_by_layer"]["riff_info"]["comment"]["new"],
            riff_new
        );
    }
}

#[test]
fn wav_dry_run_id3_only_field_appears_only_in_id3v2() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("id3-only-field.wav");
    write_tag_test_wav(&wav_path);

    let preview = wav_dry_run_json(
        &wav_path,
        HashMap::from([("key".to_string(), Some("Am".to_string()))]),
        vec![],
        CommentMode::Replace,
    );

    assert!(preview["changes_by_layer"]["id3v2"].get("key").is_some());
    assert!(
        preview["changes_by_layer"]["riff_info"]
            .get("key")
            .is_none()
    );
}

#[test]
fn wav_dry_run_single_target_contains_exactly_requested_layer() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("single-target.wav");
    write_tag_test_wav(&wav_path);

    for (target, key) in [
        (WavTarget::Id3v2, "id3v2"),
        (WavTarget::RiffInfo, "riff_info"),
    ] {
        let preview = wav_dry_run_json(
            &wav_path,
            HashMap::from([("artist".to_string(), Some("new".to_string()))]),
            vec![target],
            CommentMode::Replace,
        );
        let layers = preview["changes_by_layer"]
            .as_object()
            .expect("WAV preview should contain layer maps");
        assert_eq!(layers.keys().collect::<Vec<_>>(), vec![key]);
        assert_eq!(preview["changes"], preview["changes_by_layer"][key]);
    }
}

#[test]
fn wav_dry_run_noop_includes_empty_requested_layers() {
    let dir = tempfile::tempdir().unwrap();
    let wav_path = dir.path().join("no-op.wav");
    write_tag_test_wav(&wav_path);
    for target in [WavTarget::Id3v2, WavTarget::RiffInfo] {
        write_test_wav_layer(
            &wav_path,
            target,
            HashMap::from([("artist".to_string(), Some("same".to_string()))]),
        );
    }

    let preview = wav_dry_run_json(
        &wav_path,
        HashMap::from([("artist".to_string(), Some("same".to_string()))]),
        vec![],
        CommentMode::Replace,
    );

    assert_eq!(
        preview["changes_by_layer"],
        serde_json::json!({
            "id3v2": {},
            "riff_info": {},
        })
    );
    assert_eq!(preview["changes"], serde_json::json!({}));
}

#[test]
fn wav_dry_run_non_wav_omits_layer_map_and_preserves_legacy_changes() {
    let dir = tempfile::tempdir().unwrap();
    let aiff_path = dir.path().join("single-layer.aiff");
    write_tag_test_aiff(&aiff_path);

    let result = write_file_tags_dry_run(&WriteEntry {
        path: aiff_path,
        tags: HashMap::from([("artist".to_string(), Some("new".to_string()))]),
        wav_targets: vec![WavTarget::RiffInfo],
        comment_mode: CommentMode::Replace,
    });
    let preview = serde_json::to_value(result).expect("dry-run result should serialize");

    assert!(preview.get("changes_by_layer").is_none());
    assert_eq!(
        preview["changes"],
        serde_json::json!({"artist": {"old": null, "new": "new"}})
    );
    assert!(preview.get("wav_targets").is_none());
}

#[test]
fn merge_comment_replace() {
    assert_eq!(
        merge_comment("new", Some("old"), CommentMode::Replace),
        "new"
    );
}

#[test]
fn merge_comment_prepend() {
    assert_eq!(
        merge_comment("new", Some("old"), CommentMode::Prepend),
        "new | old"
    );
}

#[test]
fn merge_comment_append() {
    assert_eq!(
        merge_comment("new", Some("old"), CommentMode::Append),
        "old | new"
    );
}

#[test]
fn merge_comment_prepend_empty_existing() {
    assert_eq!(merge_comment("new", Some(""), CommentMode::Prepend), "new");
    assert_eq!(merge_comment("new", None, CommentMode::Prepend), "new");
}

#[test]
fn merge_comment_append_empty_existing() {
    assert_eq!(merge_comment("new", Some(""), CommentMode::Append), "new");
    assert_eq!(merge_comment("new", None, CommentMode::Append), "new");
}
