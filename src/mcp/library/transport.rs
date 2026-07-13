use schemars::JsonSchema;
use serde::Deserialize;

use crate::db;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct SearchFilterParams {
    #[schemars(description = "Search query matching title or artist")]
    pub query: Option<String>,
    #[schemars(description = "Filter by artist name (partial match)")]
    pub artist: Option<String>,
    #[schemars(description = "Filter by genre name (partial match)")]
    pub genre: Option<String>,
    #[schemars(description = "Minimum star rating (1-5)")]
    pub rating_min: Option<u8>,
    #[schemars(description = "Minimum BPM")]
    pub bpm_min: Option<f64>,
    #[schemars(description = "Maximum BPM")]
    pub bpm_max: Option<f64>,
    #[schemars(description = "Filter by musical key (e.g. 'Am', 'Cm')")]
    pub key: Option<String>,
    #[schemars(description = "Filter by whether track has a genre set")]
    pub has_genre: Option<bool>,
    #[schemars(description = "Filter by whether track has a label set")]
    pub has_label: Option<bool>,
    #[schemars(
        description = "Filter to tracks with a non-canonical genre (not in taxonomy and no known alias). Tracks with empty genre are excluded. Only true is meaningful; false or omitted means no filtering."
    )]
    pub has_unknown_genre: Option<bool>,
    #[schemars(
        description = "Filter to tracks with year = 0 (unset). Useful for targeting year-zero tracks for enrichment."
    )]
    pub year_zero: Option<bool>,
    #[schemars(description = "Filter by label name (partial match)")]
    pub label: Option<String>,
    #[schemars(description = "Filter by file path/folder (substring match)")]
    pub path: Option<String>,
    #[schemars(
        description = "Filter to tracks whose file path starts with this prefix (directory scoping)"
    )]
    pub path_prefix: Option<String>,
    #[schemars(
        description = "Only tracks added on or after this date (ISO date, e.g. '2026-01-01')"
    )]
    pub added_after: Option<String>,
    #[schemars(
        description = "Only tracks added on or before this date (ISO date, e.g. '2026-12-31')"
    )]
    pub added_before: Option<String>,
}

impl SearchFilterParams {
    pub(crate) fn into_search_params(
        self,
        exclude_samples: bool,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<db::SearchParams, String> {
        let added_after = self
            .added_after
            .map(|s| db::validate_iso_date(&s, "added_after"))
            .transpose()?;
        let added_before = self
            .added_before
            .map(|s| db::validate_iso_date(&s, "added_before"))
            .transpose()?;

        Ok(db::SearchParams {
            query: self.query,
            artist: self.artist,
            genre: self.genre,
            rating_min: self.rating_min,
            bpm_min: self.bpm_min,
            bpm_max: self.bpm_max,
            key: self.key,
            playlist: None,
            has_genre: self.has_genre,
            has_label: self.has_label,
            year_zero: self.year_zero,
            label: self.label,
            path: self.path,
            path_prefix: self.path_prefix,
            added_after,
            added_before,
            exclude_samples,
            limit,
            offset,
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Filter by playlist ID")]
    pub playlist: Option<String>,
    #[schemars(description = "Include Rekordbox factory samples (default false)")]
    pub include_samples: Option<bool>,
    #[schemars(description = "Max results (default 50, max 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N results)")]
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTrackParams {
    #[schemars(description = "Track ID")]
    pub track_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPlaylistTracksParams {
    #[schemars(description = "Playlist ID")]
    pub playlist_id: String,
    #[schemars(description = "Max results (default 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSessionsParams {
    #[schemars(description = "Max sessions to return (default 20, max 100)")]
    pub limit: Option<u32>,
    #[schemars(description = "Only sessions on or after this date (ISO date, e.g. '2024-01-01')")]
    pub after: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSessionTracksParams {
    #[schemars(description = "Session ID from get_sessions")]
    pub session_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetPlayStatsParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(
        description = "Also return tracks matching filters that have never been played in any session (default false)"
    )]
    pub include_unplayed: Option<bool>,
    #[schemars(description = "Max results (default 200, max 500)")]
    pub limit: Option<u32>,
}
