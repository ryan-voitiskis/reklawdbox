//! Planning records shared by sequencing and pool scoring.

use crate::domain::classification::taxonomy::GenreFamily;
use crate::domain::library::Track;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SequencingPriority {
    Balanced,
    Harmonic,
    Energy,
    Genre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HarmonicMixingStyle {
    Conservative,
    Balanced,
    Adventurous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnergyPhase {
    Warmup,
    Build,
    Peak,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnergyCurvePreset {
    WarmupBuildPeakRelease,
    FlatEnergy,
    PeakOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnergyCurve {
    Preset(EnergyCurvePreset),
    Custom(Vec<EnergyPhase>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum PoolPreset {
    #[default]
    Balanced,
    Timbral,
}

#[derive(Debug, Clone)]
pub(crate) struct TimbralFeatures {
    pub(crate) mfcc_mean: Vec<f64>,
    pub(crate) mfcc_std: Vec<f64>,
    pub(crate) spectral_contrast_mean: Vec<f64>,
    pub(crate) spectral_centroid_cv: f64,
    pub(crate) dissonance_mean: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct TrackProfile {
    pub(crate) track: Track,
    pub(crate) camelot_key: Option<CamelotKey>,
    pub(crate) key_display: String,
    pub(crate) bpm: f64,
    pub(crate) energy: f64,
    pub(crate) brightness: Option<f64>,
    pub(crate) rhythm_regularity: Option<f64>,
    pub(crate) loudness_range: Option<f64>,
    pub(crate) canonical_genre: Option<String>,
    pub(crate) genre_family: GenreFamily,
    pub(crate) timbral: Option<TimbralFeatures>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CamelotKey {
    pub(crate) number: u8,
    pub(crate) letter: char,
}

#[derive(Debug, Clone)]
pub(crate) struct AxisScore {
    pub(crate) value: f64,
    pub(crate) label: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ScoreAdjustment {
    pub(crate) kind: &'static str,
    pub(crate) delta: f64,
    pub(crate) composite_without: f64,
    pub(crate) reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TransitionScores {
    pub(crate) key: AxisScore,
    pub(crate) bpm: AxisScore,
    pub(crate) energy: AxisScore,
    pub(crate) genre: AxisScore,
    pub(crate) brightness: AxisScore,
    pub(crate) rhythm: AxisScore,
    pub(crate) composite: f64,
    pub(crate) effective_to_key: Option<String>,
    pub(crate) pitch_shift_semitones: i32,
    pub(crate) key_relation: String,
    pub(crate) bpm_adjustment_pct: f64,
    pub(crate) adjustments: Vec<ScoreAdjustment>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ScoringContext {
    pub(crate) genre_run_length: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidateTransition {
    pub(crate) from_index: usize,
    pub(crate) to_index: usize,
    pub(crate) scores: TransitionScores,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidatePlan {
    pub(crate) ordered_ids: Vec<String>,
    pub(crate) transitions: Vec<CandidateTransition>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PriorityWeights {
    pub(crate) key: f64,
    pub(crate) bpm: f64,
    pub(crate) energy: f64,
    pub(crate) genre: f64,
    pub(crate) brightness: f64,
    pub(crate) rhythm: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolWeights {
    pub(crate) bpm: f64,
    pub(crate) energy: f64,
    pub(crate) timbral: f64,
    pub(crate) key: f64,
    pub(crate) genre: f64,
    pub(crate) brightness: f64,
    pub(crate) rhythm: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolAxisScores {
    pub(crate) key: AxisScore,
    pub(crate) bpm: AxisScore,
    pub(crate) energy: AxisScore,
    pub(crate) genre: AxisScore,
    pub(crate) brightness: AxisScore,
    pub(crate) rhythm: AxisScore,
    pub(crate) timbral: Option<AxisScore>,
    pub(crate) composite: f64,
}

#[derive(Debug, Clone)]
pub(crate) struct CandidatePoolScore {
    pub(crate) min_score: f64,
    pub(crate) mean_score: f64,
    pub(crate) per_member: Vec<(String, PoolAxisScores)>,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolCohesionResult {
    pub(crate) mean_pairwise: f64,
    pub(crate) min_pairwise: f64,
    pub(crate) weakest_member_id: Option<String>,
    pub(crate) medoid_id: Option<String>,
    pub(crate) per_pair: Vec<(String, String, PoolAxisScores)>,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredPool {
    pub(crate) track_ids: Vec<String>,
    pub(crate) mean_compatibility: f64,
    pub(crate) min_compatibility: f64,
    pub(crate) core_members: Vec<String>,
    pub(crate) edge_members: Vec<String>,
    pub(crate) score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TimbralNormalization {
    pub(crate) dims: Vec<(f64, f64)>,
    pub(crate) sample_count: i64,
}
