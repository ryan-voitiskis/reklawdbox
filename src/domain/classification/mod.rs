//! Neutral classification records shared by classification engines.
//!
//! Classification domain code must not depend on application infrastructure.

pub(crate) mod engine;
mod model;
pub(crate) mod profiles;
pub(crate) mod taxonomy;

pub(crate) use model::{
    AudioBackendStatus, AudioFeatures, ClassificationAction, ClassificationConfidence,
    ClassificationDegradedReason, ClassificationMode, ClassificationResult, DiscogsMatchQuality,
    DiscogsReadiness, GenreCandidate, LabelProvenance, MappedGenre, TrackEvidence,
    classification_readiness,
};
