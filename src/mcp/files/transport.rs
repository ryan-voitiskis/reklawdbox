use std::collections::HashMap;

use schemars::JsonSchema;
use serde::Deserialize;

/// Which WAV tag layers to target on write.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub(in crate::mcp) enum WavTarget {
    Id3v2,
    RiffInfo,
}

/// How to merge the `comment` field with an existing value.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub(in crate::mcp) enum CommentMode {
    /// Overwrite existing comment (default).
    #[default]
    Replace,
    /// Prepend new text before existing comment, separated by ` | `.
    Prepend,
    /// Append new text after existing comment, separated by ` | `.
    Append,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ReadFileTagsParams {
    #[schemars(description = "Explicit file paths to read")]
    pub paths: Option<Vec<String>>,

    #[schemars(description = "Resolve file paths from Rekordbox track IDs")]
    pub track_ids: Option<Vec<String>>,

    #[schemars(description = "Scan directory for audio files")]
    pub directory: Option<String>,

    #[schemars(
        description = "Glob filter within directory (default: all audio files). Only used with directory."
    )]
    pub glob: Option<String>,

    #[schemars(description = "Scan subdirectories (default: false). Only used with directory.")]
    pub recursive: Option<bool>,

    #[schemars(
        description = "Return only these fields (default: all). Valid: artist, title, album, album_artist, genre, year, track, disc, comment, publisher, bpm, key, composer, remixer"
    )]
    pub fields: Option<Vec<String>>,

    #[schemars(description = "Include cover art metadata (default: false)")]
    pub include_cover_art: Option<bool>,

    #[schemars(description = "Max files to read (default: 200, max: 2000)")]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct WriteFileTagsParams {
    #[schemars(description = "Array of write operations")]
    pub writes: Vec<WriteFileTagsEntry>,

    #[schemars(description = "Preview changes without writing (default: false)")]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub(in crate::mcp) struct WriteFileTagsEntry {
    #[schemars(description = "Path to the audio file")]
    pub path: String,

    #[schemars(
        description = "Tag fields to write. Keys are field names, values are strings to set or null to delete."
    )]
    pub tags: HashMap<String, Option<String>>,

    #[schemars(
        description = "WAV only: which tag layers to write (default: both). Values: \"id3v2\", \"riff_info\""
    )]
    pub wav_targets: Option<Vec<WavTarget>>,

    #[schemars(
        description = "How to merge the comment field with any existing value: replace (default), prepend, append. Uses ' | ' as separator."
    )]
    pub comment_mode: Option<CommentMode>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct ExtractCoverArtParams {
    #[schemars(description = "Path to the audio file")]
    pub path: String,

    #[schemars(
        description = "Where to save the extracted art (default: cover.{ext} in same directory)"
    )]
    pub output_path: Option<String>,

    #[schemars(
        description = "Which art to extract (default: front_cover). Accepted exact values: other, icon, other_icon, front_cover, cover_front, back_cover, cover_back, leaflet, media, lead_artist, artist, conductor, band, composer, lyricist, recording_location, during_recording, during_performance, screen_capture, bright_fish, illustration, band_logo, publisher_logo. Unknown values are rejected. cover_front and cover_back are compatibility aliases for front_cover and back_cover."
    )]
    pub picture_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(in crate::mcp) struct EmbedCoverArtParams {
    #[schemars(description = "Path to the image file")]
    pub image_path: String,

    #[schemars(description = "Audio files to embed art into")]
    #[serde(rename = "targets")]
    pub target_audio_files: Vec<String>,

    #[schemars(
        description = "Picture type (default: front_cover). Accepted exact values: other, icon, other_icon, front_cover, cover_front, back_cover, cover_back, leaflet, media, lead_artist, artist, conductor, band, composer, lyricist, recording_location, during_recording, during_performance, screen_capture, bright_fish, illustration, band_logo, publisher_logo. Unknown values are rejected. cover_front and cover_back are compatibility aliases for front_cover and back_cover."
    )]
    pub picture_type: Option<String>,
}
