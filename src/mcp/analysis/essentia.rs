//! Compatibility façade for the Essentia audio adapter.

#![allow(unused_imports)]

pub(crate) use crate::adapters::audio::{
    ESSENTIA_IMPORT_CHECK_SCRIPT, ESSENTIA_PROBE_TIMEOUT_SECS, ESSENTIA_PYTHON_ENV_VAR,
    ESSENTIA_VENV_RELPATH, essentia_setup_hint, essentia_venv_dir,
    probe_essentia_python_from_sources, probe_essentia_python_path, validate_essentia_python,
    validate_essentia_python_with_timeout,
};
