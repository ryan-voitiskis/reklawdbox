//! Audio-tag adapter request, result, and error models.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use serde::Serialize;

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

/// Which WAV tag layers to target on write.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CommentMode {
    /// Overwrite existing comment (default).
    #[default]
    Replace,
    /// Prepend new text before existing comment, separated by ` | `.
    Prepend,
    /// Append new text after existing comment, separated by ` | `.
    Append,
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
