//! Transport-neutral managed Essentia setup workflow.

use crate::adapters::audio::{
    EssentiaRuntime, EssentiaSetupError, inspect_essentia_python, install_managed_essentia,
    probe_essentia_runtime_path,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EssentiaSetupStatus {
    AlreadyInstalled,
    Installed,
}

#[derive(Debug, Clone)]
pub(crate) struct EssentiaSetupResult {
    pub status: EssentiaSetupStatus,
    pub runtime: EssentiaRuntime,
    pub python_bin_used: Option<String>,
}

pub(crate) fn setup_essentia() -> Result<EssentiaSetupResult, EssentiaSetupError> {
    setup_essentia_with_candidate(None)
}

pub(crate) fn setup_essentia_with_candidate(
    existing_candidate: Option<&str>,
) -> Result<EssentiaSetupResult, EssentiaSetupError> {
    if let Some(runtime) = existing_candidate.and_then(inspect_essentia_python) {
        return Ok(EssentiaSetupResult {
            status: EssentiaSetupStatus::AlreadyInstalled,
            runtime,
            python_bin_used: None,
        });
    }
    if let Some(runtime) = probe_essentia_runtime_path() {
        return Ok(EssentiaSetupResult {
            status: EssentiaSetupStatus::AlreadyInstalled,
            runtime,
            python_bin_used: None,
        });
    }
    let installed = install_managed_essentia()?;
    let status = if installed.python_bin_used.is_some() {
        EssentiaSetupStatus::Installed
    } else {
        EssentiaSetupStatus::AlreadyInstalled
    };
    Ok(EssentiaSetupResult {
        status,
        runtime: installed.runtime,
        python_bin_used: installed.python_bin_used,
    })
}
