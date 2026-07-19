//! Managed, reproducible Essentia runtime discovery and installation.
//!
//! The stable `essentia-venv` entry point is a symlink selected only after a
//! complete immutable generation has passed the exact runtime probe.

mod activation;
mod contract;
mod install;
mod platform;
mod process;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use self::activation::ESSENTIA_VENV_GENERATIONS;
#[allow(unused_imports)]
pub(crate) use self::contract::{
    ESSENTIA_CONTRACT_ID, ESSENTIA_PROBE_TIMEOUT_SECS, ESSENTIA_PYTHON_ENV_VAR, EssentiaRuntime,
    EssentiaSetupError, EssentiaSetupErrorKind, SUPPORTED_ESSENTIA_MODULE_VERSION,
    SUPPORTED_ESSENTIA_VERSION, SUPPORTED_NUMPY_VERSION, SUPPORTED_PYTHON_PREFIX,
    SUPPORTED_PYYAML_VERSION, SUPPORTED_SIX_VERSION, essentia_venv_dir, inspect_essentia_python,
    probe_essentia_python_path, probe_essentia_runtime_path,
};
#[allow(unused_imports)]
pub(crate) use self::contract::{ESSENTIA_VENV_RELPATH, probe_essentia_runtime_from_sources};
#[cfg(test)]
pub(crate) use self::contract::{
    probe_essentia_python_from_sources, validate_essentia_python_with_timeout,
};
#[allow(unused_imports)]
pub(crate) use self::install::{ManagedEssentiaInstall, install_managed_essentia};
