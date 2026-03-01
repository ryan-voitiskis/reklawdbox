//! Core tag reading/writing module using `lofty`.
//!
//! Pure functions with NO MCP dependency. Called by both MCP tool wrappers
//! and CLI subcommands. All functions are synchronous — callers use
//! `spawn_blocking` for async contexts.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use lofty::config::{ParseOptions, ParsingMode, WriteOptions};
use lofty::file::{FileType, TaggedFileExt};
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum TagError {
    /// lofty open/read/write failures.
    #[error("{0}")]
    Io(String),
    /// Validation failures (unknown field, invalid year/track/disc).
    #[error("{0}")]
    Validation(String),
    /// No cover art found in file.
    #[error("No cover art found in file")]
    NoPicture,
    /// No tags found in file.
    #[error("No tags found in file")]
    NoTags,
    /// File doesn't support requested tag type.
    #[error("{0}")]
    Unsupported(String),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// All 14 canonical field names, in a stable order.
pub const ALL_FIELDS: &[&str] = &[
    "artist",
    "title",
    "album",
    "album_artist",
    "genre",
    "year",
    "track",
    "disc",
    "comment",
    "publisher",
    "bpm",
    "key",
    "composer",
    "remixer",
];

/// Fields that RIFF INFO supports. Other fields are silently skipped on
/// write and return `None` on read.
const RIFF_INFO_FIELDS: &[&str] = &["artist", "title", "album", "genre", "year", "comment"];

// ---------------------------------------------------------------------------
// Field ↔ ItemKey mapping
// ---------------------------------------------------------------------------

/// Map a canonical field name to the primary `ItemKey` used for generic `Tag`
/// reads/writes.
///
/// For fields with format-specific split keys (bpm, year) the caller may
/// need to fall through to secondary keys — see `get_field_from_tag`.
pub fn field_to_item_key(field: &str) -> Option<ItemKey> {
    match field {
        "artist" => Some(ItemKey::TrackArtist),
        "title" => Some(ItemKey::TrackTitle),
        "album" => Some(ItemKey::AlbumTitle),
        "album_artist" => Some(ItemKey::AlbumArtist),
        "genre" => Some(ItemKey::Genre),
        "year" => Some(ItemKey::RecordingDate),
        "track" => Some(ItemKey::TrackNumber),
        "disc" => Some(ItemKey::DiscNumber),
        "comment" => Some(ItemKey::Comment),
        "publisher" => Some(ItemKey::Label),
        "bpm" => Some(ItemKey::IntegerBpm),
        "key" => Some(ItemKey::InitialKey),
        "composer" => Some(ItemKey::Composer),
        "remixer" => Some(ItemKey::Remixer),
        _ => None,
    }
}

/// Reverse mapping from `ItemKey` to canonical field name (test-only).
#[cfg(test)]
fn item_key_to_field(key: &ItemKey) -> Option<&'static str> {
    match *key {
        ItemKey::TrackArtist => Some("artist"),
        ItemKey::TrackTitle => Some("title"),
        ItemKey::AlbumTitle => Some("album"),
        ItemKey::AlbumArtist => Some("album_artist"),
        ItemKey::Genre => Some("genre"),
        ItemKey::RecordingDate => Some("year"),
        ItemKey::Year => Some("year"),
        ItemKey::TrackNumber => Some("track"),
        ItemKey::DiscNumber => Some("disc"),
        ItemKey::Comment => Some("comment"),
        ItemKey::Label => Some("publisher"),
        ItemKey::IntegerBpm => Some("bpm"),
        ItemKey::Bpm => Some("bpm"),
        ItemKey::InitialKey => Some("key"),
        ItemKey::Composer => Some("composer"),
        ItemKey::Remixer => Some("remixer"),
        _ => None,
    }
}

/// Check whether a canonical field is available in RIFF INFO.
fn is_riff_info_field(field: &str) -> bool {
    RIFF_INFO_FIELDS.contains(&field)
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which WAV tag layers to target on write.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum WavTarget {
    Id3v2,
    RiffInfo,
}

/// Result of reading a single file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum FileReadResult {
    /// Single tag layer (FLAC, MP3, M4A).
    Single {
        path: String,
        format: String,
        tag_type: String,
        tags: HashMap<String, Option<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cover_art: Option<CoverArtMeta>,
    },
    /// Dual tag layer (WAV).
    Wav {
        path: String,
        format: String,
        id3v2: HashMap<String, Option<String>>,
        riff_info: HashMap<String, Option<String>>,
        tag3_missing: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cover_art: Option<CoverArtMeta>,
    },
    /// Error reading file.
    Error { path: String, error: String },
}

/// Metadata about embedded cover art (never contains binary data).
#[derive(Debug, Serialize)]
pub struct CoverArtMeta {
    pub format: String,
    pub size_bytes: usize,
}

/// How to merge the `comment` field with an existing value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum CommentMode {
    /// Overwrite existing comment (default).
    #[default]
    Replace,
    /// Prepend new text before existing comment, separated by ` | `.
    Prepend,
    /// Append new text after existing comment, separated by ` | `.
    Append,
}

const COMMENT_SEPARATOR: &str = " | ";

/// Merge a new comment value with an optional existing value.
pub fn merge_comment(new: &str, existing: Option<&str>, mode: CommentMode) -> String {
    match mode {
        CommentMode::Replace => new.to_string(),
        CommentMode::Prepend => match existing {
            Some(ex) if !ex.is_empty() => format!("{new}{COMMENT_SEPARATOR}{ex}"),
            _ => new.to_string(),
        },
        CommentMode::Append => match existing {
            Some(ex) if !ex.is_empty() => format!("{ex}{COMMENT_SEPARATOR}{new}"),
            _ => new.to_string(),
        },
    }
}

/// A single write entry.
pub struct WriteEntry {
    pub path: PathBuf,
    pub tags: HashMap<String, Option<String>>,
    /// WAV only — which tag layers to write. Default: both.
    pub wav_targets: Vec<WavTarget>,
    /// How to handle the `comment` field if it already has a value.
    pub comment_mode: CommentMode,
}

/// Result of writing a single file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum FileWriteResult {
    Ok {
        path: String,
        status: String,
        fields_written: Vec<String>,
        fields_deleted: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wav_targets: Option<Vec<String>>,
    },
    Error {
        path: String,
        status: String,
        error: String,
    },
}

/// A single field change in a dry-run result.
#[derive(Debug, Serialize)]
pub struct DryRunChange {
    pub old: Option<String>,
    pub new: Option<String>,
}

/// Dry-run result for a single file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum FileDryRunResult {
    Preview {
        path: String,
        status: String,
        changes: HashMap<String, DryRunChange>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wav_targets: Option<Vec<String>>,
    },
    Error {
        path: String,
        status: String,
        error: String,
    },
}

/// Result of extracting cover art to disk.
#[derive(Debug, Serialize)]
pub struct ExtractArtResult {
    pub path: String,
    pub output_path: String,
    pub image_format: String,
    pub size_bytes: usize,
    pub picture_type: String,
}

/// Result of embedding cover art into a single file.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum FileEmbedResult {
    Ok {
        path: String,
        status: String,
    },
    Error {
        path: String,
        status: String,
        error: String,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build `ParseOptions` with sensible defaults.
fn parse_options(read_cover_art: bool) -> ParseOptions {
    ParseOptions::new()
        .read_cover_art(read_cover_art)
        .parsing_mode(ParsingMode::BestAttempt)
}

/// Friendly format name from `FileType`.
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

/// Friendly tag-type name.
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

/// Read a canonical field value from a generic `Tag`.
///
/// Handles format-specific key splits:
/// - `year`: tries `RecordingDate`, then `Year`
/// - `bpm`: tries `IntegerBpm`, then `Bpm`
///
/// Returns:
/// - `Some(val)` — tag present with value (possibly empty string)
/// - `None` — tag absent or unknown field
fn get_field_from_tag(tag: &Tag, field: &str) -> Option<String> {
    let primary = field_to_item_key(field)?;

    if let Some(val) = tag.get_string(primary) {
        return Some(val.to_string());
    }

    // Secondary fallback keys
    match field {
        "year" => tag.get_string(ItemKey::Year).map(|s| s.to_string()),
        "bpm" => tag.get_string(ItemKey::Bpm).map(|s| s.to_string()),
        _ => None,
    }
}

/// Read all requested fields from a tag, returning a map where:
/// - present key with `Some(val)` → tag has value
/// - present key with `None` → tag absent
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

/// Resolve which fields to read — either the supplied filter or all fields.
fn resolve_fields(filter: Option<&[String]>) -> Vec<&str> {
    match filter {
        Some(f) => f.iter().map(|s| s.as_str()).collect(),
        None => ALL_FIELDS.to_vec(),
    }
}

/// Parse a `PictureType` from a string name. Defaults to `CoverFront`.
pub fn parse_picture_type(name: &str) -> PictureType {
    match name {
        "other" => PictureType::Other,
        "icon" => PictureType::Icon,
        "other_icon" => PictureType::OtherIcon,
        "front_cover" | "cover_front" => PictureType::CoverFront,
        "back_cover" | "cover_back" => PictureType::CoverBack,
        "leaflet" => PictureType::Leaflet,
        "media" => PictureType::Media,
        "lead_artist" => PictureType::LeadArtist,
        "artist" => PictureType::Artist,
        "conductor" => PictureType::Conductor,
        "band" => PictureType::Band,
        "composer" => PictureType::Composer,
        "lyricist" => PictureType::Lyricist,
        "recording_location" => PictureType::RecordingLocation,
        "during_recording" => PictureType::DuringRecording,
        "during_performance" => PictureType::DuringPerformance,
        "screen_capture" => PictureType::ScreenCapture,
        "illustration" => PictureType::Illustration,
        "band_logo" => PictureType::BandLogo,
        "publisher_logo" => PictureType::PublisherLogo,
        _ => PictureType::CoverFront,
    }
}

/// Format a `PictureType` as a snake_case string.
fn picture_type_name(pt: PictureType) -> &'static str {
    match pt {
        PictureType::Other => "other",
        PictureType::Icon => "icon",
        PictureType::OtherIcon => "other_icon",
        PictureType::CoverFront => "front_cover",
        PictureType::CoverBack => "back_cover",
        PictureType::Leaflet => "leaflet",
        PictureType::Media => "media",
        PictureType::LeadArtist => "lead_artist",
        PictureType::Artist => "artist",
        PictureType::Conductor => "conductor",
        PictureType::Band => "band",
        PictureType::Composer => "composer",
        PictureType::Lyricist => "lyricist",
        PictureType::RecordingLocation => "recording_location",
        PictureType::DuringRecording => "during_recording",
        PictureType::DuringPerformance => "during_performance",
        PictureType::ScreenCapture => "screen_capture",
        PictureType::BrightFish => "bright_fish",
        PictureType::Illustration => "illustration",
        PictureType::BandLogo => "band_logo",
        PictureType::PublisherLogo => "publisher_logo",
        _ => "other",
    }
}

/// File extension for a `MimeType`.
fn mime_extension(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Jpeg) => "jpg",
        Some(MimeType::Png) => "png",
        Some(MimeType::Tiff) => "tif",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Gif) => "gif",
        _ => "bin",
    }
}

/// Friendly MIME type name.
fn mime_name(mime: Option<&MimeType>) -> &'static str {
    match mime {
        Some(MimeType::Jpeg) => "jpeg",
        Some(MimeType::Png) => "png",
        Some(MimeType::Tiff) => "tiff",
        Some(MimeType::Bmp) => "bmp",
        Some(MimeType::Gif) => "gif",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate tag values before writing.
///
/// Rules:
/// - `year`: must be 4-digit YYYY or null/empty (delete)
/// - `track`, `disc`: must be positive integer or null/empty (delete)
/// - All other fields: accepted as-is
pub fn validate_write_tags(tags: &HashMap<String, Option<String>>) -> Result<(), TagError> {
    for (field, value) in tags {
        // Check field name validity first — even for null/empty (delete) values
        let is_validated_field = matches!(field.as_str(), "year" | "track" | "disc");
        if !is_validated_field && field_to_item_key(field).is_none() {
            return Err(TagError::Validation(format!("Unknown field \"{field}\"")));
        }

        let Some(val) = value else { continue };
        if val.is_empty() {
            continue; // empty means delete
        }

        match field.as_str() {
            "year" => {
                if val.len() != 4 || val.parse::<u16>().is_err() {
                    return Err(TagError::Validation(format!(
                        "Invalid year \"{val}\": must be 4-digit YYYY or null/empty to delete"
                    )));
                }
            }
            "track" | "disc" => match val.parse::<u32>() {
                Ok(n) if n > 0 => {}
                _ => {
                    return Err(TagError::Validation(format!(
                        "Invalid {field} \"{val}\": must be a positive integer or null/empty to delete"
                    )));
                }
            },
            _ => {}
        }
    }
    Ok(())
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

/// Read WAV file with dual tag layers.
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
                && id3v2.get(field).is_some_and(|v| v.is_some())
                && riff_info.get(field).is_some_and(|v| v.is_none())
        })
        .map(|f| f.to_string())
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

/// Read single-layer tag (FLAC, MP3, M4A, etc.).
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

// ---------------------------------------------------------------------------
// 2. write_file_tags
// ---------------------------------------------------------------------------

/// Write tags to a single audio file with merge semantics.
///
/// Read-modify-write: only the specified fields are touched, everything else
/// is preserved. Both `None` and `Some("")` delete the tag frame.
pub fn write_file_tags(entry: &WriteEntry) -> FileWriteResult {
    let path_str = entry.path.display().to_string();

    if let Err(e) = validate_write_tags(&entry.tags) {
        return FileWriteResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        };
    }

    match write_file_tags_inner(entry) {
        Ok(result) => result,
        Err(e) => FileWriteResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

/// Generate a temp path in the same directory as the original for atomic rename.
/// Format: `.{stem}.rklw-{pid}-{ms}.{ext}`
fn atomic_temp_path(original: &Path) -> PathBuf {
    let stem = original
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = original
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("tmp");
    let pid = std::process::id();
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let filename = format!(".{stem}.rklw-{pid}-{ms}.{ext}");
    original.with_file_name(filename)
}

fn write_file_tags_inner(entry: &WriteEntry) -> Result<FileWriteResult, TagError> {
    let path = &entry.path;
    let path_str = path.display().to_string();

    // Read file to detect format
    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(false))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();
    let is_wav = file_type == FileType::Wav;

    let wav_targets = if is_wav {
        if entry.wav_targets.is_empty() {
            vec![WavTarget::Id3v2, WavTarget::RiffInfo]
        } else {
            entry.wav_targets.clone()
        }
    } else {
        vec![]
    };

    let mut fields_written = Vec::new();
    let mut fields_deleted = Vec::new();

    if is_wav && wav_targets.len() > 1 {
        // Atomic dual-layer WAV write: copy → write both layers → rename.
        // Prevents split-state files from partial failures.
        let temp_path = atomic_temp_path(path);
        fs::copy(path, &temp_path).map_err(|e| {
            TagError::Io(format!("Failed to create temp copy: {e}"))
        })?;

        let result = (|| -> Result<(), TagError> {
            for target in &wav_targets {
                let tag_type = match target {
                    WavTarget::Id3v2 => TagType::Id3v2,
                    WavTarget::RiffInfo => TagType::RiffInfo,
                };
                write_tag_layer(
                    &temp_path,
                    tag_type,
                    &entry.tags,
                    *target == WavTarget::RiffInfo,
                    entry.comment_mode,
                    &mut fields_written,
                    &mut fields_deleted,
                )?;
            }
            Ok(())
        })();

        if let Err(e) = result {
            let _ = fs::remove_file(&temp_path);
            return Err(e);
        }

        fs::rename(&temp_path, path).map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            TagError::Io(format!("Failed to atomically replace file: {e}"))
        })?;
    } else if is_wav {
        // Single-target WAV — direct write, no atomicity concern
        let target = &wav_targets[0];
        let tag_type = match target {
            WavTarget::Id3v2 => TagType::Id3v2,
            WavTarget::RiffInfo => TagType::RiffInfo,
        };
        write_tag_layer(
            path,
            tag_type,
            &entry.tags,
            *target == WavTarget::RiffInfo,
            entry.comment_mode,
            &mut fields_written,
            &mut fields_deleted,
        )?;
    } else {
        // Single tag layer — use primary tag type
        let tag_type = file_type.primary_tag_type();
        write_tag_layer(
            path,
            tag_type,
            &entry.tags,
            false,
            entry.comment_mode,
            &mut fields_written,
            &mut fields_deleted,
        )?;
    }

    // De-duplicate (WAV writes to both layers → duplicate entries)
    fields_written.sort();
    fields_written.dedup();
    fields_deleted.sort();
    fields_deleted.dedup();

    Ok(FileWriteResult::Ok {
        path: path_str,
        status: "ok".to_string(),
        fields_written,
        fields_deleted,
        wav_targets: if is_wav {
            Some(
                wav_targets
                    .iter()
                    .map(|t| match t {
                        WavTarget::Id3v2 => "id3v2".to_string(),
                        WavTarget::RiffInfo => "riff_info".to_string(),
                    })
                    .collect(),
            )
        } else {
            None
        },
    })
}

/// Write to a single tag layer within a file.
///
/// For each field in `tags`:
/// - `None` or `Some("")` → delete the field
/// - `Some(value)` → set the field
///
/// If `riff_info_layer` is true, skip fields not available in RIFF INFO.
fn write_tag_layer(
    path: &Path,
    tag_type: TagType,
    tags: &HashMap<String, Option<String>>,
    riff_info_layer: bool,
    comment_mode: CommentMode,
    fields_written: &mut Vec<String>,
    fields_deleted: &mut Vec<String>,
) -> Result<(), TagError> {
    // Re-read the file for this tag layer (lofty requires read-modify-write
    // per tag type since save_to_path reopens the file).
    // Must read cover art (`true`) so existing pictures survive the round-trip.
    let mut tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    // Get or create the tag
    let tag = match tagged_file.tag_mut(tag_type) {
        Some(t) => t,
        None => {
            // Insert a new empty tag of this type
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.tag_mut(tag_type).ok_or_else(|| {
                TagError::Unsupported(format!("File does not support {tag_type:?} tags"))
            })?
        }
    };

    let mut any_changes = false;

    for (field, value) in tags {
        // Skip fields unavailable in RIFF INFO
        if riff_info_layer && !is_riff_info_field(field) {
            continue;
        }

        let Some(primary_key) = field_to_item_key(field) else {
            continue;
        };

        let should_delete = value.as_ref().is_none_or(|v| v.is_empty());
        let current_value = get_field_from_tag(tag, field);

        if should_delete {
            // Skip if already absent
            if current_value.is_none() {
                continue;
            }
            // Remove primary key
            tag.remove_key(primary_key);
            // Also remove secondary keys for split-key fields
            match field.as_str() {
                "year" => tag.remove_key(ItemKey::Year),
                "bpm" => tag.remove_key(ItemKey::Bpm),
                _ => {}
            }
            fields_deleted.push(field.clone());
            any_changes = true;
        } else {
            let raw_value = value.as_ref().unwrap();
            // Apply comment merge logic when writing the comment field
            let new_value = if field == "comment" && comment_mode != CommentMode::Replace {
                merge_comment(raw_value, current_value.as_deref(), comment_mode)
            } else {
                raw_value.clone()
            };
            // Skip if value is unchanged
            if current_value.as_deref() == Some(new_value.as_str()) {
                continue;
            }
            tag.insert_text(primary_key, new_value.clone());
            // For non-Vorbis tags, also write secondary keys for compatibility.
            // Vorbis Comments use DATE (not YEAR) per spec, and BPM is already
            // the correct key — secondary writes would create duplicate fields.
            if tag_type != TagType::VorbisComments {
                if field == "year" {
                    tag.insert_text(ItemKey::Year, new_value.clone());
                }
                if field == "bpm" {
                    tag.insert_text(ItemKey::Bpm, new_value.clone());
                }
            }
            fields_written.push(field.clone());
            any_changes = true;
        }
    }

    if any_changes {
        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write {tag_type:?} tag: {e}")))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// 3. write_file_tags_dry_run
// ---------------------------------------------------------------------------

/// Preview what a write would do — returns old→new diff for each field.
pub fn write_file_tags_dry_run(entry: &WriteEntry) -> FileDryRunResult {
    let path_str = entry.path.display().to_string();

    if let Err(e) = validate_write_tags(&entry.tags) {
        return FileDryRunResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        };
    }

    match write_file_tags_dry_run_inner(entry) {
        Ok(result) => result,
        Err(e) => FileDryRunResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

fn write_file_tags_dry_run_inner(entry: &WriteEntry) -> Result<FileDryRunResult, TagError> {
    let path = &entry.path;
    let path_str = path.display().to_string();

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(false))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();
    let is_wav = file_type == FileType::Wav;

    let wav_targets = if is_wav {
        if entry.wav_targets.is_empty() {
            vec![WavTarget::Id3v2, WavTarget::RiffInfo]
        } else {
            entry.wav_targets.clone()
        }
    } else {
        vec![]
    };

    // For the dry-run diff, read from the tag layer that will be written.
    // WAV with riff_info-only target: diff against RIFF INFO.
    // WAV with id3v2-only or both: diff against ID3v2.
    // Non-WAV: use primary tag.
    let riff_only = is_wav && wav_targets.len() == 1 && wav_targets[0] == WavTarget::RiffInfo;
    let primary_tag = if is_wav {
        if riff_only {
            tagged_file.tag(TagType::RiffInfo)
        } else {
            tagged_file.tag(TagType::Id3v2)
        }
    } else {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    };
    let mut changes = HashMap::new();

    for (field, new_value) in &entry.tags {
        // Skip fields that the write path would skip for RIFF-only WAV targets
        if riff_only && !is_riff_info_field(field) {
            continue;
        }

        let old_value = primary_tag.and_then(|t| get_field_from_tag(t, field));

        let effective_new: Option<String> = match new_value {
            None => None,
            Some(v) if v.is_empty() => None,
            Some(v) => {
                if field == "comment" && entry.comment_mode != CommentMode::Replace {
                    Some(merge_comment(v, old_value.as_deref(), entry.comment_mode))
                } else {
                    Some(v.clone())
                }
            }
        };

        // Only include in diff if there's an actual change
        if old_value != effective_new {
            changes.insert(
                field.clone(),
                DryRunChange {
                    old: old_value,
                    new: effective_new,
                },
            );
        }
    }

    Ok(FileDryRunResult::Preview {
        path: path_str,
        status: "preview".to_string(),
        changes,
        wav_targets: if is_wav {
            Some(
                wav_targets
                    .iter()
                    .map(|t| match t {
                        WavTarget::Id3v2 => "id3v2".to_string(),
                        WavTarget::RiffInfo => "riff_info".to_string(),
                    })
                    .collect(),
            )
        } else {
            None
        },
    })
}

// ---------------------------------------------------------------------------
// 4. extract_cover_art
// ---------------------------------------------------------------------------

/// Extract embedded cover art to disk.
///
/// For WAV files, reads from ID3v2 only (RIFF INFO does not support images).
/// If `output_path` is `None`, writes to `{parent_dir}/cover.{ext}`.
pub fn extract_cover_art(
    path: &Path,
    output_path: Option<&Path>,
    picture_type: &str,
) -> Result<ExtractArtResult, TagError> {
    let path_str = path.display().to_string();
    let pic_type = parse_picture_type(picture_type);

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    // For WAV, read from ID3v2 only
    let tag = if file_type == FileType::Wav {
        tagged_file.tag(TagType::Id3v2)
    } else {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    };

    let tag = tag.ok_or(TagError::NoTags)?;

    // Find the requested picture type, fall back to any picture
    let picture = tag
        .pictures()
        .iter()
        .find(|p| p.pic_type() == pic_type)
        .or_else(|| tag.pictures().first())
        .ok_or(TagError::NoPicture)?;

    let ext = mime_extension(picture.mime_type());
    let image_format = mime_name(picture.mime_type());

    let out_path = match output_path {
        Some(p) => p.to_path_buf(),
        None => {
            let parent = path.parent().unwrap_or(Path::new("."));
            parent.join(format!("cover.{ext}"))
        }
    };

    fs::write(&out_path, picture.data())
        .map_err(|e| TagError::Io(format!("Failed to write cover art: {e}")))?;

    Ok(ExtractArtResult {
        path: path_str,
        output_path: out_path.display().to_string(),
        image_format: image_format.to_string(),
        size_bytes: picture.data().len(),
        picture_type: picture_type_name(picture.pic_type()).to_string(),
    })
}

// ---------------------------------------------------------------------------
// 5. embed_cover_art
// ---------------------------------------------------------------------------

/// Embed an image file into an audio file as cover art.
///
/// For WAV files, writes to ID3v2 only (RIFF INFO does not support images).
pub fn embed_cover_art(
    image_path: &Path,
    target_path: &Path,
    picture_type: &str,
) -> FileEmbedResult {
    let target_str = target_path.display().to_string();

    match embed_cover_art_inner(image_path, target_path, picture_type) {
        Ok(()) => FileEmbedResult::Ok {
            path: target_str,
            status: "ok".to_string(),
        },
        Err(e) => FileEmbedResult::Error {
            path: target_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

fn embed_cover_art_inner(
    image_path: &Path,
    target_path: &Path,
    picture_type_str: &str,
) -> Result<(), TagError> {
    let pic_type = parse_picture_type(picture_type_str);

    // Read image data and detect format via lofty
    let image_data =
        fs::read(image_path).map_err(|e| TagError::Io(format!("Failed to read image: {e}")))?;

    // Detect MIME type from the data
    let mut cursor = std::io::Cursor::new(&image_data);
    let detected = Picture::from_reader(&mut cursor)
        .map_err(|e| TagError::Io(format!("Failed to parse image: {e}")))?;

    // Build a new picture with the desired PictureType and detected MIME
    let mut builder = Picture::unchecked(image_data).pic_type(pic_type);
    if let Some(mime) = detected.mime_type() {
        builder = builder.mime_type(mime.clone());
    }
    let picture = builder.build();

    // Read the target file
    let mut tagged_file = Probe::open(target_path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    if file_type == FileType::Wav {
        // WAV: embed into ID3v2 only
        let tag = match tagged_file.tag_mut(TagType::Id3v2) {
            Some(t) => t,
            None => {
                tagged_file.insert_tag(Tag::new(TagType::Id3v2));
                tagged_file
                    .tag_mut(TagType::Id3v2)
                    .ok_or(TagError::Unsupported(
                        "WAV file does not support ID3v2".to_string(),
                    ))?
            }
        };

        tag.remove_picture_type(pic_type);
        tag.push_picture(picture);

        tag.save_to_path(target_path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write ID3v2 tag: {e}")))?;
    } else {
        // Single tag layer — use primary tag type
        let primary_type = file_type.primary_tag_type();
        let tag = match tagged_file.tag_mut(primary_type) {
            Some(t) => t,
            None => {
                tagged_file.insert_tag(Tag::new(primary_type));
                tagged_file.tag_mut(primary_type).ok_or_else(|| {
                    TagError::Unsupported(format!("File does not support {primary_type:?} tags"))
                })?
            }
        };

        tag.remove_picture_type(pic_type);
        tag.push_picture(picture);

        tag.save_to_path(target_path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write tag: {e}")))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_to_key_roundtrip() {
        for &field in ALL_FIELDS {
            let key = field_to_item_key(field)
                .unwrap_or_else(|| panic!("No ItemKey for field \"{field}\""));
            let back =
                item_key_to_field(&key).unwrap_or_else(|| panic!("No field for ItemKey {key:?}"));
            assert_eq!(back, field, "Roundtrip failed for {field}");
        }
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
        assert!(validate_write_tags(&tags).is_ok());
    }

    #[test]
    fn validate_year_delete() {
        let mut tags = HashMap::new();
        tags.insert("year".to_string(), None);
        assert!(validate_write_tags(&tags).is_ok());

        let mut tags2 = HashMap::new();
        tags2.insert("year".to_string(), Some("".to_string()));
        assert!(validate_write_tags(&tags2).is_ok());
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
        assert!(validate_write_tags(&tags).is_ok());

        let mut tags2 = HashMap::new();
        tags2.insert("track".to_string(), Some("99".to_string()));
        assert!(validate_write_tags(&tags2).is_ok());
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
        assert!(validate_write_tags(&tags).is_ok());
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
        assert!(validate_write_tags(&tags).is_ok());
    }

    #[test]
    fn parse_picture_type_known() {
        assert_eq!(parse_picture_type("front_cover"), PictureType::CoverFront);
        assert_eq!(parse_picture_type("cover_front"), PictureType::CoverFront);
        assert_eq!(parse_picture_type("back_cover"), PictureType::CoverBack);
        assert_eq!(parse_picture_type("band_logo"), PictureType::BandLogo);
    }

    #[test]
    fn parse_picture_type_default() {
        assert_eq!(parse_picture_type("garbage"), PictureType::CoverFront);
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
        // Create a minimal valid WAV file for testing.
        // It has no real tag payload; this test validates RIFF-only dry-run
        // field filtering for unsupported fields.
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("test.wav");

        // Write a minimal valid WAV file (44-byte header + 2 bytes of silence)
        let wav_header: Vec<u8> = {
            let data_size: u32 = 2; // 1 sample, 16-bit mono
            let file_size = 36 + data_size;
            let mut h = Vec::new();
            h.extend_from_slice(b"RIFF");
            h.extend_from_slice(&file_size.to_le_bytes());
            h.extend_from_slice(b"WAVE");
            h.extend_from_slice(b"fmt ");
            h.extend_from_slice(&16u32.to_le_bytes()); // chunk size
            h.extend_from_slice(&1u16.to_le_bytes()); // PCM
            h.extend_from_slice(&1u16.to_le_bytes()); // mono
            h.extend_from_slice(&44100u32.to_le_bytes()); // sample rate
            h.extend_from_slice(&88200u32.to_le_bytes()); // byte rate
            h.extend_from_slice(&2u16.to_le_bytes()); // block align
            h.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
            h.extend_from_slice(b"data");
            h.extend_from_slice(&data_size.to_le_bytes());
            h.extend_from_slice(&[0u8; 2]); // 1 silent sample
            h
        };
        std::fs::write(&wav_path, &wav_header).unwrap();

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
}
