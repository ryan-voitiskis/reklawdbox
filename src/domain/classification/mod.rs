//! Neutral classification records shared by classification engines.
//!
//! Classification domain code must not depend on application infrastructure.

mod model;

pub(crate) use model::{
    AudioFeatures, ClassificationAction, ClassificationConfidence, ClassificationResult,
    CompactClassificationResult, GenreCandidate, MappedGenre, TrackEvidence,
};
