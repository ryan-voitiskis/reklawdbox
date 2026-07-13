use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::library::SearchFilterParams;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeTrackAudioParams {
    #[schemars(description = "Track ID to analyze")]
    pub track_id: String,
    #[schemars(description = "Skip if already cached (default true)")]
    pub skip_cached: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnalyzeAudioBatchParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to analyze (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Analyze tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(
        description = "Max pending tracks to analyze after freshness checks (default 20, max 200)"
    )]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Index into the stable underlying selector order (default 0)")]
    pub offset: Option<u32>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
    #[schemars(
        description = "Max concurrent track analyses (default: CPU cores - 2, min 1, max 4)"
    )]
    pub concurrency: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CacheCoverageParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to check (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Check tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to check (default unbounded)")]
    pub max_tracks: Option<u32>,
}
