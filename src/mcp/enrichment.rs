mod cache;
mod core;
pub(super) mod discogs_auth;
mod handlers;
mod resolve;
mod resolve_handlers;
mod transport;

pub(super) use cache::HasScore;
pub(super) use core::auth_remediation_message;
pub(super) use discogs_auth::lookup_discogs_remote;
pub(super) use handlers::{
    EnrichTracksOutput, handle_enrich_tracks, handle_lookup_bandcamp, handle_lookup_discogs,
    handle_lookup_musicbrainz, lookup_bandcamp_remote, lookup_musicbrainz_remote,
};
pub(super) use resolve::{
    BatchPage, OffsetPage, ResolveTracksOpts, apply_offset_limit, describe_resolve_scope,
    offset_page_bounds, resolve_pending_tracks, resolve_tracks, to_percent,
    track_has_unknown_genre,
};
pub(super) use resolve_handlers::{handle_resolve_track_data, handle_resolve_tracks_data};

#[cfg(test)]
pub(super) use core::{
    lookup_output_with_cache_metadata, set_test_bandcamp_lookup_override,
    set_test_discogs_lookup_override, set_test_musicbrainz_lookup_override,
};
#[cfg(test)]
pub(super) use discogs_auth::{
    DiscogsAuthTestDependencies, InMemoryDiscogsSessionPersistence,
    resolve_discogs_auth_transition_for_test,
};
#[cfg(test)]
pub(super) use resolve::pending_batch_page;
#[cfg(test)]
pub(super) use resolve_handlers::resolve_single_track;
pub(super) use transport::{
    EnrichTracksParams, LookupBandcampParams, LookupDiscogsParams, LookupMusicBrainzParams,
    ResolveFormat, ResolveTrackDataParams, ResolveTracksDataParams,
};
