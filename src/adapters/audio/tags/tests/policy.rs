use std::collections::HashMap;

use lofty::tag::{ItemKey, Tag, TagType};

use super::super::fields::*;
use super::super::model::*;
use super::super::mutation::*;
use super::super::read::resolve_fields;
use super::support::dry_run_test_patch;

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
