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
    #[schemars(description = "Filter by whether track has a label set")]
    pub has_label: Option<bool>,
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
        description = "Set to true to acknowledge that label research is complete and bypass the label gate. Required when backfill_labels reported unlabeled tracks. Only set this after completing step 3 (label research) of the metadata backfill SOP."
    )]
    pub skip_label_gate: Option<bool>,
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
    #[schemars(description = "Offset for pagination (skip first N tracks in result set)")]
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
    #[schemars(description = "Offset for pagination (skip first N tracks in result set)")]
    pub offset: Option<u32>,
    #[schemars(description = "Skip tracks already in cache (default true)")]
    pub skip_cached: Option<bool>,
    #[schemars(
        description = "Max concurrent track analyses (default: CPU cores - 2, min 1, max 4)"
    )]
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ClassifyTracksParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to classify (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Classify tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to classify (default 50, max 200)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N tracks)")]
    pub offset: Option<u32>,
    #[schemars(
        description = "Genre overrides: remap a genre string before scoring. Example: [{\"from\": \"Melodic House & Techno\", \"to\": \"Deep Techno\"}]"
    )]
    pub genre_overrides: Option<Vec<GenreOverrideInput>>,
    #[schemars(
        description = "Response format: 'full' (default) returns evidence, candidates, flags, and review hints. 'compact' returns only track_id, artist, title, genre, confidence, action — use when classifying all tracks upfront before dispatching review subagents. 'summary' returns only confidence distribution and genre-grouped counts with no per-track results — use to get the lay of the land before deciding what to stage."
    )]
    pub format: Option<ClassifyFormat>,
    #[schemars(
        description = "Auto-stage results at these confidence levels after classification. Example: [\"high\", \"medium\"]. Only results with a recommended genre are staged. Omit to classify without staging (default)."
    )]
    pub auto_stage: Option<Vec<StageLevel>>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ClassifyFormat {
    #[default]
    Full,
    Compact,
    Summary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum StageLevel {
    High,
    Medium,
    Low,
    Insufficient,
}

impl StageLevel {
    pub fn matches_confidence(&self, conf: &crate::classify::ClassificationConfidence) -> bool {
        use crate::classify::ClassificationConfidence;
        matches!(
            (self, conf),
            (Self::High, ClassificationConfidence::High)
                | (Self::Medium, ClassificationConfidence::Medium)
                | (Self::Low, ClassificationConfidence::Low)
                | (Self::Insufficient, ClassificationConfidence::Insufficient)
        )
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AuditGenresParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to audit (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Audit tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to audit (default 50, max 200)")]
    pub max_tracks: Option<u32>,
    #[schemars(description = "Offset for pagination (skip first N tracks)")]
    pub offset: Option<u32>,
    #[schemars(
        description = "Include confirmed tracks (genre matches evidence) in results (default false)"
    )]
    pub include_confirmed: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(inline)]
pub struct GenreOverrideInput {
    #[schemars(description = "Source genre string to match (case-insensitive)")]
    pub from: String,
    #[schemars(description = "Target canonical genre to use instead")]
    pub to: String,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ResolveFormat {
    #[default]
    Full,
    Classification,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum SequencingPriority {
    Balanced,
    Harmonic,
    Energy,
    Genre,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum HarmonicMixingStyle {
    Conservative,
    Balanced,
    Adventurous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum EnergyPhase {
    Warmup,
    Build,
    Peak,
    Release,
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum EnergyCurvePreset {
    WarmupBuildPeakRelease,
    #[serde(rename = "flat")]
    FlatEnergy,
    PeakOnly,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
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
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'harmonic', 'energy', 'genre'), preset with overrides ({preset: 'harmonic', overrides: {energy: 0.25}}), or full custom weights ({key: 0.3, bpm: 0.2, ...}). Default: balanced."
    )]
    pub priority: Option<TransitionWeightSpec>,
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
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'harmonic', 'energy', 'genre'), preset with overrides, or custom weights. Default: balanced."
    )]
    pub priority: Option<TransitionWeightSpec>,
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
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'harmonic', 'energy', 'genre'), preset with overrides, or custom weights. Default: balanced."
    )]
    pub priority: Option<TransitionWeightSpec>,
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

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum PoolPreset {
    #[default]
    Balanced,
    Timbral,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum ScorerType {
    #[default]
    Pool,
    Transition,
}

/// Weight input for transition scoring axes. All fields optional —
/// missing fields inherit from the base preset. Auto-renormalized.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(deny_unknown_fields)]
pub struct TransitionWeightInput {
    pub key: Option<f64>,
    pub bpm: Option<f64>,
    pub energy: Option<f64>,
    pub genre: Option<f64>,
    pub brightness: Option<f64>,
    pub rhythm: Option<f64>,
}

/// Weight input for pool scoring axes. All fields optional —
/// missing fields inherit from the base preset. Auto-renormalized.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(deny_unknown_fields)]
pub struct PoolWeightInput {
    pub bpm: Option<f64>,
    pub energy: Option<f64>,
    pub timbral: Option<f64>,
    pub key: Option<f64>,
    pub genre: Option<f64>,
    pub brightness: Option<f64>,
    pub rhythm: Option<f64>,
}

/// Flexible weight specification: a preset name, preset with overrides, or full custom weights.
///
/// Examples:
/// - `"balanced"` — built-in or saved preset by name
/// - `{"preset": "balanced", "overrides": {"timbral": 0.35}}` — preset with axis overrides
/// - `{"bpm": 0.25, "energy": 0.20, ...}` — fully custom weights
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(untagged)]
pub enum TransitionWeightSpec {
    WithOverrides {
        preset: String,
        overrides: Option<TransitionWeightInput>,
    },
    Custom(TransitionWeightInput),
    Named(String),
}

/// Flexible weight specification for pool scoring.
///
/// Examples:
/// - `"balanced"` — built-in or saved preset by name
/// - `{"preset": "timbral", "overrides": {"timbral": 0.4}}` — preset with axis overrides
/// - `{"bpm": 0.25, "energy": 0.20, ...}` — fully custom weights
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(untagged)]
pub enum PoolWeightSpec {
    WithOverrides {
        preset: String,
        overrides: Option<PoolWeightInput>,
    },
    Custom(PoolWeightInput),
    Named(String),
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScorePoolCompatibilityParams {
    #[schemars(
        description = "First track ID (pairwise mode). Provide track_a + track_b for pairwise scoring."
    )]
    pub track_a: Option<String>,
    #[schemars(description = "Second track ID (pairwise mode)")]
    pub track_b: Option<String>,
    #[schemars(
        description = "Single track ID to score against a pool (one-vs-pool mode). Provide with pool_track_ids."
    )]
    pub track_id: Option<String>,
    #[schemars(
        description = "Pool track IDs. Used with track_id for one-vs-pool mode, or alone for cohesion mode."
    )]
    pub pool_track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Master Tempo mode (default false). When true, keys are fixed regardless of BPM adjustment."
    )]
    pub master_tempo: Option<bool>,
    #[schemars(
        description = "Reference BPM for key evaluation when master_tempo=false. Defaults to median BPM of tracks being scored."
    )]
    pub reference_bpm: Option<f64>,
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'timbral'), preset with overrides ({preset: 'timbral', overrides: {timbral: 0.4}}), or custom weights. Default: balanced."
    )]
    pub preset: Option<PoolWeightSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExpandPoolParams {
    #[schemars(description = "Seed track IDs that define the initial pool")]
    pub seed_track_ids: Vec<String>,
    #[schemars(description = "Number of tracks to add (default 3)")]
    pub additions: Option<u32>,
    #[schemars(
        description = "Master Tempo mode (default false). When true, keys are fixed regardless of BPM adjustment."
    )]
    pub master_tempo: Option<bool>,
    #[schemars(
        description = "Reference BPM for key evaluation when master_tempo=false. Defaults to median BPM of seeds."
    )]
    pub reference_bpm: Option<f64>,
    #[schemars(
        description = "Allow cross-genre discovery (default false). When true, disables genre family pre-filter."
    )]
    pub cross_genre: Option<bool>,
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'timbral'), preset with overrides ({preset: 'timbral', overrides: {timbral: 0.4}}), or custom weights. Default: balanced."
    )]
    pub preset: Option<PoolWeightSpec>,
    #[schemars(description = "Use tracks from this playlist as candidate universe")]
    pub playlist_id: Option<String>,
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Max candidate tracks to consider from search (default: no limit)")]
    pub max_tracks: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribePoolParams {
    #[schemars(
        description = "Track IDs in the pool (takes precedence over playlist_id if both provided)"
    )]
    pub pool_track_ids: Option<Vec<String>>,
    #[schemars(
        description = "Playlist ID to use as the pool (ignored if pool_track_ids provided)"
    )]
    pub playlist_id: Option<String>,
    #[schemars(
        description = "Master Tempo mode (default false). When true, keys are fixed regardless of BPM adjustment."
    )]
    pub master_tempo: Option<bool>,
    #[schemars(
        description = "Reference BPM for key evaluation when master_tempo=false. Defaults to median BPM."
    )]
    pub reference_bpm: Option<f64>,
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'timbral'), preset with overrides ({preset: 'timbral', overrides: {timbral: 0.4}}), or custom weights. Default: balanced."
    )]
    pub preset: Option<PoolWeightSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DiscoverPoolsParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Specific track IDs to analyze (highest priority selector)")]
    pub track_ids: Option<Vec<String>>,
    #[schemars(description = "Analyze tracks in this playlist")]
    pub playlist_id: Option<String>,
    #[schemars(description = "Max tracks to analyze (default 200)")]
    pub max_tracks: Option<u32>,
    #[schemars(
        description = "Compatibility threshold for graph edges (0.0-1.0, default 0.7). Higher = tighter pools, fewer results."
    )]
    pub threshold: Option<f64>,
    #[schemars(description = "Min pool size (default 3, min 2)")]
    pub min_pool_size: Option<u32>,
    #[schemars(description = "Max pool size (default 12)")]
    pub max_pool_size: Option<u32>,
    #[schemars(description = "Max pools to return (default 10)")]
    pub max_pools: Option<u32>,
    #[schemars(
        description = "Master Tempo mode (default false). When true, keys are fixed regardless of BPM adjustment."
    )]
    pub master_tempo: Option<bool>,
    #[schemars(
        description = "Reference BPM for key evaluation when master_tempo=false. Defaults to median BPM."
    )]
    pub reference_bpm: Option<f64>,
    #[schemars(
        description = "Scoring weights. Named preset ('balanced', 'timbral'), preset with overrides ({preset: 'timbral', overrides: {timbral: 0.4}}), or custom weights. Default: balanced."
    )]
    pub preset: Option<PoolWeightSpec>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveWeightPresetParams {
    #[schemars(description = "Name for the preset (e.g. 'deep_techno_pool')")]
    pub name: String,
    #[schemars(description = "Scorer type: 'pool' or 'transition'")]
    pub scorer_type: ScorerType,
    #[schemars(
        description = "Weight values. For pool: {bpm, energy, timbral, key, genre, brightness, rhythm}. For transition: {key, bpm, energy, genre, brightness, rhythm}. Auto-renormalized to sum to 1.0."
    )]
    pub weights: serde_json::Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWeightPresetsParams {
    #[schemars(description = "Filter by scorer type: 'pool' or 'transition'. Omit for all.")]
    pub scorer_type: Option<ScorerType>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteWeightPresetParams {
    #[schemars(description = "Name of the preset to delete")]
    pub name: String,
    #[schemars(description = "Scorer type: 'pool' or 'transition'")]
    pub scorer_type: ScorerType,
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
#[schemars(inline)]
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

    #[schemars(
        description = "How to merge the comment field with any existing value: replace (default), prepend, append. Uses ' | ' as separator."
    )]
    pub comment_mode: Option<tags::CommentMode>,
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

#[derive(Debug, Deserialize)]
#[serde(tag = "operation")]
pub(super) enum AuditOperation {
    #[serde(rename = "scan")]
    Scan {
        #[serde(rename = "scope")]
        path_prefix: String,
        revalidate: Option<bool>,
        skip_issue_types: Option<Vec<String>>,
    },

    #[serde(rename = "query_issues")]
    QueryIssues {
        #[serde(rename = "scope")]
        path_prefix: String,
        status: Option<String>,
        issue_type: Option<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    },

    #[serde(rename = "resolve_issues")]
    ResolveIssues {
        issue_ids: Vec<i64>,
        resolution: String,
        note: Option<String>,
    },

    #[serde(rename = "get_summary")]
    GetSummary {
        #[serde(rename = "scope")]
        path_prefix: String,
    },
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct ScanBrokenLinksParams {
    #[schemars(description = "Scope to tracks whose file path starts with this prefix")]
    pub path_prefix: Option<String>,
    #[schemars(
        description = "Attempt case-insensitive filename matching for relocations (default true)"
    )]
    pub suggest_relocations: Option<bool>,
    #[schemars(description = "Max broken links to report (default 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination")]
    pub offset: Option<u32>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct ScanOrphanFilesParams {
    #[schemars(description = "Directory to scan (default: content roots from library)")]
    pub path_prefix: Option<String>,
    #[schemars(description = "Max orphan files to report (default 200)")]
    pub limit: Option<u32>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct ScanPlaylistCoverageParams {
    #[serde(flatten)]
    pub filters: SearchFilterParams,
    #[schemars(description = "Max uncovered tracks to return (default 200)")]
    pub limit: Option<u32>,
    #[schemars(description = "Offset for pagination")]
    pub offset: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub(super) enum DuplicateDetectionLevel {
    /// Byte-identical file matching via SHA-256 hash
    Exact,
    /// Match by artist + title (case-insensitive)
    #[default]
    Metadata,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub(super) struct ScanDuplicatesParams {
    #[schemars(description = "Detection level: 'metadata' (default) or 'exact' (SHA-256 hash)")]
    pub detection_level: Option<DuplicateDetectionLevel>,
    #[schemars(description = "Scope to tracks whose file path starts with this prefix")]
    pub path_prefix: Option<String>,
    #[schemars(description = "Max duplicate groups to report (default 50)")]
    pub limit: Option<u32>,
}

impl schemars::JsonSchema for AuditOperation {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("AuditOperation")
    }

    fn json_schema(_gen: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "required": ["operation"],
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["scan", "query_issues", "resolve_issues", "get_summary"],
                    "description": "The audit operation to perform"
                },
                "scope": {
                    "type": "string",
                    "description": "Directory path prefix (required for scan, query_issues, get_summary)"
                },
                "revalidate": {
                    "type": "boolean",
                    "description": "Re-read all files including unchanged (default: false). Only for scan."
                },
                "skip_issue_types": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Issue types to exclude from detection (e.g. [\"GENRE_SET\"]). Only for scan."
                },
                "status": {
                    "type": "string",
                    "description": "Filter by status: open | resolved | accepted | deferred. Only for query_issues."
                },
                "issue_type": {
                    "type": "string",
                    "description": "Filter by issue type (e.g. WAV_TAG3_MISSING). Only for query_issues."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results (default: 100). Only for query_issues."
                },
                "offset": {
                    "type": "integer",
                    "description": "Offset for pagination (default: 0). Only for query_issues."
                },
                "issue_ids": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Issue IDs to resolve. Required for resolve_issues."
                },
                "resolution": {
                    "type": "string",
                    "description": "Resolution: accepted_as_is | wont_fix | deferred. Required for resolve_issues."
                },
                "note": {
                    "type": "string",
                    "description": "Optional user comment. Only for resolve_issues."
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::ClassificationConfidence;

    #[test]
    fn stage_level_matches_only_corresponding_confidence() {
        let cases = [
            (StageLevel::High, ClassificationConfidence::High, true),
            (StageLevel::High, ClassificationConfidence::Medium, false),
            (StageLevel::High, ClassificationConfidence::Low, false),
            (
                StageLevel::High,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (StageLevel::Medium, ClassificationConfidence::High, false),
            (StageLevel::Medium, ClassificationConfidence::Medium, true),
            (StageLevel::Medium, ClassificationConfidence::Low, false),
            (
                StageLevel::Medium,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (StageLevel::Low, ClassificationConfidence::High, false),
            (StageLevel::Low, ClassificationConfidence::Medium, false),
            (StageLevel::Low, ClassificationConfidence::Low, true),
            (
                StageLevel::Low,
                ClassificationConfidence::Insufficient,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::High,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Medium,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Low,
                false,
            ),
            (
                StageLevel::Insufficient,
                ClassificationConfidence::Insufficient,
                true,
            ),
        ];
        for (level, conf, expected) in cases {
            assert_eq!(
                level.matches_confidence(&conf),
                expected,
                "{level:?} vs {conf:?}"
            );
        }
    }
}
