//! Audio-tag and cover-art metadata reads.

use std::collections::HashMap;
use std::path::Path;

use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{MimeType, PictureType};
use lofty::probe::Probe;
use lofty::tag::{Tag, TagType};

use super::fields::{ALL_FIELDS, get_field_from_tag, is_riff_info_field};
use super::model::{CoverArtMeta, FileReadResult};

pub(super) fn parse_options(read_cover_art: bool) -> ParseOptions {
    ParseOptions::new()
        .read_cover_art(read_cover_art)
        .parsing_mode(ParsingMode::BestAttempt)
}

fn file_type_name(ft: FileType) -> &'static str {
    match ft {
        FileType::Wav => "wav",
        FileType::Flac => "flac",
        FileType::Mpeg => "mp3",
        FileType::Mp4 => "m4a",
        FileType::Aiff => "aiff",
        FileType::Aac => "aac",
        FileType::Ape => "ape",
        FileType::Opus => "opus",
        FileType::Vorbis => "vorbis",
        FileType::Speex => "speex",
        FileType::WavPack => "wavpack",
        FileType::Mpc => "mpc",
        _ => "unknown",
    }
}

fn tag_type_name(tt: TagType) -> &'static str {
    match tt {
        TagType::Id3v2 => "id3v2",
        TagType::Id3v1 => "id3v1",
        TagType::VorbisComments => "vorbis_comment",
        TagType::Mp4Ilst => "ilst",
        TagType::RiffInfo => "riff_info",
        TagType::Ape => "ape",
        TagType::AiffText => "aiff_text",
        _ => "unknown",
    }
}

fn read_tag_fields(tag: &Tag, fields: &[&str]) -> HashMap<String, Option<String>> {
    let mut map = HashMap::with_capacity(fields.len());
    for &field in fields {
        let value = get_field_from_tag(tag, field);
        map.insert(field.to_string(), value);
    }
    map
}

/// Read cover art metadata from a tag (format + size, no binary data).
fn read_cover_art_meta(tag: &Tag) -> Option<CoverArtMeta> {
    // Prefer CoverFront, fall back to first picture
    let pic = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| tag.pictures().first())?;

    let format = match pic.mime_type() {
        Some(MimeType::Jpeg) => "jpeg".to_string(),
        Some(MimeType::Png) => "png".to_string(),
        Some(MimeType::Tiff) => "tiff".to_string(),
        Some(MimeType::Bmp) => "bmp".to_string(),
        Some(MimeType::Gif) => "gif".to_string(),
        Some(MimeType::Unknown(s)) => s.clone(),
        Some(_) => "unknown".to_string(),
        None => "unknown".to_string(),
    };

    Some(CoverArtMeta {
        format,
        size_bytes: pic.data().len(),
    })
}

pub(super) fn resolve_fields(filter: Option<&[String]>) -> Vec<&str> {
    match filter {
        Some(f) => f.iter().map(std::string::String::as_str).collect(),
        None => ALL_FIELDS.to_vec(),
    }
}

// ---------------------------------------------------------------------------
// 1. read_file_tags
// ---------------------------------------------------------------------------

/// Read tags from a single audio file.
///
/// - `fields`: optional filter — only return these canonical fields.
/// - `include_cover_art`: include cover art metadata (format, size).
pub fn read_file_tags(
    path: &Path,
    fields: Option<&[String]>,
    include_cover_art: bool,
) -> FileReadResult {
    let path_str = path.display().to_string();

    let tagged_file = match Probe::open(path).map_err(|e| e.to_string()).and_then(|p| {
        p.options(parse_options(include_cover_art))
            .read()
            .map_err(|e| e.to_string())
    }) {
        Ok(f) => f,
        Err(e) => {
            return FileReadResult::Error {
                path: path_str,
                error: e,
            };
        }
    };

    let file_type = tagged_file.file_type();
    let fmt = file_type_name(file_type);
    let fields_list = resolve_fields(fields);

    match file_type {
        FileType::Wav => read_wav_tags(
            &tagged_file,
            &path_str,
            fmt,
            &fields_list,
            include_cover_art,
        ),
        _ => read_single_tags(
            &tagged_file,
            &path_str,
            fmt,
            &fields_list,
            include_cover_art,
        ),
    }
}

fn read_wav_tags(
    tagged_file: &lofty::file::TaggedFile,
    path: &str,
    fmt: &str,
    fields: &[&str],
    include_cover_art: bool,
) -> FileReadResult {
    let id3v2_tag = tagged_file.tag(TagType::Id3v2);
    let riff_tag = tagged_file.tag(TagType::RiffInfo);

    let id3v2 = match id3v2_tag {
        Some(tag) => read_tag_fields(tag, fields),
        None => fields.iter().map(|&f| (f.to_string(), None)).collect(),
    };

    // For RIFF INFO, only read fields that are available in RIFF INFO.
    // For unavailable fields, return None.
    let riff_info: HashMap<String, Option<String>> = fields
        .iter()
        .map(|&field| {
            let value = if is_riff_info_field(field) {
                riff_tag.and_then(|tag| get_field_from_tag(tag, field))
            } else {
                None
            };
            (field.to_string(), value)
        })
        .collect();

    // tag3_missing: fields that have a non-null value in id3v2 but are null
    // in riff_info. Only consider fields that are valid for RIFF INFO.
    let tag3_missing: Vec<String> = fields
        .iter()
        .filter(|&&field| {
            is_riff_info_field(field)
                && id3v2.get(field).is_some_and(std::option::Option::is_some)
                && riff_info
                    .get(field)
                    .is_some_and(std::option::Option::is_none)
        })
        .map(std::string::ToString::to_string)
        .collect();

    let cover_art = if include_cover_art {
        id3v2_tag.and_then(read_cover_art_meta)
    } else {
        None
    };

    FileReadResult::Wav {
        path: path.to_string(),
        format: fmt.to_string(),
        id3v2,
        riff_info,
        tag3_missing,
        cover_art,
    }
}

fn read_single_tags(
    tagged_file: &lofty::file::TaggedFile,
    path: &str,
    fmt: &str,
    fields: &[&str],
    include_cover_art: bool,
) -> FileReadResult {
    let tag = tagged_file
        .primary_tag()
        .or_else(|| tagged_file.first_tag());

    let (tag_type_str, tags, cover_art) = match tag {
        Some(t) => {
            let tag_type_str = tag_type_name(t.tag_type());
            let tags_map = read_tag_fields(t, fields);
            let cover_art_meta = if include_cover_art {
                read_cover_art_meta(t)
            } else {
                None
            };
            (tag_type_str, tags_map, cover_art_meta)
        }
        None => {
            // No tags at all — return all fields as None
            let empty: HashMap<String, Option<String>> =
                fields.iter().map(|&f| (f.to_string(), None)).collect();
            ("none", empty, None)
        }
    };

    FileReadResult::Single {
        path: path.to_string(),
        format: fmt.to_string(),
        tag_type: tag_type_str.to_string(),
        tags,
        cover_art,
    }
}
