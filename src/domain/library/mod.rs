//! Rekordbox library records and rating conversions.
//!
//! This module owns stable library concepts and has no infrastructure
//! dependencies.

mod model;
mod rating;

pub(crate) use model::{
    FileKind, GenreCount, KeyCount, LibraryStats, Playlist, Session, Track, TrackPlayStats,
};
pub(crate) use rating::{rating_to_stars, stars_to_rating};
