//! Audio-file tag reading and atomic mutation adapter using `lofty`.
//!
//! Pure functions with NO MCP dependency. Called by both MCP tool wrappers
//! and CLI subcommands. All functions are synchronous — callers use
//! `spawn_blocking` for async contexts.

use std::collections::{BTreeMap, HashMap};
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
    #[error("No cover art found in file")]
    NoPicture,
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

/// Exact, case-sensitive picture type names accepted by cover-art operations.
pub(crate) const ACCEPTED_PICTURE_TYPES: &[&str] = &[
    "other",
    "icon",
    "other_icon",
    "front_cover",
    "cover_front",
    "back_cover",
    "cover_back",
    "leaflet",
    "media",
    "lead_artist",
    "artist",
    "conductor",
    "band",
    "composer",
    "lyricist",
    "recording_location",
    "during_recording",
    "during_performance",
    "screen_capture",
    "bright_fish",
    "illustration",
    "band_logo",
    "publisher_logo",
];

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

impl From<&WavTarget> for TagType {
    fn from(target: &WavTarget) -> Self {
        match target {
            WavTarget::Id3v2 => TagType::Id3v2,
            WavTarget::RiffInfo => TagType::RiffInfo,
        }
    }
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

impl FileWriteResult {
    pub(crate) fn with_reported_path(mut self, reported_path: String) -> Self {
        match &mut self {
            Self::Ok { path, .. } | Self::Error { path, .. } => *path = reported_path,
        }
        self
    }
}

/// A single field change in a dry-run result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
        changes_by_layer: Option<BTreeMap<String, BTreeMap<String, DryRunChange>>>,
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

fn parse_options(read_cover_art: bool) -> ParseOptions {
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

    match field {
        "year" => tag
            .get_string(ItemKey::Year)
            .map(std::string::ToString::to_string),
        "bpm" => tag
            .get_string(ItemKey::Bpm)
            .map(std::string::ToString::to_string),
        _ => None,
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

fn resolve_fields(filter: Option<&[String]>) -> Vec<&str> {
    match filter {
        Some(f) => f.iter().map(std::string::String::as_str).collect(),
        None => ALL_FIELDS.to_vec(),
    }
}

/// Parse an exact, case-sensitive cover-art picture type name.
pub fn parse_picture_type(name: &str) -> Result<PictureType, TagError> {
    let picture_type = match name {
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
        "bright_fish" => PictureType::BrightFish,
        "illustration" => PictureType::Illustration,
        "band_logo" => PictureType::BandLogo,
        "publisher_logo" => PictureType::PublisherLogo,
        _ => {
            return Err(TagError::Validation(format!(
                "Unknown picture type {name:?}. Accepted values: {}",
                ACCEPTED_PICTURE_TYPES.join(", ")
            )));
        }
    };
    Ok(picture_type)
}

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
        fs::copy(path, &temp_path)
            .map_err(|e| TagError::Io(format!("Failed to create temp copy: {e}")))?;

        let result = (|| -> Result<(), TagError> {
            for target in &wav_targets {
                let tag_type = TagType::from(target);
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
        let tag_type = TagType::from(target);
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

    let tag = match tagged_file.tag_mut(tag_type) {
        Some(t) => t,
        None => {
            tagged_file.insert_tag(Tag::new(tag_type));
            tagged_file.tag_mut(tag_type).ok_or_else(|| {
                TagError::Unsupported(format!("File does not support {tag_type:?} tags"))
            })?
        }
    };

    let mut any_changes = false;

    for (field, value) in tags {
        if riff_info_layer && !is_riff_info_field(field) {
            continue;
        }

        let Some(primary_key) = field_to_item_key(field) else {
            continue;
        };

        let should_delete = value.as_ref().is_none_or(std::string::String::is_empty);
        let current_value = get_field_from_tag(tag, field);

        if should_delete {
            if current_value.is_none() {
                continue;
            }
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
            let new_value = if field == "comment" && comment_mode != CommentMode::Replace {
                merge_comment(raw_value, current_value.as_deref(), comment_mode)
            } else {
                raw_value.clone()
            };
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

fn dry_run_layer_diff(
    entry: &WriteEntry,
    tag: Option<&Tag>,
    riff_info_layer: bool,
) -> BTreeMap<String, DryRunChange> {
    let mut changes = BTreeMap::new();

    for (field, new_value) in &entry.tags {
        if riff_info_layer && !is_riff_info_field(field) {
            continue;
        }

        let old_value = tag.and_then(|tag| get_field_from_tag(tag, field));
        let effective_new = match new_value {
            None => None,
            Some(value) if value.is_empty() => None,
            Some(value) if field == "comment" && entry.comment_mode != CommentMode::Replace => {
                Some(merge_comment(
                    value,
                    old_value.as_deref(),
                    entry.comment_mode,
                ))
            }
            Some(value) => Some(value.clone()),
        };

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

    changes
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

    let (changes, changes_by_layer) = if is_wav {
        let mut changes_by_layer = BTreeMap::new();
        for target in &wav_targets {
            let (key, tag_type, riff_info_layer) = match target {
                WavTarget::Id3v2 => ("id3v2", TagType::Id3v2, false),
                WavTarget::RiffInfo => ("riff_info", TagType::RiffInfo, true),
            };
            changes_by_layer.insert(
                key.to_string(),
                dry_run_layer_diff(entry, tagged_file.tag(tag_type), riff_info_layer),
            );
        }

        // Preserve the legacy map's source layer exactly: a single RIFF target
        // mirrors RIFF INFO, while ID3-only, both, and duplicate-target inputs
        // retain the historical ID3v2 comparison.
        let riff_only = wav_targets.len() == 1 && wav_targets[0] == WavTarget::RiffInfo;
        let compatibility_key = if riff_only { "riff_info" } else { "id3v2" };
        let compatibility_diff = changes_by_layer
            .get(compatibility_key)
            .cloned()
            .unwrap_or_else(|| dry_run_layer_diff(entry, tagged_file.tag(TagType::Id3v2), false));
        (
            compatibility_diff.into_iter().collect(),
            Some(changes_by_layer),
        )
    } else {
        let primary_tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        (
            dry_run_layer_diff(entry, primary_tag, false)
                .into_iter()
                .collect(),
            None,
        )
    };

    Ok(FileDryRunResult::Preview {
        path: path_str,
        status: "preview".to_string(),
        changes,
        changes_by_layer,
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
    let pic_type = parse_picture_type(picture_type)?;

    let tagged_file = Probe::open(path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    let tag = if file_type == FileType::Wav {
        tagged_file.tag(TagType::Id3v2)
    } else {
        tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag())
    };

    let tag = tag.ok_or(TagError::NoTags)?;

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
    let pic_type = parse_picture_type(picture_type_str)?;

    let image_data =
        fs::read(image_path).map_err(|e| TagError::Io(format!("Failed to read image: {e}")))?;

    // Detect MIME type from the data, then drop the temporary Picture to free its copy
    let mime = {
        let mut cursor = std::io::Cursor::new(&image_data);
        let detected = Picture::from_reader(&mut cursor)
            .map_err(|e| TagError::Io(format!("Failed to parse image: {e}")))?;
        detected.mime_type().cloned()
    };

    let mut builder = Picture::unchecked(image_data).pic_type(pic_type);
    if let Some(mime) = mime {
        builder = builder.mime_type(mime);
    }
    let picture = builder.build();

    let mut tagged_file = Probe::open(target_path)
        .map_err(|e| TagError::Io(format!("Failed to open: {e}")))?
        .options(parse_options(true))
        .read()
        .map_err(|e| TagError::Io(format!("Failed to read: {e}")))?;

    let file_type = tagged_file.file_type();

    if file_type == FileType::Wav {
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

    fn dry_run_test_entry(
        tags: HashMap<String, Option<String>>,
        comment_mode: CommentMode,
    ) -> WriteEntry {
        WriteEntry {
            path: PathBuf::from("synthetic.wav"),
            tags,
            wav_targets: vec![],
            comment_mode,
        }
    }

    fn cover_art_test_png() -> Vec<u8> {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
            0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
        ]
    }

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
    fn dry_run_layer_diff_reports_deletion() {
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::TrackArtist, "old".to_string());

        for deletion in [None, Some(String::new())] {
            let entry = dry_run_test_entry(
                HashMap::from([("artist".to_string(), deletion)]),
                CommentMode::Replace,
            );
            assert_eq!(
                dry_run_layer_diff(&entry, Some(&tag), false).get("artist"),
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
        let entry = dry_run_test_entry(
            HashMap::from([("artist".to_string(), Some("same".to_string()))]),
            CommentMode::Replace,
        );

        assert!(dry_run_layer_diff(&entry, Some(&tag), false).is_empty());
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
            let entry = dry_run_test_entry(
                HashMap::from([("comment".to_string(), Some("new".to_string()))]),
                mode,
            );
            assert_eq!(
                dry_run_layer_diff(&entry, Some(&tag), false)["comment"]
                    .new
                    .as_deref(),
                Some(expected)
            );
        }
    }

    #[test]
    fn dry_run_layer_diff_handles_missing_tag() {
        let entry = dry_run_test_entry(
            HashMap::from([("artist".to_string(), Some("new".to_string()))]),
            CommentMode::Replace,
        );

        assert_eq!(
            dry_run_layer_diff(&entry, None, false).get("artist"),
            Some(&DryRunChange {
                old: None,
                new: Some("new".to_string()),
            })
        );
    }

    #[test]
    fn dry_run_layer_diff_filters_riff_unsupported_fields() {
        let entry = dry_run_test_entry(
            HashMap::from([
                ("artist".to_string(), Some("new artist".to_string())),
                ("key".to_string(), Some("Am".to_string())),
            ]),
            CommentMode::Replace,
        );

        let changes = dry_run_layer_diff(&entry, None, true);
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
}
