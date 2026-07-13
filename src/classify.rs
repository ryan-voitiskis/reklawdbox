//! Compatibility facade for the classification domain engine.

#![allow(unused_imports)]

#[cfg(test)]
pub(crate) use crate::domain::classification::engine::classify_track;
pub(crate) use crate::domain::classification::engine::classify_track_with_profiles;
pub(crate) use crate::domain::classification::{
    AudioFeatures, ClassificationAction, ClassificationConfidence, ClassificationResult,
    CompactClassificationResult, GenreCandidate, MappedGenre, TrackEvidence,
};
