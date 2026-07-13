//! Compatibility facade for classification audio profiles.

#![allow(unused_imports)]

pub(crate) use crate::adapters::state::classification::{load_from_db, save_to_db};
pub(crate) use crate::domain::classification::profiles::{
    AudioAffinity, AudioScoreResult, FeatureContribution, FeatureStat, GenrePrototype, MIN_TRACKS,
    ProfileRegistry, calibrate, format_evidence, score_all,
};
