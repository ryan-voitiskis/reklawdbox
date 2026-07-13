//! Audio analyzer adapters.
//!
//! Decoding, Stratum, Essentia, and filesystem discovery are reusable from
//! either transport and do not depend on MCP or CLI types.

mod decode;
mod error;
mod essentia;
mod scan;
mod stratum;
pub(crate) mod tags;

#[cfg(test)]
pub(crate) use decode::downmix_to_mono;
pub(crate) use decode::{decode_to_samples, resolve_audio_path};
pub(crate) use error::AudioError;
#[cfg(test)]
pub(crate) use essentia::parse_essentia_stdout;
pub(crate) use essentia::{
    ESSENTIA_IMPORT_CHECK_SCRIPT, ESSENTIA_PROBE_TIMEOUT_SECS, ESSENTIA_PYTHON_ENV_VAR,
    ESSENTIA_VENV_RELPATH, EssentiaOutput, essentia_setup_hint, essentia_venv_dir,
    probe_essentia_python_from_sources, probe_essentia_python_path, run_essentia,
    validate_essentia_python, validate_essentia_python_with_timeout,
};
pub(crate) use scan::scan_audio_directory;
#[cfg(test)]
pub(crate) use stratum::stratum_notation_to_camelot;
pub(crate) use stratum::{StratumResult, TrackSectionView, analyze_with_stratum};

pub(crate) const AUDIO_EXTENSIONS: &[&str] = &["flac", "wav", "mp3", "m4a", "aac", "aiff"];

/// DB cache key for stratum-dsp.
pub(crate) const ANALYZER_STRATUM: &str = "stratum-dsp";
/// DB cache key for Essentia.
pub(crate) const ANALYZER_ESSENTIA: &str = "essentia";

/// Expected analysis schema versions. Bump these only when analyzer output
/// fields change; this architectural move intentionally preserves both.
pub(crate) const STRATUM_SCHEMA_VERSION: &str = "21";
pub(crate) const ESSENTIA_SCHEMA_VERSION: &str = "2";
pub(crate) const STRATUM_HMM_INPUT_FINGERPRINT: &str = "hmm:v1";

#[cfg(test)]
mod tests;
