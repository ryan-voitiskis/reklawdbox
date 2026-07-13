//! Compatibility façade for the shared analysis application and audio adapters.
//!
//! Canonical flow: `CLI/MCP -> application::analysis -> adapters + domain`.

#![allow(unused_imports)]

pub(crate) use crate::adapters::audio::{
    ANALYZER_ESSENTIA, ANALYZER_STRATUM, AUDIO_EXTENSIONS, AudioError, ESSENTIA_SCHEMA_VERSION,
    EssentiaOutput, STRATUM_HMM_INPUT_FINGERPRINT, STRATUM_SCHEMA_VERSION, StratumResult,
    TrackSectionView, analyze_with_stratum, decode_to_samples, resolve_audio_path, run_essentia,
};
pub(crate) use crate::adapters::rekordbox::anlz::load_rekordbox_grid_for_path;
pub(crate) use crate::application::analysis::identity::{
    analyze_with_stratum_input, load_rekordbox_grid_input_for_path,
    load_rekordbox_grid_inputs_for_paths,
};
pub(crate) use crate::application::analysis::model::{RekordboxGridInput, StratumAnalysis};
