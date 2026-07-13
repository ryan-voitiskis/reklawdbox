use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::library::SearchFilterParams;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupDiscogsParams {
    #[schemars(description = "Track ID — auto-fills artist/title/album from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(description = "Album/release title for more accurate matching")]
    pub album: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupBeatportParams {
    #[schemars(description = "Track ID — auto-fills artist/title from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupMusicBrainzParams {
    #[schemars(description = "Track ID — auto-fills artist/title from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LookupBandcampParams {
    #[schemars(description = "Track ID — auto-fills artist/title from library")]
    pub track_id: Option<String>,
    #[schemars(description = "Artist name (required if no track_id)")]
    pub artist: Option<String>,
    #[schemars(description = "Track title (required if no track_id)")]
    pub title: Option<String>,
    #[schemars(
        description = "Direct Bandcamp /track/ or /album/ URL. Bypasses search and refreshes the artist/title cache entry."
    )]
    pub url: Option<String>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EnrichTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to enrich (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Enrich tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(
        description = "Max pending tracks to enrich after cache checks (default 50, max 200)"
    )]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Index into the stable underlying selector order (default 0)")]
    pub offset: Option<u32>,
    #[schemars(
        description = "Providers to use: 'discogs', 'beatport', 'bandcamp' (default ['discogs'])"
    )]
    pub providers: Option<Vec<crate::types::Provider>>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
    #[schemars(description = "Max concurrent enrichments (default 4, max 8)")]
    pub concurrency: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveTrackDataParams {
    #[schemars(description = "Track ID to resolve")]
    pub track_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveTracksDataParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to resolve (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Resolve tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to resolve (default 50)")]
    pub max_tracks: Option<u32>,
    #[schemars(
        description = "Response format: 'full' (default) or 'classification' (compact, only decision-tree fields)"
    )]
    pub format: Option<ResolveFormat>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ResolveFormat {
    #[default]
    Full,
    Classification,
}
