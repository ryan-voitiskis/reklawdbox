pub(super) mod pools;
mod presets;
pub(super) mod scoring;
mod sequencing;
mod transport;
mod weights;

pub(super) use pools::{
    handle_describe_pool, handle_discover_pools, handle_expand_pool,
    handle_score_pool_compatibility,
};
pub(super) use presets::{
    handle_delete_weight_preset, handle_list_weight_presets, handle_save_weight_preset,
};
pub(super) use scoring::{
    PoolAxisScoresPresentation, PoolCohesionResult, TrackProfile, TransitionScoresPresentation,
    format_camelot, map_genre_through_taxonomy, round_to_3_decimals, transpose_camelot_key,
};
pub(super) use sequencing::{
    handle_build_set, handle_query_transition_candidates, handle_score_transition,
};
pub(super) use transport::{
    BuildSetParams, DeleteWeightPresetParams, DescribePoolParams, DiscoverPoolsParams,
    EnergyCurveInput, EnergyCurvePreset, EnergyPhase, ExpandPoolParams, HarmonicMixingStyle,
    ListWeightPresetsParams, PoolPreset, PoolWeightInput, PoolWeightSpec,
    QueryTransitionCandidatesParams, SaveWeightPresetParams, ScorePoolCompatibilityParams,
    ScoreTransitionParams, ScorerType, SequencingPriority, TransitionWeightInput,
    TransitionWeightSpec,
};
pub(super) use weights::{resolve_pool_weights, resolve_transition_weights};

// Plan 046 splits the legacy monolithic tests; retain their compatibility surface until then.
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use pools::sweep_optimal_reference_bpm;
#[cfg(test)]
#[allow(unused_imports)]
pub(super) use scoring::{
    AxisScore, CamelotKey, CandidatePlan, CandidatePoolScore, CandidateTransition, DiscoveredPool,
    GenreFamily, PoolAxisScores, PoolWeights, PriorityWeights, ScoreAdjustment, ScoringContext,
    TIMBRAL_VECTOR_SCHEMA_VERSION, TimbralFeatures, TimbralSourceSnapshot, TransitionScores,
    bpm_pitch_shift, build_candidate_plan, build_candidate_plan_beam, build_timbral_vector,
    build_track_profile, build_track_profiles, composite_score, compute_bpm_trajectory,
    compute_pool_cohesion, compute_timbral_norm_stats, compute_track_energy, discover_pools,
    ensure_timbral_norm_stats, find_bridge_tracks, genre_family_for, key_to_camelot,
    load_timbral_source_snapshot, load_timbral_source_snapshot_for_test, musical_key_to_camelot,
    normalize_timbral_vector, parse_camelot_key, pool_weights, priority_weights,
    resolve_energy_curve, score_bpm_axis, score_candidate_vs_pool, score_energy_axis,
    score_genre_axis, score_key_axis, score_key_with_pitch_shifts, score_pool_bpm_axis,
    score_pool_compatibility_pair, score_pool_energy_axis, score_pool_genre_axis,
    score_pool_timbral_axis, score_transition_profiles, select_start_track_ids,
};
