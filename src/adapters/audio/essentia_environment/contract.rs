//! Exact Essentia runtime identity, probing, and source priority.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::process::{
    CommandRequest, CommandRunner, ProcessError, ProcessErrorKind, SystemCommandRunner,
};

pub(crate) const ESSENTIA_PYTHON_ENV_VAR: &str = "CRATE_DIG_ESSENTIA_PYTHON";
pub(crate) const ESSENTIA_VENV_RELPATH: &str = ".local/share/reklawdbox/essentia-venv";
pub(crate) const ESSENTIA_PROBE_TIMEOUT_SECS: u64 = 30;
pub(crate) const ESSENTIA_CONTRACT_ID: &str =
    "essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14";
pub(crate) const SUPPORTED_PYTHON_PREFIX: &str = "3.14.";
pub(crate) const SUPPORTED_ESSENTIA_VERSION: &str = "2.1b6.dev1438";
pub(crate) const SUPPORTED_ESSENTIA_MODULE_VERSION: &str = "2.1-beta6-dev";
pub(crate) const SUPPORTED_NUMPY_VERSION: &str = "2.5.1";
pub(crate) const SUPPORTED_PYYAML_VERSION: &str = "6.0.3";
pub(crate) const SUPPORTED_SIX_VERSION: &str = "1.17.0";

/// Emits one JSON object so arbitrary version text cannot look compatible.
pub(crate) const ESSENTIA_IMPORT_CHECK_SCRIPT: &str = r#"import importlib.metadata as metadata, json, platform, sys
import essentia, numpy, yaml, six
print(json.dumps({"python": platform.python_version(), "implementation": sys.implementation.name, "essentia": metadata.version("essentia"), "essentia_module": essentia.__version__, "numpy": numpy.__version__, "pyyaml": yaml.__version__, "six": six.__version__}))"#;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EssentiaRuntime {
    pub python_path: String,
    pub python_version: String,
    pub essentia_version: String,
    pub essentia_module_version: String,
    pub numpy_version: String,
    pub pyyaml_version: String,
    pub six_version: String,
    pub analyzer_contract: String,
}

/// Machine-distinguishable setup failures. Transport layers may render these
/// for humans, but installer policy and failure classification stay here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EssentiaSetupErrorKind {
    LockTimeout,
    CandidateNotFound,
    VenvCreation,
    WheelUnavailable,
    PipFailure,
    ImportFailure,
    ManifestMismatch,
    ProbeTimeout,
    Filesystem,
    Activation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EssentiaSetupError {
    pub kind: EssentiaSetupErrorKind,
    pub message: String,
}

impl EssentiaSetupError {
    pub(crate) fn new(kind: EssentiaSetupErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EssentiaSetupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.kind, self.message)
    }
}

impl std::error::Error for EssentiaSetupError {}

impl From<String> for EssentiaSetupError {
    fn from(message: String) -> Self {
        Self::new(EssentiaSetupErrorKind::Filesystem, message)
    }
}

impl From<&str> for EssentiaSetupError {
    fn from(message: &str) -> Self {
        Self::new(EssentiaSetupErrorKind::Filesystem, message)
    }
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    python: String,
    implementation: String,
    essentia: String,
    essentia_module: String,
    numpy: String,
    pyyaml: String,
    six: String,
}

pub(crate) fn essentia_venv_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(ESSENTIA_VENV_RELPATH))
}

pub(crate) fn inspect_essentia_python(path: &str) -> Option<EssentiaRuntime> {
    inspect_essentia_python_with_timeout_result(
        path,
        Duration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS),
    )
    .ok()
}

#[cfg(test)]
pub(crate) fn validate_essentia_python_with_timeout(path: &str, timeout: Duration) -> bool {
    inspect_essentia_python_with_timeout_result(path, timeout).is_ok()
}

pub(super) fn inspect_essentia_python_with_timeout_result(
    path: &str,
    timeout: Duration,
) -> Result<EssentiaRuntime, EssentiaSetupError> {
    inspect_essentia_python_with_runner(&SystemCommandRunner::default(), path, timeout)
}

pub(super) fn inspect_essentia_python_with_runner(
    runner: &dyn CommandRunner,
    path: &str,
    timeout: Duration,
) -> Result<EssentiaRuntime, EssentiaSetupError> {
    let args = ["-c".to_string(), ESSENTIA_IMPORT_CHECK_SCRIPT.to_string()];
    let output = runner
        .run(CommandRequest {
            program: path,
            args: &args,
            timeout,
        })
        .map_err(|error| probe_process_error(path, timeout, error))?;
    if !output.success {
        return Err(EssentiaSetupError::new(
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "runtime probe imports failed for {path}{}",
                diagnostic_text(&String::from_utf8_lossy(&output.stderr), "")
            ),
        ));
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout).map_err(|error| {
        EssentiaSetupError::new(
            EssentiaSetupErrorKind::ImportFailure,
            format!("runtime probe for {path} returned invalid JSON: {error}"),
        )
    })?;
    if probe.implementation != "cpython"
        || !probe.python.starts_with(SUPPORTED_PYTHON_PREFIX)
        || probe.essentia != SUPPORTED_ESSENTIA_VERSION
        || probe.essentia_module != SUPPORTED_ESSENTIA_MODULE_VERSION
        || probe.numpy != SUPPORTED_NUMPY_VERSION
        || probe.pyyaml != SUPPORTED_PYYAML_VERSION
        || probe.six != SUPPORTED_SIX_VERSION
    {
        return Err(EssentiaSetupError::new(
            EssentiaSetupErrorKind::ManifestMismatch,
            format!(
                "runtime at {path} does not match the supported manifest (implementation={}, python={}, essentia_distribution={}, essentia_module={}, numpy={}, pyyaml={}, six={})",
                probe.implementation,
                probe.python,
                probe.essentia,
                probe.essentia_module,
                probe.numpy,
                probe.pyyaml,
                probe.six
            ),
        ));
    }
    Ok(EssentiaRuntime {
        python_path: path.to_string(),
        python_version: probe.python,
        essentia_version: probe.essentia,
        essentia_module_version: probe.essentia_module,
        numpy_version: probe.numpy,
        pyyaml_version: probe.pyyaml,
        six_version: probe.six,
        analyzer_contract: ESSENTIA_CONTRACT_ID.to_string(),
    })
}

pub(super) fn probe_process_error(
    path: &str,
    timeout: Duration,
    error: ProcessError,
) -> EssentiaSetupError {
    let (kind, message) = match error.kind {
        ProcessErrorKind::Start(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed to start runtime probe for {path}: {source}"),
        ),
        ProcessErrorKind::MissingCapture(stream) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "failed to capture runtime probe {} for {path}",
                stream.name()
            ),
        ),
        ProcessErrorKind::ReaderStart { stream, source } => (
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "failed to start runtime probe {} reader for {path}: {source}",
                stream.name()
            ),
        ),
        ProcessErrorKind::Wait(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed while waiting for runtime probe {path}: {source}"),
        ),
        ProcessErrorKind::Timeout | ProcessErrorKind::ReaderShutdownTimeout => (
            EssentiaSetupErrorKind::ProbeTimeout,
            format!(
                "runtime probe for {path} timed out after {} seconds",
                timeout.as_secs_f64()
            ),
        ),
        ProcessErrorKind::ReaderRead { stream, source } => (
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "runtime probe {} reader failed for {path}: {source}",
                stream.name()
            ),
        ),
        ProcessErrorKind::ReaderPanicked(stream) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("runtime probe {} reader panicked for {path}", stream.name()),
        ),
        ProcessErrorKind::ProcessGroup(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed while waiting for runtime probe {path}: {source:?}"),
        ),
        ProcessErrorKind::SurvivingDescendants => (
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "failed while waiting for runtime probe {path}: process left surviving descendants"
            ),
        ),
    };
    EssentiaSetupError::new(kind, message)
}

pub(super) fn generic_process_error(
    program: &str,
    timeout: Duration,
    error: ProcessError,
) -> EssentiaSetupError {
    let (kind, message) = match error.kind {
        ProcessErrorKind::Start(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed to start {program}: {source}"),
        ),
        ProcessErrorKind::MissingCapture(stream) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed to capture {} for {program}", stream.name()),
        ),
        ProcessErrorKind::ReaderStart { stream, source } => (
            EssentiaSetupErrorKind::ImportFailure,
            format!(
                "failed to start {} reader for {program}: {source}",
                stream.name()
            ),
        ),
        ProcessErrorKind::Wait(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed while waiting for {program}: {source}"),
        ),
        ProcessErrorKind::Timeout | ProcessErrorKind::ReaderShutdownTimeout => (
            EssentiaSetupErrorKind::ProbeTimeout,
            format!(
                "{program} timed out after {} seconds",
                timeout.as_secs_f64()
            ),
        ),
        ProcessErrorKind::ReaderRead { stream, source } => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("{} reader failed for {program}: {source}", stream.name()),
        ),
        ProcessErrorKind::ReaderPanicked(stream) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("{} reader panicked for {program}", stream.name()),
        ),
        ProcessErrorKind::ProcessGroup(source) => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed while waiting for {program}: {source:?}"),
        ),
        ProcessErrorKind::SurvivingDescendants => (
            EssentiaSetupErrorKind::ImportFailure,
            format!("failed while waiting for {program}: process left surviving descendants"),
        ),
    };
    EssentiaSetupError::new(kind, message)
}

#[cfg(test)]
pub(super) fn inspect_essentia_python_with_timeout(
    path: &str,
    timeout: Duration,
) -> Option<EssentiaRuntime> {
    inspect_essentia_python_with_timeout_result(path, timeout).ok()
}

pub(crate) fn probe_essentia_runtime_from_sources(
    env_override: Option<&str>,
    default_candidate: Option<PathBuf>,
) -> Option<EssentiaRuntime> {
    let mut candidates = Vec::new();
    if let Some(path) = env_override.map(str::trim).filter(|path| !path.is_empty()) {
        candidates.push(path.to_string());
    }
    if let Some(path) = default_candidate {
        let path = path.to_string_lossy().to_string();
        if !path.is_empty() && !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    candidates.into_iter().find_map(|path| {
        inspect_essentia_python_with_timeout_result(
            &path,
            Duration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS),
        )
        .ok()
    })
}

pub(crate) fn probe_essentia_runtime_path() -> Option<EssentiaRuntime> {
    let override_path = std::env::var(ESSENTIA_PYTHON_ENV_VAR).ok();
    let default = essentia_venv_dir().map(|dir| dir.join("bin/python"));
    probe_essentia_runtime_from_sources(override_path.as_deref(), default)
}

#[cfg(test)]
pub(crate) fn probe_essentia_python_from_sources(
    env_override: Option<&str>,
    default_candidate: Option<PathBuf>,
) -> Option<String> {
    probe_essentia_runtime_from_sources(env_override, default_candidate)
        .map(|runtime| runtime.python_path)
}

pub(crate) fn probe_essentia_python_path() -> Option<String> {
    probe_essentia_runtime_path().map(|runtime| runtime.python_path)
}

pub(super) fn diagnostic_text(stderr: &str, stdout: &str) -> String {
    let stderr = stderr.trim();
    let stdout = stdout.trim();
    match (stderr.is_empty(), stdout.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(": {stderr}"),
        (true, false) => format!(": {stdout}"),
        (false, false) => format!(": {stderr}\n{stdout}"),
    }
}
