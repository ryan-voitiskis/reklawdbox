use std::collections::HashMap;
use std::path::Path;

use lofty::file::TaggedFileExt;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

use super::super::fields::{ValidatedTagPatch, get_field_from_tag};
use super::super::model::{CommentMode, FileWriteResult, WavTarget, WriteEntry};
use super::super::mutation::{write_file_tags, write_file_tags_dry_run};
use super::super::read::parse_options;

pub(super) fn write_tag_test_wav(path: &Path) {
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

pub(super) fn write_tag_test_aiff(path: &Path) {
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

pub(super) fn write_test_wav_layer(
    path: &Path,
    target: WavTarget,
    tags: HashMap<String, Option<String>>,
) {
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

pub(super) fn wav_dry_run_json(
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

pub(super) fn dry_run_test_patch(tags: HashMap<String, Option<String>>) -> ValidatedTagPatch {
    ValidatedTagPatch::try_from(&tags).expect("synthetic patch should validate")
}

pub(super) fn cover_art_test_png() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 4,
        0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15, 0, 1, 5,
        1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}

pub(super) fn wav_layer_field(path: &Path, target: WavTarget, field: &str) -> Option<String> {
    let tagged_file = Probe::open(path)
        .expect("synthetic WAV should open")
        .options(parse_options(true))
        .read()
        .expect("synthetic WAV should read");
    tagged_file
        .tag(TagType::from(&target))
        .and_then(|tag| get_field_from_tag(tag, field))
}

pub(super) fn physical_tag_field(tag: &Tag, field: &str) -> Option<String> {
    get_field_from_tag(tag, field).or_else(|| {
        (field == "publisher")
            .then(|| tag.get_string(ItemKey::Publisher).map(str::to_string))
            .flatten()
    })
}

pub(super) fn physical_wav_layer_field(
    path: &Path,
    target: WavTarget,
    field: &str,
) -> Option<String> {
    let tagged_file = Probe::open(path)
        .expect("synthetic WAV should open")
        .options(parse_options(true))
        .read()
        .expect("synthetic WAV should read");
    tagged_file
        .tag(TagType::from(&target))
        .and_then(|tag| physical_tag_field(tag, field))
}
