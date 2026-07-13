//! Compatibility façade for the read-only Rekordbox persistence adapter.
//!
//! Canonical implementations live under [`crate::adapters::rekordbox`].

#![allow(unused_imports)]

pub(crate) use crate::adapters::rekordbox::{
    DuplicateGroup, MetadataDuplicatePage, PlaylistCoverageResult, SAMPLER_PATH_FRAGMENT,
    SearchParams, TRACK_COLUMNS, TRACK_JOINS, TRACK_SELECT, TrackPathEntry, active_track_count,
    all_track_paths, content_roots, default_db_path, escape_like, find_metadata_duplicates_page,
    get_library_stats, get_library_stats_filtered, get_play_stats, get_playlist_tracks,
    get_playlist_tracks_page, get_playlist_tracks_unbounded, get_playlist_tracks_unbounded_page,
    get_playlists, get_session_tracks, get_sessions, get_track, get_tracks_by_exact_genre,
    get_tracks_by_ids, non_sampler_track_count, open, paths_imported_in_scope,
    playlist_membership_counts, resolve_db_path, row_to_track, search_tracks,
    search_tracks_unbounded, tracks_not_in_any_playlist, validate_iso_date,
};
#[cfg(test)]
pub(crate) use crate::adapters::rekordbox::{find_metadata_duplicates, open_real_db, open_test};
