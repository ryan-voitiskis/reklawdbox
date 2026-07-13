//! Read-only access to the encrypted Rekordbox library database.
//!
//! ANLZ beat-grid parsing and the `AnalysisDataPath` lookup live in [`anlz`].

pub(crate) mod anlz;
mod connection;
mod health;
mod history;
mod playlists;
mod tracks;

#[cfg(test)]
pub(crate) use connection::{
    REKORDBOX_SQLCIPHER_KEY, open_real_db, open_test, resolve_db_path_from,
};
pub(crate) use connection::{default_db_path, open, resolve_db_path};
#[cfg(test)]
pub(crate) use health::find_metadata_duplicates;
pub(crate) use health::{
    DuplicateGroup, MetadataDuplicatePage, PlaylistCoverageResult, TrackPathEntry,
    active_track_count, all_track_paths, find_metadata_duplicates_page, non_sampler_track_count,
    paths_imported_in_scope, playlist_membership_counts, tracks_not_in_any_playlist,
};
pub(crate) use history::{get_play_stats, get_session_tracks, get_sessions};
pub(crate) use playlists::{
    get_playlist_tracks, get_playlist_tracks_page, get_playlist_tracks_unbounded,
    get_playlist_tracks_unbounded_page, get_playlists,
};
pub(crate) use tracks::{
    SAMPLER_PATH_FRAGMENT, SearchParams, TRACK_COLUMNS, TRACK_JOINS, TRACK_SELECT, content_roots,
    escape_like, get_library_stats, get_library_stats_filtered, get_track,
    get_tracks_by_exact_genre, get_tracks_by_ids, row_to_track, search_tracks,
    search_tracks_unbounded, validate_iso_date,
};
#[cfg(test)]
pub(crate) use tracks::{decode_rating_stars, is_sampler_path, next_day};

#[cfg(test)]
mod tests;
