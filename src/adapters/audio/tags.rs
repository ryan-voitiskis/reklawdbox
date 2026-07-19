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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum TagField {
    Artist,
    Title,
    Album,
    AlbumArtist,
    Genre,
    Year,
    Track,
    Disc,
    Comment,
    Publisher,
    Bpm,
    Key,
    Composer,
    Remixer,
}

impl TagField {
    #[cfg(test)]
    const ALL: [Self; 14] = [
        Self::Artist,
        Self::Title,
        Self::Album,
        Self::AlbumArtist,
        Self::Genre,
        Self::Year,
        Self::Track,
        Self::Disc,
        Self::Comment,
        Self::Publisher,
        Self::Bpm,
        Self::Key,
        Self::Composer,
        Self::Remixer,
    ];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Artist => "artist",
            Self::Title => "title",
            Self::Album => "album",
            Self::AlbumArtist => "album_artist",
            Self::Genre => "genre",
            Self::Year => "year",
            Self::Track => "track",
            Self::Disc => "disc",
            Self::Comment => "comment",
            Self::Publisher => "publisher",
            Self::Bpm => "bpm",
            Self::Key => "key",
            Self::Composer => "composer",
            Self::Remixer => "remixer",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "artist" => Some(Self::Artist),
            "title" => Some(Self::Title),
            "album" => Some(Self::Album),
            "album_artist" => Some(Self::AlbumArtist),
            "genre" => Some(Self::Genre),
            "year" => Some(Self::Year),
            "track" => Some(Self::Track),
            "disc" => Some(Self::Disc),
            "comment" => Some(Self::Comment),
            "publisher" => Some(Self::Publisher),
            "bpm" => Some(Self::Bpm),
            "key" => Some(Self::Key),
            "composer" => Some(Self::Composer),
            "remixer" => Some(Self::Remixer),
            _ => None,
        }
    }

    const fn is_riff_info(self) -> bool {
        matches!(
            self,
            Self::Artist | Self::Title | Self::Album | Self::Genre | Self::Year | Self::Comment
        )
    }

    const fn primary_item_key(self) -> ItemKey {
        match self {
            Self::Artist => ItemKey::TrackArtist,
            Self::Title => ItemKey::TrackTitle,
            Self::Album => ItemKey::AlbumTitle,
            Self::AlbumArtist => ItemKey::AlbumArtist,
            Self::Genre => ItemKey::Genre,
            Self::Year => ItemKey::RecordingDate,
            Self::Track => ItemKey::TrackNumber,
            Self::Disc => ItemKey::DiscNumber,
            Self::Comment => ItemKey::Comment,
            Self::Publisher => ItemKey::Label,
            Self::Bpm => ItemKey::IntegerBpm,
            Self::Key => ItemKey::InitialKey,
            Self::Composer => ItemKey::Composer,
            Self::Remixer => ItemKey::Remixer,
        }
    }

    const fn secondary_item_key(self) -> Option<ItemKey> {
        match self {
            Self::Year => Some(ItemKey::Year),
            Self::Bpm => Some(ItemKey::Bpm),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TagEdit {
    Set(String),
    Delete,
}

#[derive(Debug)]
struct ValidatedTagPatch {
    edits: BTreeMap<TagField, TagEdit>,
}

impl TryFrom<&HashMap<String, Option<String>>> for ValidatedTagPatch {
    type Error = TagError;

    fn try_from(tags: &HashMap<String, Option<String>>) -> Result<Self, Self::Error> {
        let mut edits = BTreeMap::new();

        for (raw_field, raw_value) in tags {
            let field = TagField::parse(raw_field)
                .ok_or_else(|| TagError::Validation(format!("Unknown field \"{raw_field}\"")))?;
            let edit = match raw_value {
                None => TagEdit::Delete,
                Some(value) if value.is_empty() => TagEdit::Delete,
                Some(value) => {
                    match field {
                        TagField::Year => {
                            if value.len() != 4 || value.parse::<u16>().is_err() {
                                return Err(TagError::Validation(format!(
                                    "Invalid year \"{value}\": must be 4-digit YYYY or null/empty to delete"
                                )));
                            }
                        }
                        TagField::Track | TagField::Disc => match value.parse::<u32>() {
                            Ok(number) if number > 0 => {}
                            _ => {
                                return Err(TagError::Validation(format!(
                                    "Invalid {} \"{value}\": must be a positive integer or null/empty to delete",
                                    field.as_str()
                                )));
                            }
                        },
                        _ => {}
                    }
                    TagEdit::Set(value.clone())
                }
            };
            edits.insert(field, edit);
        }

        Ok(Self { edits })
    }
}

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
    TagField::parse(field).is_some_and(TagField::is_riff_info)
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

impl WavTarget {
    const fn capability(&self) -> LayerCapability {
        match self {
            Self::Id3v2 => LayerCapability::Id3v2,
            Self::RiffInfo => LayerCapability::RiffInfo,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LayerCapability {
    Id3v2,
    RiffInfo,
    Primary,
}

impl LayerCapability {
    const fn supports(self, field: TagField) -> bool {
        !matches!(self, Self::RiffInfo) || field.is_riff_info()
    }
}

#[derive(Debug)]
struct ExistingLayerValues {
    values: BTreeMap<TagField, Option<String>>,
}

impl ExistingLayerValues {
    fn read(tag: Option<&Tag>, patch: &ValidatedTagPatch) -> Self {
        let values = patch
            .edits
            .keys()
            .copied()
            .map(|field| (field, tag.and_then(|tag| get_tag_field(tag, field))))
            .collect();
        Self { values }
    }

    fn get(&self, field: TagField) -> Option<String> {
        self.values.get(&field).cloned().flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlannedTagMutation {
    field: TagField,
    edit: TagEdit,
}

#[derive(Debug, PartialEq, Eq)]
struct LayerMutationPlan {
    operations: Vec<PlannedTagMutation>,
    changes: BTreeMap<String, DryRunChange>,
}

fn plan_layer_mutation(
    patch: &ValidatedTagPatch,
    capability: LayerCapability,
    existing: &ExistingLayerValues,
    comment_mode: CommentMode,
) -> LayerMutationPlan {
    let mut operations = Vec::new();
    let mut changes = BTreeMap::new();

    for (&field, edit) in &patch.edits {
        if !capability.supports(field) {
            continue;
        }

        let old = existing.get(field);
        let effective_edit = match edit {
            TagEdit::Delete => TagEdit::Delete,
            TagEdit::Set(value)
                if field == TagField::Comment && comment_mode != CommentMode::Replace =>
            {
                TagEdit::Set(merge_comment(value, old.as_deref(), comment_mode))
            }
            TagEdit::Set(value) => TagEdit::Set(value.clone()),
        };
        let new = match &effective_edit {
            TagEdit::Set(value) => Some(value.clone()),
            TagEdit::Delete => None,
        };

        if old == new {
            continue;
        }

        changes.insert(
            field.as_str().to_string(),
            DryRunChange {
                old,
                new: new.clone(),
            },
        );
        operations.push(PlannedTagMutation {
            field,
            edit: effective_edit,
        });
    }

    LayerMutationPlan {
        operations,
        changes,
    }
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
fn get_tag_field(tag: &Tag, field: TagField) -> Option<String> {
    let primary = field.primary_item_key();

    if let Some(val) = tag.get_string(primary) {
        return Some(val.to_string());
    }

    match field {
        TagField::Year => tag
            .get_string(ItemKey::Year)
            .map(std::string::ToString::to_string),
        TagField::Bpm => tag
            .get_string(ItemKey::Bpm)
            .map(std::string::ToString::to_string),
        _ => None,
    }
}

fn get_field_from_tag(tag: &Tag, field: &str) -> Option<String> {
    get_tag_field(tag, TagField::parse(field)?)
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
#[cfg(test)]
fn validate_write_tags(tags: &HashMap<String, Option<String>>) -> Result<(), TagError> {
    ValidatedTagPatch::try_from(tags).map(|_| ())
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

    let patch = match ValidatedTagPatch::try_from(&entry.tags) {
        Ok(patch) => patch,
        Err(error) => {
            return FileWriteResult::Error {
                path: path_str,
                status: "error".to_string(),
                error: error.to_string(),
            };
        }
    };

    match write_file_tags_inner(entry, &patch) {
        Ok(result) => result,
        Err(e) => FileWriteResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

#[cfg(test)]
thread_local! {
    static TEST_FAIL_TAG_LAYER_WRITE: std::cell::Cell<Option<TagType>> = const {
        std::cell::Cell::new(None)
    };
}

#[cfg(test)]
fn with_test_tag_layer_write_failure<T>(tag_type: TagType, operation: impl FnOnce() -> T) -> T {
    struct ResetFailure;

    impl Drop for ResetFailure {
        fn drop(&mut self) {
            TEST_FAIL_TAG_LAYER_WRITE.with(|failure| failure.set(None));
        }
    }

    TEST_FAIL_TAG_LAYER_WRITE.with(|failure| {
        assert!(
            failure.replace(Some(tag_type)).is_none(),
            "test tag-layer failure must not already be armed"
        );
    });
    let _reset = ResetFailure;
    operation()
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

fn write_file_tags_inner(
    entry: &WriteEntry,
    patch: &ValidatedTagPatch,
) -> Result<FileWriteResult, TagError> {
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
                    patch,
                    target.capability(),
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
            patch,
            target.capability(),
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
            patch,
            LayerCapability::Primary,
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

/// Plan and apply one validated patch to a single tag layer.
///
/// The pure planner owns capability filtering, comment merging, deletion, and
/// no-op policy. This function owns only Lofty I/O and plan application.
fn write_tag_layer(
    path: &Path,
    tag_type: TagType,
    patch: &ValidatedTagPatch,
    capability: LayerCapability,
    comment_mode: CommentMode,
    fields_written: &mut Vec<String>,
    fields_deleted: &mut Vec<String>,
) -> Result<(), TagError> {
    #[cfg(test)]
    if TEST_FAIL_TAG_LAYER_WRITE.with(|failure| {
        if failure.get() == Some(tag_type) {
            failure.set(None);
            true
        } else {
            false
        }
    }) {
        return Err(TagError::Io("injected tag-layer write failure".to_string()));
    }

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

    let existing = ExistingLayerValues::read(Some(tag), patch);
    let plan = plan_layer_mutation(patch, capability, &existing, comment_mode);
    apply_layer_mutation_plan(tag, tag_type, &plan, fields_written, fields_deleted);

    if !plan.operations.is_empty() {
        tag.save_to_path(path, WriteOptions::default())
            .map_err(|e| TagError::Io(format!("Failed to write {tag_type:?} tag: {e}")))?;
    }

    Ok(())
}

fn apply_layer_mutation_plan(
    tag: &mut Tag,
    tag_type: TagType,
    plan: &LayerMutationPlan,
    fields_written: &mut Vec<String>,
    fields_deleted: &mut Vec<String>,
) {
    for operation in &plan.operations {
        let primary_key = operation.field.primary_item_key();
        match &operation.edit {
            TagEdit::Delete => {
                tag.remove_key(primary_key);
                if let Some(secondary_key) = operation.field.secondary_item_key() {
                    tag.remove_key(secondary_key);
                }
                fields_deleted.push(operation.field.as_str().to_string());
            }
            TagEdit::Set(value) => {
                tag.insert_text(primary_key, value.clone());
                // Vorbis Comments use DATE (not YEAR) per spec, and BPM is
                // already the correct key. A secondary write would duplicate
                // either field.
                if tag_type != TagType::VorbisComments
                    && let Some(secondary_key) = operation.field.secondary_item_key()
                {
                    tag.insert_text(secondary_key, value.clone());
                }
                fields_written.push(operation.field.as_str().to_string());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3. write_file_tags_dry_run
// ---------------------------------------------------------------------------

/// Preview what a write would do — returns old→new diff for each field.
pub fn write_file_tags_dry_run(entry: &WriteEntry) -> FileDryRunResult {
    let path_str = entry.path.display().to_string();

    let patch = match ValidatedTagPatch::try_from(&entry.tags) {
        Ok(patch) => patch,
        Err(error) => {
            return FileDryRunResult::Error {
                path: path_str,
                status: "error".to_string(),
                error: error.to_string(),
            };
        }
    };

    match write_file_tags_dry_run_inner(entry, &patch) {
        Ok(result) => result,
        Err(e) => FileDryRunResult::Error {
            path: path_str,
            status: "error".to_string(),
            error: e.to_string(),
        },
    }
}

fn dry_run_layer_diff(
    patch: &ValidatedTagPatch,
    tag: Option<&Tag>,
    capability: LayerCapability,
    comment_mode: CommentMode,
) -> BTreeMap<String, DryRunChange> {
    let existing = ExistingLayerValues::read(tag, patch);
    plan_layer_mutation(patch, capability, &existing, comment_mode).changes
}

fn write_file_tags_dry_run_inner(
    entry: &WriteEntry,
    patch: &ValidatedTagPatch,
) -> Result<FileDryRunResult, TagError> {
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
            let (key, tag_type, capability) = match target {
                WavTarget::Id3v2 => ("id3v2", TagType::Id3v2, LayerCapability::Id3v2),
                WavTarget::RiffInfo => ("riff_info", TagType::RiffInfo, LayerCapability::RiffInfo),
            };
            changes_by_layer.insert(
                key.to_string(),
                dry_run_layer_diff(
                    patch,
                    tagged_file.tag(tag_type),
                    capability,
                    entry.comment_mode,
                ),
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
            .unwrap_or_else(|| {
                dry_run_layer_diff(
                    patch,
                    tagged_file.tag(TagType::Id3v2),
                    LayerCapability::Id3v2,
                    entry.comment_mode,
                )
            });
        (
            compatibility_diff.into_iter().collect(),
            Some(changes_by_layer),
        )
    } else {
        let primary_tag = tagged_file
            .primary_tag()
            .or_else(|| tagged_file.first_tag());
        (
            dry_run_layer_diff(
                patch,
                primary_tag,
                LayerCapability::Primary,
                entry.comment_mode,
            )
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

    fn dry_run_test_patch(tags: HashMap<String, Option<String>>) -> ValidatedTagPatch {
        ValidatedTagPatch::try_from(&tags).expect("synthetic patch should validate")
    }

    fn cover_art_test_png() -> Vec<u8> {
        vec![
            137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1,
            8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 100, 248, 15,
            0, 1, 5, 1, 1, 39, 24, 227, 102, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
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
            let patch =
                ValidatedTagPatch::try_from(&HashMap::from([("artist".to_string(), delete)]))
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

        let result =
            with_test_tag_layer_write_failure(TagType::RiffInfo, || write_file_tags(&entry));
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
}
