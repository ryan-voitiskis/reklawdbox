use std::collections::HashMap;

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::TagType;

use super::super::fields::*;
use super::super::model::*;
use super::super::mutation::*;
use super::super::read::*;
use super::support::*;

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
