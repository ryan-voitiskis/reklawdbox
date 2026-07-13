use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTracksParams {
    #[schemars(description = "Array of track changes to stage")]
    pub changes: Vec<TrackChangeInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct TrackChangeInput {
    #[schemars(description = "Track ID")]
    pub track_id: String,
    #[schemars(description = "New genre")]
    pub genre: Option<String>,
    #[schemars(description = "New comments")]
    pub comments: Option<String>,
    #[schemars(description = "New star rating (1-5)")]
    pub rating: Option<u8>,
    #[schemars(description = "New color name")]
    pub color: Option<String>,
    #[schemars(description = "New label (record label)")]
    pub label: Option<String>,
    #[schemars(description = "Release year")]
    pub year: Option<i32>,
    #[schemars(description = "Album name")]
    pub album: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct WriteXmlPlaylistInput {
    #[schemars(description = "Playlist name")]
    pub name: String,
    #[schemars(description = "Track IDs in playlist order")]
    pub track_ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteXmlParams {
    #[schemars(
        description = "Output file path (default: ~/reklawdbox-exports/reklawdbox-{timestamp}.xml)"
    )]
    pub output_path: Option<String>,
    #[schemars(
        description = "Optional playlist exports. Each playlist includes a name and ordered track_ids."
    )]
    pub playlists: Option<Vec<WriteXmlPlaylistInput>>,
    #[schemars(
        description = "Set to true to acknowledge that label research is complete and bypass the label gate. Required when backfill_labels reported unlabeled tracks. Only set this after completing Step 1c (label research) of the metadata backfill SOP."
    )]
    pub skip_label_gate: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum PreviewFormat {
    #[default]
    Full,
    Summary,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewChangesParams {
    #[schemars(description = "Filter to specific track IDs (if empty, shows all staged changes)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Response format: 'full' (default) returns per-track diffs. 'summary' returns aggregate counts by field and genre — use for verification before write_xml."
    )]
    pub format: Option<PreviewFormat>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearChangesParams {
    #[schemars(
        description = "Track IDs to clear (if omitted, clears all; an empty array clears nothing)"
    )]
    pub track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Specific fields to unstage: \"genre\", \"comments\", \"rating\", \"color\", \"label\", \"year\", \"album\". If omitted, clears all fields (removes entire entries)."
    )]
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestNormalizationsParams {
    #[schemars(description = "Only show genres with at least this many tracks (default 1)")]
    #[serde(rename = "min_count")]
    pub min_genre_count: Option<i32>,
    #[schemars(
        description = "Auto-stage all alias normalizations (default false). When true, non-debatable alias mappings (e.g. 'Hip-Hop' → 'Hip Hop') are staged immediately."
    )]
    pub stage_aliases: Option<bool>,
}
