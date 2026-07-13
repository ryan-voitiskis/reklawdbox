//! Compatibility façade for domain library and metadata types.
//!
//! `Provider` remains here temporarily until Plan 041 moves enrichment
//! application vocabulary to its canonical owner.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) use crate::domain::library::{
    FileKind, GenreCount, KeyCount, LibraryStats, Playlist, Session, Track, TrackPlayStats,
    rating_to_stars, stars_to_rating,
};
#[allow(unused_imports)]
pub(crate) use crate::domain::metadata::{EditableField, FieldDiff, TrackChange, TrackDiff};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[schemars(inline)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Discogs,
    Beatport,
    Bandcamp,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discogs => "discogs",
            Self::Beatport => "beatport",
            Self::Bandcamp => "bandcamp",
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
