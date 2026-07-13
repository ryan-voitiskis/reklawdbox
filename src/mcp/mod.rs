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
    AnalyzeAudioBatchParams, AnalyzeTrackAudioParams, BatchProgress, resolve_file_path,
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
    ExpandPoolParams, HarmonicMixingStyle, ListWeightPresetsParams, PoolPreset, PoolWeightInput,
    QueryTransitionCandidatesParams, SaveWeightPresetParams, ScorePoolCompatibilityParams,
    ScoreTransitionParams, ScorerType, SequencingPriority, TransitionWeightInput,
    resolve_pool_weights, resolve_transition_weights,
};

#[cfg(test)]
mod tests;
