mod analysis;
mod audit;
mod classification;
mod context;
mod enrichment;
mod error;
mod files;
mod help;
mod library;
mod metadata;
mod planning;
mod server;

pub use server::ReklawdboxServer;

// Capability imports remain private to the MCP transport boundary.
use analysis::{
    AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, AudioCacheIdentity, BatchProgress,
    ESSENTIA_IMPORT_CHECK_SCRIPT, audio_cache_identities_with_current_stratum_input,
    check_analysis_cache, essentia_setup_hint, essentia_venv_dir, get_fresh_analysis_entry,
    resolve_file_path, validate_essentia_python,
};
use audit::AuditOperation;
use enrichment::{
    BatchPage, EnrichTracksParams, LookupBandcampParams, LookupBeatportParams, LookupDiscogsParams,
    LookupMusicBrainzParams, OffsetPage, ResolveFormat, ResolveTrackDataParams,
    ResolveTracksDataParams, ResolveTracksOpts, apply_offset_limit, auth_remediation_message,
    lookup_bandcamp_remote, lookup_discogs_remote, lookup_musicbrainz_remote, offset_page_bounds,
    resolve_pending_tracks, resolve_tracks, track_has_unknown_genre,
};
use error::{cache_error, db_error, mcp_internal_error, ok_json, ok_structured_json};
use files::{EmbedCoverArtParams, ExtractCoverArtParams, ReadFileTagsParams, WriteFileTagsParams};
use library::{GetPlaylistTracksParams, GetTrackParams, SearchFilterParams, SearchTracksParams};
use metadata::{
    ClearChangesParams, PreviewChangesParams, PreviewFormat, SuggestNormalizationsParams,
    UpdateTracksParams, WriteXmlParams,
};
use planning::{
    BuildSetParams, DeleteWeightPresetParams, DescribePoolParams, DiscoverPoolsParams,
    ExpandPoolParams, HarmonicMixingStyle, ListWeightPresetsParams, PoolAxisScoresPresentation,
    PoolCohesionResult, PoolPreset, PoolWeightInput, QueryTransitionCandidatesParams,
    SaveWeightPresetParams, ScorePoolCompatibilityParams, ScoreTransitionParams, ScorerType,
    SequencingPriority, TrackProfile, TransitionScoresPresentation, TransitionWeightInput,
    format_camelot, map_genre_through_taxonomy, resolve_pool_weights, resolve_transition_weights,
    round_to_3_decimals, transpose_camelot_key,
};

#[cfg(test)]
mod eval_scoring;
#[cfg(test)]
mod tests;
