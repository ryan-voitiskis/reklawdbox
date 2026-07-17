//! Audio analyzer adapters.
//!
//! Decoding, Stratum, Essentia, and filesystem discovery are reusable from
//! either transport and do not depend on MCP or CLI types.

mod decode;
mod error;
mod essentia;
mod essentia_environment;
mod scan;
mod stratum;
pub(crate) mod tags;

#[cfg(test)]
pub(crate) use decode::downmix_to_mono;
pub(crate) use decode::{decode_to_samples, resolve_audio_path};
pub(crate) use error::AudioError;
pub(crate) use essentia::{EssentiaOutput, essentia_setup_hint, run_essentia};
#[cfg(test)]
pub(crate) use essentia::{parse_essentia_stdout, validate_runtime_manifest};
pub(crate) use essentia_environment::{
    EssentiaRuntime, EssentiaSetupError, inspect_essentia_python, install_managed_essentia,
    probe_essentia_python_path, probe_essentia_runtime_path,
};
#[cfg(test)]
pub(crate) use essentia_environment::{
    EssentiaSetupErrorKind, probe_essentia_python_from_sources,
    validate_essentia_python_with_timeout,
};
pub(crate) use scan::scan_audio_directory;
#[cfg(test)]
pub(crate) use stratum::TrackSectionView;
#[cfg(test)]
pub(crate) use stratum::stratum_notation_to_camelot;
pub(crate) use stratum::{StratumResult, analyze_with_stratum};

pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["flac", "wav", "mp3", "m4a", "aac", "aiff"];

/// DB cache key for stratum-dsp.
pub(crate) const ANALYZER_STRATUM: &str = "stratum-dsp";
/// DB cache key for Essentia.
pub(crate) const ANALYZER_ESSENTIA: &str = "essentia";

/// Expected analysis schema versions. Bump these only when analyzer output
/// fields change; this architectural move intentionally preserves both.
pub(crate) const STRATUM_SCHEMA_VERSION: &str = "21";
pub(crate) const ESSENTIA_SCHEMA_VERSION: &str = "3";
pub(crate) const STRATUM_HMM_INPUT_FINGERPRINT: &str = "hmm:v1";

#[cfg(test)]
mod tests;
