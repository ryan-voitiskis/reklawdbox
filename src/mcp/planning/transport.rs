use schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::library::SearchFilterParams;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum SequencingPriority {
    Balanced,
    Harmonic,
    Energy,
    Genre,
}

impl From<SequencingPriority> for crate::domain::planning::SequencingPriority {
    fn from(value: SequencingPriority) -> Self {
        match value {
            SequencingPriority::Balanced => Self::Balanced,
            SequencingPriority::Harmonic => Self::Harmonic,
            SequencingPriority::Energy => Self::Energy,
            SequencingPriority::Genre => Self::Genre,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "snake_case")]
pub enum HarmonicMixingStyle {
    Conservative,
    Balanced,
    Adventurous,
}

impl From<HarmonicMixingStyle> for crate::domain::planning::HarmonicMixingStyle {
    fn from(value: HarmonicMixingStyle) -> Self {
        match value {
            HarmonicMixingStyle::Conservative => Self::Conservative,
            HarmonicMixingStyle::Balanced => Self::Balanced,
            HarmonicMixingStyle::Adventurous => Self::Adventurous,
        }
    }
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

impl From<EnergyPhase> for crate::domain::planning::EnergyPhase {
    fn from(value: EnergyPhase) -> Self {
        match value {
            EnergyPhase::Warmup => Self::Warmup,
            EnergyPhase::Build => Self::Build,
            EnergyPhase::Peak => Self::Peak,
            EnergyPhase::Release => Self::Release,
        }
    }
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

impl From<EnergyCurvePreset> for crate::domain::planning::EnergyCurvePreset {
    fn from(value: EnergyCurvePreset) -> Self {
        match value {
            EnergyCurvePreset::WarmupBuildPeakRelease => Self::WarmupBuildPeakRelease,
            EnergyCurvePreset::FlatEnergy => Self::FlatEnergy,
            EnergyCurvePreset::PeakOnly => Self::PeakOnly,
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(untagged)]
pub enum EnergyCurveInput {
    Preset(EnergyCurvePreset),
    Custom(Vec<EnergyPhase>),
}

impl From<&EnergyCurveInput> for crate::domain::planning::EnergyCurve {
    fn from(value: &EnergyCurveInput) -> Self {
        match value {
            EnergyCurveInput::Preset(preset) => Self::Preset((*preset).into()),
            EnergyCurveInput::Custom(phases) => {
                Self::Custom(phases.iter().copied().map(Into::into).collect())
            }
        }
    }
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

impl From<PoolPreset> for crate::domain::planning::PoolPreset {
    fn from(value: PoolPreset) -> Self {
        match value {
            PoolPreset::Balanced => Self::Balanced,
            PoolPreset::Timbral => Self::Timbral,
        }
    }
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
