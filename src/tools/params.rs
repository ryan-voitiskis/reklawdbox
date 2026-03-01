use std::collections::HashMap;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::db;
use crate::tags;

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
    ) -> db::SearchParams {
        db::SearchParams {
            query: self.query,
            artist: self.artist,
            genre: self.genre,
            rating_min: self.rating_min,
            bpm_min: self.bpm_min,
            bpm_max: self.bpm_max,
            key: self.key,
            playlist: None,
            has_genre: self.has_genre,
            label: self.label,
            path: self.path,
            path_prefix: self.path_prefix,
            added_after: self.added_after,
            added_before: self.added_before,
            exclude_samples,
            limit,
            offset,
        }
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
pub struct UpdateTracksParams {
    #[schemars(description = "Array of track changes to stage")]
    pub changes: Vec<TrackChangeInput>,
}

#[derive(Debug, Deserialize, JsonSchema)]
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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteXmlPlaylistInput {
    #[schemars(description = "Playlist name")]
    pub name: String,
    #[schemars(description = "Track IDs in playlist order")]
    pub track_ids: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteXmlParams {
    #[schemars(
        description = "Output file path (default: ./rekordbox-exports/reklawdbox-{timestamp}.xml)"
    )]
    pub output_path: Option<String>,
    #[schemars(
        description = "Optional playlist exports. Each playlist includes a name and ordered track_ids."
    )]
    pub playlists: Option<Vec<WriteXmlPlaylistInput>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PreviewChangesParams {
    #[schemars(description = "Filter to specific track IDs (if empty, shows all staged changes)")]
    pub track_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClearChangesParams {
    #[schemars(description = "Track IDs to clear (if empty, clears all)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Specific fields to unstage: \"genre\", \"comments\", \"rating\", \"color\". If omitted, clears all fields (removes entire entries)."
    )]
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SuggestNormalizationsParams {
    #[schemars(description = "Only show genres with at least this many tracks (default 1)")]
    #[serde(rename = "min_count")]
    pub min_genre_count: Option<i32>,
}

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
pub struct EnrichTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to enrich (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Enrich tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to enrich (default 50)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Providers to use: 'discogs', 'beatport' (default ['discogs'])")]
    pub providers: Option<Vec<crate::types::Provider>>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
    #[schemars(description = "Bypass cache and fetch fresh data (default false)")]
    pub force_refresh: Option<bool>,
}

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
    #[schemars(description = "Max tracks to analyze (default 20)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
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
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SequencingPriority {
    Balanced,
    Harmonic,
    Energy,
    Genre,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HarmonicMixingStyle {
    Conservative,
    Balanced,
    Adventurous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnergyPhase {
    Warmup,
    Build,
    Peak,
    Release,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnergyCurvePreset {
    WarmupBuildPeakRelease,
    #[serde(rename = "flat")]
    FlatEnergy,
    PeakOnly,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EnergyCurveInput {
    Preset(EnergyCurvePreset),
    Custom(Vec<EnergyPhase>),
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BuildSetParams {
    #[schemars(description = "Pool of candidate track IDs (pre-filtered by agent)")]
    pub track_ids: Vec<String>,
    #[schemars(description = "Desired number of tracks in each candidate set")]
    pub target_tracks: u32,
    #[schemars(description = "Weighting axis (balanced, harmonic, energy, genre)")]
    pub priority: Option<SequencingPriority>,
    #[schemars(
        description = "Energy curve: preset name ('warmup_build_peak_release', 'flat', 'peak_only') or an array of phase strings (warmup/build/peak/release), one per target position."
    )]
    pub energy_curve: Option<EnergyCurveInput>,
    #[schemars(description = "Optional track ID to force as the opening track")]
    #[serde(rename = "start_track_id")]
    pub opening_track_id: Option<String>,
    #[schemars(
        description = "Deprecated — use beam_width. Number of set candidates to generate (default 3, max 8)."
    )]
    pub candidates: Option<u8>,
    #[schemars(
        description = "Beam search width: controls how many candidate paths are explored. 1 = greedy (fast), higher = broader search (default 3, max 8). Supersedes 'candidates'."
    )]
    pub beam_width: Option<u8>,
    #[schemars(
        description = "Master Tempo mode (default true). When false, accounts for pitch shift from BPM adjustment when scoring key compatibility."
    )]
    #[serde(rename = "master_tempo")]
    pub use_master_tempo: Option<bool>,
    #[schemars(
        description = "Harmonic mixing style: conservative (strict key matching), balanced (default), adventurous (creative key clashes allowed)."
    )]
    pub harmonic_style: Option<HarmonicMixingStyle>,
    #[schemars(
        description = "Maximum BPM drift from start track as a percentage (default 6.0). The last track may deviate up to this percentage from the opening BPM; intermediate tracks get a proportional fraction."
    )]
    pub bpm_drift_pct: Option<f64>,
    #[schemars(
        description = "BPM range as [start_bpm, end_bpm]. When set, plans a BPM trajectory from start to end across the set's energy curve, and outputs per-track play_at_bpm, pitch_adjustment_pct, and effective_key."
    )]
    pub bpm_range: Option<(f64, f64)>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct QueryTransitionCandidatesParams {
    #[schemars(description = "Source track ID to transition from")]
    #[serde(rename = "from_track_id")]
    pub source_track_id: String,
    #[schemars(description = "Pool of candidate track IDs to rank")]
    #[serde(rename = "pool_track_ids")]
    pub candidate_track_ids: Option<Vec<String>>,
    #[schemars(description = "Playlist ID to use as the candidate pool")]
    pub playlist_id: Option<String>,
    #[schemars(
        description = "Target BPM for the next track. When set, scores how well each candidate fits this BPM target."
    )]
    pub target_bpm: Option<f64>,
    #[schemars(description = "Energy phase preference (warmup, build, peak, release)")]
    pub energy_phase: Option<EnergyPhase>,
    #[schemars(description = "Weighting axis (balanced, harmonic, energy, genre)")]
    pub priority: Option<SequencingPriority>,
    #[schemars(
        description = "Master Tempo mode (default true). When false, accounts for pitch shift from BPM adjustment when scoring key compatibility."
    )]
    #[serde(rename = "master_tempo")]
    pub use_master_tempo: Option<bool>,
    #[schemars(
        description = "Harmonic mixing style: conservative (strict key matching), balanced (default), adventurous (creative key clashes allowed)."
    )]
    pub harmonic_style: Option<HarmonicMixingStyle>,
    #[schemars(description = "Max results to return (default 10, max 50)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScoreTransitionParams {
    #[schemars(description = "Source track ID")]
    #[serde(rename = "from_track_id")]
    pub source_track_id: String,
    #[schemars(description = "Destination track ID")]
    #[serde(rename = "to_track_id")]
    pub target_track_id: String,
    #[schemars(description = "Energy phase preference (warmup, build, peak, release)")]
    pub energy_phase: Option<EnergyPhase>,
    #[schemars(description = "Weighting axis (balanced, harmonic, energy, genre)")]
    pub priority: Option<SequencingPriority>,
    #[schemars(
        description = "Master Tempo mode (default true). When false, accounts for pitch shift from BPM adjustment when scoring key compatibility."
    )]
    #[serde(rename = "master_tempo")]
    pub use_master_tempo: Option<bool>,
    #[schemars(
        description = "Harmonic mixing style: conservative (strict key matching), balanced (default), adventurous (creative key clashes allowed)."
    )]
    pub harmonic_style: Option<HarmonicMixingStyle>,
}

// ---------------------------------------------------------------------------
// Native tag tool params
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ReadFileTagsParams {
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
pub(super) struct WriteFileTagsParams {
    #[schemars(description = "Array of write operations")]
    pub writes: Vec<WriteFileTagsEntry>,

    #[schemars(description = "Preview changes without writing (default: false)")]
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct WriteFileTagsEntry {
    #[schemars(description = "Path to the audio file")]
    pub path: String,

    #[schemars(
        description = "Tag fields to write. Keys are field names, values are strings to set or null to delete."
    )]
    pub tags: HashMap<String, Option<String>>,

    #[schemars(
        description = "WAV only: which tag layers to write (default: both). Values: \"id3v2\", \"riff_info\""
    )]
    pub wav_targets: Option<Vec<tags::WavTarget>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct ExtractCoverArtParams {
    #[schemars(description = "Path to the audio file")]
    pub path: String,

    #[schemars(
        description = "Where to save the extracted art (default: cover.{ext} in same directory)"
    )]
    pub output_path: Option<String>,

    #[schemars(description = "Which art to extract (default: front_cover)")]
    pub picture_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub(super) struct EmbedCoverArtParams {
    #[schemars(description = "Path to the image file")]
    pub image_path: String,

    #[schemars(description = "Audio files to embed art into")]
    #[serde(rename = "targets")]
    pub target_audio_files: Vec<String>,

    #[schemars(description = "Picture type (default: front_cover)")]
    pub picture_type: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "operation")]
pub(super) enum AuditOperation {
    #[serde(rename = "scan")]
    Scan {
        #[schemars(description = "Directory path to audit (trailing / enforced)")]
        #[serde(rename = "scope")]
        path_prefix: String,

        #[schemars(description = "Re-read all files including unchanged (default: false)")]
        revalidate: Option<bool>,

        #[schemars(description = "Issue types to exclude from detection (e.g. [\"GENRE_SET\"])")]
        skip_issue_types: Option<Vec<String>>,
    },

    #[serde(rename = "query_issues")]
    QueryIssues {
        #[schemars(description = "Directory path prefix to filter issues")]
        #[serde(rename = "scope")]
        path_prefix: String,

        #[schemars(description = "Filter by status: open | resolved | accepted | deferred")]
        status: Option<String>,

        #[schemars(description = "Filter by issue type (e.g. WAV_TAG3_MISSING)")]
        issue_type: Option<String>,

        #[schemars(description = "Max results (default: 100)")]
        limit: Option<u32>,

        #[schemars(description = "Offset for pagination (default: 0)")]
        offset: Option<u32>,
    },

    #[serde(rename = "resolve_issues")]
    ResolveIssues {
        #[schemars(description = "Issue IDs to resolve")]
        issue_ids: Vec<i64>,

        #[schemars(description = "Resolution: accepted_as_is | wont_fix | deferred")]
        resolution: String,

        #[schemars(description = "Optional user comment")]
        note: Option<String>,
    },

    #[serde(rename = "get_summary")]
    GetSummary {
        #[schemars(description = "Directory path prefix for summary")]
        #[serde(rename = "scope")]
        path_prefix: String,
    },
}
