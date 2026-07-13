//! Compatibility façade for domain library and metadata types.
//!
//! `Provider` remains as a compatibility alias for MCP parameter paths.

pub(crate) use crate::application::enrichment::model::EnrichmentProvider as Provider;
pub(crate) use crate::domain::library::{
    FileKind, GenreCount, KeyCount, LibraryStats, Playlist, Session, Track, TrackPlayStats,
    rating_to_stars, stars_to_rating,
};
#[allow(unused_imports)]
pub(crate) use crate::domain::metadata::{EditableField, FieldDiff, TrackChange, TrackDiff};
