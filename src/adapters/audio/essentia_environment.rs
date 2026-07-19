//! Managed, reproducible Essentia runtime discovery and installation.
//!
//! The stable `essentia-venv` entry point is a symlink selected only after a
//! complete immutable generation has passed the exact runtime probe.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

mod platform;
mod process;

use self::platform::atomic_exchange_paths;
use self::process::{
    CommandRequest, CommandResult, CommandRunner, ProcessError, ProcessErrorKind,
    SystemCommandRunner,
};

pub(crate) const ESSENTIA_PYTHON_ENV_VAR: &str = "CRATE_DIG_ESSENTIA_PYTHON";
pub(crate) const ESSENTIA_VENV_RELPATH: &str = ".local/share/reklawdbox/essentia-venv";
pub(crate) const ESSENTIA_VENV_GENERATIONS: &str = "essentia-venv.generations";
pub(crate) const ESSENTIA_PROBE_TIMEOUT_SECS: u64 = 30;
pub(crate) const ESSENTIA_CONTRACT_ID: &str =
    "essentia:2.1b6.dev1438:numpy:2.5.1:pyyaml:6.0.3:six:1.17.0:cpython:3.14";
pub(crate) const SUPPORTED_PYTHON_PREFIX: &str = "3.14.";
pub(crate) const SUPPORTED_ESSENTIA_VERSION: &str = "2.1b6.dev1438";
pub(crate) const SUPPORTED_ESSENTIA_MODULE_VERSION: &str = "2.1-beta6-dev";
pub(crate) const SUPPORTED_NUMPY_VERSION: &str = "2.5.1";
pub(crate) const SUPPORTED_PYYAML_VERSION: &str = "6.0.3";
pub(crate) const SUPPORTED_SIX_VERSION: &str = "1.17.0";
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_POLL: Duration = Duration::from_millis(100);
const PYTHON_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const VENV_CREATE_TIMEOUT: Duration = Duration::from_secs(120);
const PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
const PYTHON_CANDIDATES: &[&str] = &["python3.14", "python3"];
const PACKAGE_SPECS: &[&str] = &[
    "essentia==2.1b6.dev1438",
    "numpy==2.5.1",
    "PyYAML==6.0.3",
    "six==1.17.0",
];

/// Emits a single JSON object so a probe cannot mistake arbitrary version text
/// for a compatible analyzer runtime.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEssentiaInstall {
    pub runtime: EssentiaRuntime,
    pub python_bin_used: Option<String>,
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

fn inspect_essentia_python_with_timeout_result(
    path: &str,
    timeout: Duration,
) -> Result<EssentiaRuntime, EssentiaSetupError> {
    inspect_essentia_python_with_runner(&SystemCommandRunner::default(), path, timeout)
}

fn inspect_essentia_python_with_runner(
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

fn probe_process_error(path: &str, timeout: Duration, error: ProcessError) -> EssentiaSetupError {
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

fn generic_process_error(
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
fn inspect_essentia_python_with_timeout(path: &str, timeout: Duration) -> Option<EssentiaRuntime> {
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

trait EnvironmentOps {
    fn run(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<CommandResult, EssentiaSetupError>;
    fn inspect_runtime(
        &self,
        python_path: &Path,
        timeout: Duration,
    ) -> Result<EssentiaRuntime, EssentiaSetupError>;
}

struct SystemEnvironmentOps;

impl EnvironmentOps for SystemEnvironmentOps {
    fn run(
        &self,
        program: &str,
        args: &[String],
        timeout: Duration,
    ) -> Result<CommandResult, EssentiaSetupError> {
        SystemCommandRunner::default()
            .run(CommandRequest {
                program,
                args,
                timeout,
            })
            .map_err(|error| generic_process_error(program, timeout, error))
    }

    fn inspect_runtime(
        &self,
        python_path: &Path,
        timeout: Duration,
    ) -> Result<EssentiaRuntime, EssentiaSetupError> {
        inspect_essentia_python_with_runner(
            &SystemCommandRunner::default(),
            &python_path.to_string_lossy(),
            timeout,
        )
    }
}

#[derive(Debug, Clone)]
struct ManagedEnvironmentPaths {
    stable: PathBuf,
    generations: PathBuf,
    lock: PathBuf,
}

impl ManagedEnvironmentPaths {
    fn from_stable(stable: PathBuf) -> Result<Self, String> {
        let parent = stable
            .parent()
            .ok_or("Managed Essentia path has no parent")?;
        Ok(Self {
            generations: parent.join(ESSENTIA_VENV_GENERATIONS),
            lock: parent.join("essentia-venv.lock"),
            stable,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct LockConfig {
    timeout: Duration,
    poll: Duration,
}

pub(crate) fn install_managed_essentia() -> Result<ManagedEssentiaInstall, EssentiaSetupError> {
    let stable = essentia_venv_dir()
        .ok_or("Cannot determine home directory for managed Essentia location")?;
    install_managed_essentia_at(
        &ManagedEnvironmentPaths::from_stable(stable)?,
        &SystemEnvironmentOps,
        LockConfig {
            timeout: LOCK_TIMEOUT,
            poll: LOCK_POLL,
        },
    )
}

fn install_managed_essentia_at(
    paths: &ManagedEnvironmentPaths,
    ops: &dyn EnvironmentOps,
    lock_config: LockConfig,
) -> Result<ManagedEssentiaInstall, EssentiaSetupError> {
    let stable_python = paths.stable.join("bin/python");
    let probe_timeout = Duration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS);
    if let Ok(mut runtime) = ops.inspect_runtime(&stable_python, probe_timeout) {
        runtime.python_path = stable_python.to_string_lossy().into_owned();
        return Ok(ManagedEssentiaInstall {
            runtime,
            python_bin_used: None,
        });
    }

    let parent = paths
        .stable
        .parent()
        .ok_or("Managed Essentia path has no parent")?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create {}: {error}", parent.display()))?;
    let _lock = AdvisoryLock::acquire(&paths.lock, lock_config.timeout, lock_config.poll)?;

    if let Ok(mut runtime) = ops.inspect_runtime(&stable_python, probe_timeout) {
        runtime.python_path = stable_python.to_string_lossy().into_owned();
        return Ok(ManagedEssentiaInstall {
            runtime,
            python_bin_used: None,
        });
    }

    let python = find_python_314(ops)?;
    if fs::symlink_metadata(&paths.generations)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(EssentiaSetupError::new(
            EssentiaSetupErrorKind::Filesystem,
            format!(
                "Managed generation directory must not be a symlink: {}",
                paths.generations.display()
            ),
        ));
    }
    fs::create_dir_all(&paths.generations).map_err(|error| {
        format!(
            "Failed to create managed generation directory {}: {error}",
            paths.generations.display()
        )
    })?;
    let generations_metadata = fs::symlink_metadata(&paths.generations).map_err(|error| {
        format!(
            "Failed to inspect managed generation directory {}: {error}",
            paths.generations.display()
        )
    })?;
    if generations_metadata.file_type().is_symlink() || !generations_metadata.is_dir() {
        return Err(EssentiaSetupError::new(
            EssentiaSetupErrorKind::Filesystem,
            format!(
                "Managed generation path is not a real directory: {}",
                paths.generations.display()
            ),
        ));
    }
    let generation = paths.generations.join(format!(
        "runtime-{}-{}",
        std::process::id(),
        unique_suffix()
    ));
    let mut generation_guard = IncompleteGeneration::new(generation.clone());
    let venv_args = vec![
        "-m".to_string(),
        "venv".to_string(),
        "--copies".to_string(),
        generation.to_string_lossy().into_owned(),
    ];
    run_checked(
        ops,
        &python,
        &venv_args,
        VENV_CREATE_TIMEOUT,
        "managed venv creation",
        InstallCommand::Venv,
    )?;
    let candidate = generation.join("bin/python");
    if !candidate.exists() {
        return Err(EssentiaSetupError::new(
            EssentiaSetupErrorKind::VenvCreation,
            format!(
                "Venv was created but Python is missing at {}",
                candidate.display()
            ),
        ));
    }
    let args = pip_install_args();
    run_checked(
        ops,
        &candidate.to_string_lossy(),
        &args,
        PIP_INSTALL_TIMEOUT,
        "wheel-only Essentia installation",
        InstallCommand::Pip,
    )?;
    ops.inspect_runtime(&candidate, probe_timeout)?;

    let previous = switch_stable_generation(&paths.stable, &generation)
        .map_err(|error| EssentiaSetupError::new(EssentiaSetupErrorKind::Activation, error))?;
    let stable_runtime = ops.inspect_runtime(&stable_python, probe_timeout);
    let mut stable_runtime = match stable_runtime {
        Ok(runtime) => runtime,
        Err(stable_error) => {
            if let Err(restore_error) = restore_stable_generation(&paths.stable, previous) {
                generation_guard.commit();
                return Err(EssentiaSetupError::new(
                    EssentiaSetupErrorKind::Activation,
                    format!(
                        "Validated generation failed through the stable managed path ({stable_error}), and the prior runtime could not be restored; the directly validated generation was preserved: {restore_error}"
                    ),
                ));
            }
            return Err(EssentiaSetupError::new(
                stable_error.kind,
                format!(
                    "Validated generation failed through the stable managed path; previous runtime restored: {}",
                    stable_error.message
                ),
            ));
        }
    };
    stable_runtime.python_path = stable_python.to_string_lossy().to_string();
    generation_guard.commit();
    prune_superseded_runtime(&paths.stable, previous);
    Ok(ManagedEssentiaInstall {
        runtime: stable_runtime,
        python_bin_used: Some(python),
    })
}

fn find_python_314(ops: &dyn EnvironmentOps) -> Result<String, EssentiaSetupError> {
    let mut diagnostics = Vec::new();
    for candidate in PYTHON_CANDIDATES {
        let args = vec![
            "-c".to_string(),
            "import platform, sys; raise SystemExit(0 if platform.python_implementation() == 'CPython' and sys.version_info[:2] == (3, 14) else 1)".to_string(),
        ];
        match ops.run(candidate, &args, PYTHON_CHECK_TIMEOUT) {
            Ok(result) if result.success => return Ok(candidate.to_string()),
            Ok(result) => diagnostics.push(format!(
                "{candidate}: not CPython 3.14{}",
                format_diagnostic_output(&result)
            )),
            Err(error) => diagnostics.push(format!("{candidate}: {error}")),
        }
    }
    Err(EssentiaSetupError::new(
        EssentiaSetupErrorKind::CandidateNotFound,
        format!(
            "No supported CPython 3.14 found. Tried: {}",
            diagnostics.join(", ")
        ),
    ))
}

fn pip_install_args() -> Vec<String> {
    let mut args: Vec<String> = ["-m", "pip", "install", "--only-binary=:all:", "--no-deps"]
        .into_iter()
        .map(str::to_string)
        .collect();
    args.extend(PACKAGE_SPECS.iter().map(|spec| (*spec).to_string()));
    args
}

fn run_checked(
    ops: &dyn EnvironmentOps,
    program: &str,
    args: &[String],
    timeout: Duration,
    context: &str,
    stage: InstallCommand,
) -> Result<(), EssentiaSetupError> {
    let output = ops.run(program, args, timeout).map_err(|error| {
        EssentiaSetupError::new(
            stage.default_error_kind(),
            format!("Failed to run {context}: {error}"),
        )
    })?;
    if output.success {
        return Ok(());
    }
    let kind = if matches!(stage, InstallCommand::Pip) && output_reports_missing_wheel(&output) {
        EssentiaSetupErrorKind::WheelUnavailable
    } else {
        stage.default_error_kind()
    };
    Err(EssentiaSetupError::new(
        kind,
        format!("{context} failed{}", format_diagnostic_output(&output)),
    ))
}

#[derive(Debug, Clone, Copy)]
enum InstallCommand {
    Venv,
    Pip,
}

impl InstallCommand {
    fn default_error_kind(self) -> EssentiaSetupErrorKind {
        match self {
            Self::Venv => EssentiaSetupErrorKind::VenvCreation,
            Self::Pip => EssentiaSetupErrorKind::PipFailure,
        }
    }
}

fn output_reports_missing_wheel(output: &CommandResult) -> bool {
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    diagnostic.contains("no matching distribution found")
        || diagnostic.contains("could not find a version that satisfies")
}

fn format_diagnostic_output(output: &CommandResult) -> String {
    diagnostic_text(
        &String::from_utf8_lossy(&output.stderr),
        &String::from_utf8_lossy(&output.stdout),
    )
}

fn diagnostic_text(stderr: &str, stdout: &str) -> String {
    let stderr = stderr.trim();
    let stdout = stdout.trim();
    match (stderr.is_empty(), stdout.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(": {stderr}"),
        (true, false) => format!(": {stdout}"),
        (false, false) => format!(": {stderr}\n{stdout}"),
    }
}

fn switch_stable_generation(stable: &Path, generation: &Path) -> Result<Option<PathBuf>, String> {
    let parent = stable
        .parent()
        .ok_or("Managed Essentia path has no parent")?;
    let legacy = (stable.exists() && !stable.is_symlink())
        .then(|| parent.join(format!("essentia-venv.legacy-{}", unique_suffix())));
    // For legacy-directory migration the exchange name is already the final
    // preserved name. One atomic exchange is therefore sufficient: there is
    // no vulnerable follow-up rename or second exchange to recover from.
    let replacement = legacy
        .clone()
        .unwrap_or_else(|| parent.join(format!(".essentia-venv-switch-{}", unique_suffix())));
    let previous = if stable.exists() || stable.is_symlink() {
        Some(fs::read_link(stable).unwrap_or_else(|_| stable.to_path_buf()))
    } else {
        None
    };
    let relative = generation.strip_prefix(parent).unwrap_or(generation);
    #[cfg(unix)]
    std::os::unix::fs::symlink(relative, &replacement)
        .map_err(|e| format!("Failed to create managed runtime switch: {e}"))?;
    #[cfg(not(unix))]
    return Err("Managed Essentia generations require Unix symlink support".to_string());
    if let Some(legacy) = legacy {
        if let Err(error) = atomic_exchange_paths(stable, &replacement) {
            let _ = fs::remove_file(&replacement);
            return Err(format!(
                "Failed to atomically activate the managed runtime over the legacy directory: {error}"
            ));
        }
        debug_assert_eq!(replacement, legacy);
        return Ok(Some(legacy));
    }
    if let Err(error) = fs::rename(&replacement, stable) {
        let _ = fs::remove_file(&replacement);
        return Err(format!(
            "Failed to atomically activate validated managed runtime: {error}"
        ));
    }
    Ok(previous)
}

fn restore_stable_generation(stable: &Path, previous: Option<PathBuf>) -> Result<(), String> {
    let parent = stable
        .parent()
        .ok_or("Managed Essentia path has no parent")?;
    match previous {
        Some(previous)
            if previous.file_name().is_some_and(|name| {
                name.to_string_lossy().starts_with("essentia-venv.legacy-")
            }) =>
        {
            atomic_exchange_paths(stable, &previous)
                .map_err(|error| format!("Failed to restore legacy managed runtime: {error}"))?;
            fs::remove_file(&previous).map_err(|error| {
                format!(
                    "Legacy runtime was restored, but the failed runtime link {} could not be removed: {error}",
                    previous.display()
                )
            })?;
        }
        Some(target) => {
            #[cfg(unix)]
            {
                let replacement =
                    parent.join(format!(".essentia-venv-restore-{}", unique_suffix()));
                std::os::unix::fs::symlink(target, &replacement).map_err(|error| {
                    format!("Failed to prepare managed runtime restoration: {error}")
                })?;
                if let Err(error) = fs::rename(&replacement, stable) {
                    let _ = fs::remove_file(&replacement);
                    return Err(format!(
                        "Failed to restore managed runtime symlink: {error}"
                    ));
                }
            }
            #[cfg(not(unix))]
            {
                return Err("Managed Essentia generations require Unix symlink support".to_string());
            }
        }
        None => {
            fs::remove_file(stable)
                .map_err(|error| format!("Failed to remove rejected managed runtime: {error}"))?;
        }
    }
    Ok(())
}

fn prune_superseded_runtime(stable: &Path, previous: Option<PathBuf>) {
    let Some(previous) = previous else {
        return;
    };
    let Some(parent) = stable.parent() else {
        return;
    };
    let path = if previous.is_absolute() {
        previous
    } else {
        parent.join(previous)
    };
    let generations = parent.join(ESSENTIA_VENV_GENERATIONS);
    if fs::symlink_metadata(&generations).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return;
    }
    let Ok(canonical_path) = fs::canonicalize(&path) else {
        return;
    };
    let name = canonical_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let is_managed_generation = fs::canonicalize(&generations).is_ok_and(|root| {
        canonical_path.parent() == Some(root.as_path()) && name.starts_with("runtime-")
    });
    let is_preserved_legacy = fs::canonicalize(parent).is_ok_and(|root| {
        canonical_path.parent() == Some(root.as_path()) && name.starts_with("essentia-venv.legacy-")
    });
    if is_managed_generation || is_preserved_legacy {
        let _ = fs::remove_dir_all(path);
    }
}

struct IncompleteGeneration {
    path: PathBuf,
    committed: bool,
}
impl IncompleteGeneration {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }
    fn commit(&mut self) {
        self.committed = true;
    }
}
impl Drop for IncompleteGeneration {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// A process-owned advisory lock. Unlike a create-new sentinel, the kernel
/// releases this lock when a process exits unexpectedly.
#[derive(Debug)]
struct AdvisoryLock {
    file: File,
}
impl AdvisoryLock {
    fn acquire(path: &Path, timeout: Duration, poll: Duration) -> Result<Self, EssentiaSetupError> {
        let started = Instant::now();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|error| {
                EssentiaSetupError::new(
                    EssentiaSetupErrorKind::Filesystem,
                    format!(
                        "Failed to open managed Essentia setup lock at {}: {error}",
                        path.display()
                    ),
                )
            })?;
        loop {
            if platform::try_lock_exclusive(&file).map_err(|error| {
                EssentiaSetupError::new(
                    EssentiaSetupErrorKind::Filesystem,
                    format!(
                        "Failed to acquire managed Essentia setup lock at {}: {error}",
                        path.display()
                    ),
                )
            })? {
                return Ok(Self { file });
            }
            if started.elapsed() >= timeout {
                return Err(EssentiaSetupError::new(
                    EssentiaSetupErrorKind::LockTimeout,
                    format!(
                        "Timed out waiting for managed Essentia setup lock at {}",
                        path.display()
                    ),
                ));
            }
            std::thread::sleep(poll);
        }
    }
}
impl Drop for AdvisoryLock {
    fn drop(&mut self) {
        if let Err(error) = platform::unlock(&self.file) {
            tracing::warn!("Failed to release managed Essentia setup lock: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCall {
        program: String,
        args: Vec<String>,
    }

    #[derive(Debug, Clone, Copy)]
    struct FakeConfig {
        python314: bool,
        python3: bool,
        venv: bool,
        pip: bool,
        pip_wheel_unavailable: bool,
        direct_probe: bool,
        stable_probe: bool,
    }

    impl Default for FakeConfig {
        fn default() -> Self {
            Self {
                python314: true,
                python3: true,
                venv: true,
                pip: true,
                pip_wheel_unavailable: false,
                direct_probe: true,
                stable_probe: true,
            }
        }
    }

    struct FakeEnvironmentOps {
        paths: ManagedEnvironmentPaths,
        config: FakeConfig,
        calls: Mutex<Vec<RecordedCall>>,
        new_generation: Mutex<Option<PathBuf>>,
    }

    impl FakeEnvironmentOps {
        fn new(paths: ManagedEnvironmentPaths, config: FakeConfig) -> Self {
            Self {
                paths,
                config,
                calls: Mutex::new(Vec::new()),
                new_generation: Mutex::new(None),
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl EnvironmentOps for FakeEnvironmentOps {
        fn run(
            &self,
            program: &str,
            args: &[String],
            _timeout: Duration,
        ) -> Result<CommandResult, EssentiaSetupError> {
            self.calls.lock().unwrap().push(RecordedCall {
                program: program.to_string(),
                args: args.to_vec(),
            });
            let success = if args.get(1).is_some_and(|arg| arg == "venv") {
                if self.config.venv {
                    let generation = PathBuf::from(args.last().unwrap());
                    fs::create_dir_all(generation.join("bin")).unwrap();
                    fs::write(generation.join("bin/python"), b"fake python").unwrap();
                    *self.new_generation.lock().unwrap() = Some(generation);
                }
                self.config.venv
            } else if args.get(1).is_some_and(|arg| arg == "pip") {
                self.config.pip
            } else if program == "python3.14" {
                self.config.python314
            } else if program == "python3" {
                self.config.python3
            } else {
                false
            };
            Ok(CommandResult {
                success,
                stdout: Vec::new(),
                stderr: if success {
                    Vec::new()
                } else if args.get(1).is_some_and(|arg| arg == "pip")
                    && self.config.pip_wheel_unavailable
                {
                    b"ERROR: No matching distribution found for essentia==2.1b6.dev1438".to_vec()
                } else {
                    b"scripted failure".to_vec()
                },
            })
        }

        fn inspect_runtime(
            &self,
            python_path: &Path,
            _timeout: Duration,
        ) -> Result<EssentiaRuntime, EssentiaSetupError> {
            let generation = self.new_generation.lock().unwrap().clone().ok_or_else(|| {
                EssentiaSetupError::new(
                    EssentiaSetupErrorKind::ImportFailure,
                    "no scripted generation is available",
                )
            })?;
            let is_direct = python_path == generation.join("bin/python");
            let is_stable = python_path == self.paths.stable.join("bin/python")
                && fs::read_link(&self.paths.stable)
                    .ok()
                    .map(|target| {
                        if target.is_absolute() {
                            target
                        } else {
                            self.paths.stable.parent().unwrap().join(target)
                        }
                    })
                    .is_some_and(|target| target == generation);
            if (is_direct && !self.config.direct_probe)
                || (is_stable && !self.config.stable_probe)
                || (!is_direct && !is_stable)
            {
                return Err(EssentiaSetupError::new(
                    EssentiaSetupErrorKind::ManifestMismatch,
                    "scripted runtime probe mismatch",
                ));
            }
            Ok(exact_runtime(python_path))
        }
    }

    fn exact_runtime(python_path: &Path) -> EssentiaRuntime {
        EssentiaRuntime {
            python_path: python_path.to_string_lossy().into_owned(),
            python_version: "3.14.6".into(),
            essentia_version: "2.1b6.dev1438".into(),
            essentia_module_version: "2.1-beta6-dev".into(),
            numpy_version: "2.5.1".into(),
            pyyaml_version: "6.0.3".into(),
            six_version: "1.17.0".into(),
            analyzer_contract: ESSENTIA_CONTRACT_ID.into(),
        }
    }

    fn test_paths(root: &Path) -> ManagedEnvironmentPaths {
        ManagedEnvironmentPaths::from_stable(root.join("essentia-venv")).unwrap()
    }

    fn test_lock_config() -> LockConfig {
        LockConfig {
            timeout: Duration::from_millis(50),
            poll: Duration::from_millis(1),
        }
    }

    #[cfg(unix)]
    fn make_previous_symlink(paths: &ManagedEnvironmentPaths) -> PathBuf {
        let previous = paths.generations.join("runtime-previous");
        fs::create_dir_all(previous.join("bin")).unwrap();
        fs::write(previous.join("bin/python"), b"previous python").unwrap();
        fs::create_dir_all(paths.stable.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(
            previous
                .strip_prefix(paths.stable.parent().unwrap())
                .unwrap(),
            &paths.stable,
        )
        .unwrap();
        previous
    }

    fn assert_no_switch_artifacts(parent: &Path) {
        let names: Vec<String> = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names.iter().all(|name| {
                !name.starts_with(".essentia-venv-switch-")
                    && !name.starts_with(".essentia-venv-failed-")
                    && !name.starts_with(".essentia-venv-restore-")
            }),
            "unexpected switch artifacts: {names:?}"
        );
    }

    #[cfg(unix)]
    fn fake_python(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("python");
        fs::write(&path, format!("#!/bin/sh\necho '{}'\n", body)).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    #[cfg(unix)]
    fn fake_python_script(dir: &Path, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("python");
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }
    #[test]
    #[cfg(unix)]
    fn essentia_environment_accepts_exact_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python(
            dir.path(),
            r#"{"python":"3.14.6","implementation":"cpython","essentia":"2.1b6.dev1438","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#,
        );
        let output = Command::new(&path)
            .args(["-c", "ignored"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            r#"{"python":"3.14.6","implementation":"cpython","essentia":"2.1b6.dev1438","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#
        );
        let runtime =
            inspect_essentia_python_with_timeout(&path.to_string_lossy(), Duration::from_secs(1))
                .unwrap();
        assert_eq!(runtime.analyzer_contract, ESSENTIA_CONTRACT_ID);
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_probe_rejects_invalid_utf8_before_manifest_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python_script(
            dir.path(),
            "printf '{\"python\":\"\\377\",\"implementation\":\"cpython\",\"essentia\":\"2.1b6.dev1438\",\"essentia_module\":\"2.1-beta6-dev\",\"numpy\":\"2.5.1\",\"pyyaml\":\"6.0.3\",\"six\":\"1.17.0\"}'",
        );

        let error = inspect_essentia_python_with_timeout_result(
            &path.to_string_lossy(),
            Duration::from_secs(5),
        )
        .unwrap_err();

        assert_eq!(error.kind, EssentiaSetupErrorKind::ImportFailure);
        assert!(error.message.starts_with(&format!(
            "runtime probe for {} returned invalid JSON:",
            path.to_string_lossy()
        )));
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_probe_errors_are_typed() {
        let dir = tempfile::tempdir().unwrap();
        let mismatch = fake_python(
            dir.path(),
            r#"{"python":"3.14.6","implementation":"pypy","essentia":"2.1b6.dev1438","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#,
        );
        let mismatch = inspect_essentia_python_with_timeout_result(
            &mismatch.to_string_lossy(),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(mismatch.kind, EssentiaSetupErrorKind::ManifestMismatch);

        let failed_dir = dir.path().join("failed-import");
        fs::create_dir(&failed_dir).unwrap();
        let failed = fake_python_script(&failed_dir, "echo 'missing yaml' >&2; exit 1");
        let failed = inspect_essentia_python_with_timeout_result(
            &failed.to_string_lossy(),
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(failed.kind, EssentiaSetupErrorKind::ImportFailure);
        assert!(failed.message.contains("missing yaml"));

        let timeout_dir = dir.path().join("probe-timeout");
        fs::create_dir(&timeout_dir).unwrap();
        let timed_out = fake_python_script(&timeout_dir, "sleep 2");
        let timed_out = inspect_essentia_python_with_timeout_result(
            &timed_out.to_string_lossy(),
            Duration::from_millis(20),
        )
        .unwrap_err();
        assert_eq!(timed_out.kind, EssentiaSetupErrorKind::ProbeTimeout);
    }
    #[test]
    #[cfg(unix)]
    fn essentia_environment_rejects_wrong_package_and_prefers_valid_default() {
        let dir = tempfile::tempdir().unwrap();
        let bad = fake_python(
            dir.path(),
            r#"{"python":"3.14.6","implementation":"cpython","essentia":"2.1b6.dev1389","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#,
        );
        let good = dir.path().join("good");
        fs::create_dir(&good).unwrap();
        let good = fake_python(
            &good,
            r#"{"python":"3.14.6","implementation":"cpython","essentia":"2.1b6.dev1438","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#,
        );
        assert_eq!(
            probe_essentia_runtime_from_sources(Some(&bad.to_string_lossy()), Some(good.clone()))
                .unwrap()
                .python_path,
            good.to_string_lossy()
        );
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_probe_fails_closed_and_keeps_valid_override_priority() {
        let dir = tempfile::tempdir().unwrap();
        let valid_dir = dir.path().join("valid");
        fs::create_dir(&valid_dir).unwrap();
        let valid = fake_python(
            &valid_dir,
            r#"{"python":"3.14.6","implementation":"cpython","essentia":"2.1b6.dev1438","essentia_module":"2.1-beta6-dev","numpy":"2.5.1","pyyaml":"6.0.3","six":"1.17.0"}"#,
        );
        assert_eq!(
            probe_essentia_runtime_from_sources(Some(&valid.to_string_lossy()), None)
                .unwrap()
                .python_path,
            valid.to_string_lossy()
        );
        assert!(
            inspect_essentia_python_with_timeout(
                &dir.path().join("missing").to_string_lossy(),
                Duration::from_millis(20)
            )
            .is_none()
        );

        for (name, script) in [
            ("non-numeric", "echo unsupported"),
            (
                "non-zero",
                "echo '{\"python\":\"3.14.6\",\"implementation\":\"cpython\",\"essentia\":\"2.1b6.dev1438\",\"essentia_module\":\"2.1-beta6-dev\",\"numpy\":\"2.5.1\",\"pyyaml\":\"6.0.3\",\"six\":\"1.17.0\"}'; exit 7",
            ),
        ] {
            let script_dir = dir.path().join(name);
            fs::create_dir(&script_dir).unwrap();
            let path = fake_python_script(&script_dir, script);
            assert!(
                inspect_essentia_python_with_timeout(
                    &path.to_string_lossy(),
                    Duration::from_millis(100)
                )
                .is_none(),
                "{name} probe must fail closed"
            );
        }

        let timeout_dir = dir.path().join("timeout");
        fs::create_dir(&timeout_dir).unwrap();
        let timeout_path = fake_python_script(&timeout_dir, "sleep 2");
        let started = Instant::now();
        assert!(
            inspect_essentia_python_with_timeout(
                &timeout_path.to_string_lossy(),
                Duration::from_millis(20)
            )
            .is_none()
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_command_runner_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python_script(dir.path(), "sleep 2");
        let started = Instant::now();
        let error = SystemEnvironmentOps
            .run(&path.to_string_lossy(), &[], Duration::from_millis(20))
            .unwrap_err();
        assert_eq!(error.kind, EssentiaSetupErrorKind::ProbeTimeout);
        assert_eq!(
            error.message,
            format!("{} timed out after 0.02 seconds", path.to_string_lossy())
        );
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_process_success_preserves_both_streams() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python_script(
            dir.path(),
            "printf 'stdout-value'; printf 'stderr-value' >&2",
        );

        let result = SystemEnvironmentOps
            .run(&path.to_string_lossy(), &[], Duration::from_secs(5))
            .unwrap();

        assert!(result.success);
        assert_eq!(result.stdout, b"stdout-value");
        assert_eq!(result.stderr, b"stderr-value");
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_process_nonzero_preserves_diagnostic_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python_script(
            dir.path(),
            "printf 'stdout-value'; printf 'stderr-value' >&2; exit 7",
        );

        let result = SystemEnvironmentOps
            .run(&path.to_string_lossy(), &[], Duration::from_secs(5))
            .unwrap();

        assert!(!result.success);
        assert_eq!(result.stdout, b"stdout-value");
        assert_eq!(result.stderr, b"stderr-value");
        assert_eq!(
            format_diagnostic_output(&result),
            ": stderr-value\nstdout-value"
        );
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_probe_nonzero_reports_stderr_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = fake_python_script(
            dir.path(),
            "printf 'stdout-must-not-surface'; printf 'stderr-diagnostic' >&2; exit 7",
        );

        let error = inspect_essentia_python_with_timeout_result(
            &path.to_string_lossy(),
            Duration::from_secs(5),
        )
        .unwrap_err();

        assert_eq!(error.kind, EssentiaSetupErrorKind::ImportFailure);
        assert_eq!(
            error.message,
            format!(
                "runtime probe imports failed for {}: stderr-diagnostic",
                path.to_string_lossy()
            )
        );
    }

    #[test]
    fn essentia_environment_error_kinds_and_display_are_stable() {
        let cases = [
            (EssentiaSetupErrorKind::LockTimeout, "lock_timeout"),
            (
                EssentiaSetupErrorKind::CandidateNotFound,
                "candidate_not_found",
            ),
            (EssentiaSetupErrorKind::VenvCreation, "venv_creation"),
            (
                EssentiaSetupErrorKind::WheelUnavailable,
                "wheel_unavailable",
            ),
            (EssentiaSetupErrorKind::PipFailure, "pip_failure"),
            (EssentiaSetupErrorKind::ImportFailure, "import_failure"),
            (
                EssentiaSetupErrorKind::ManifestMismatch,
                "manifest_mismatch",
            ),
            (EssentiaSetupErrorKind::ProbeTimeout, "probe_timeout"),
            (EssentiaSetupErrorKind::Filesystem, "filesystem"),
            (EssentiaSetupErrorKind::Activation, "activation"),
        ];

        for (kind, serialized) in cases {
            assert_eq!(
                serde_json::to_string(&kind).unwrap(),
                format!("\"{serialized}\"")
            );
            assert_eq!(
                EssentiaSetupError::new(kind, "stable message").to_string(),
                format!("{kind:?}: stable message")
            );
        }
    }

    #[test]
    fn essentia_environment_process_error_rendering_is_consumer_specific() {
        use super::process::{OutputStream, ProcessError, ProcessErrorKind};

        let probe_path = "/managed/python";
        let timeout = Duration::from_millis(20);
        let probe_cases = [
            (
                ProcessError::new(ProcessErrorKind::Start(std::io::Error::other("sentinel"))),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to start runtime probe for /managed/python: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::MissingCapture(OutputStream::Stdout)),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to capture runtime probe stdout for /managed/python",
            ),
            (
                ProcessError::new(ProcessErrorKind::ReaderStart {
                    stream: OutputStream::Stderr,
                    source: std::io::Error::other("sentinel"),
                }),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to start runtime probe stderr reader for /managed/python: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::Wait(std::io::Error::other("sentinel"))),
                EssentiaSetupErrorKind::ImportFailure,
                "failed while waiting for runtime probe /managed/python: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::ReaderPanicked(OutputStream::Stderr)),
                EssentiaSetupErrorKind::ImportFailure,
                "runtime probe stderr reader panicked for /managed/python",
            ),
            (
                ProcessError::new(ProcessErrorKind::Timeout),
                EssentiaSetupErrorKind::ProbeTimeout,
                "runtime probe for /managed/python timed out after 0.02 seconds",
            ),
        ];
        for (source, kind, message) in probe_cases {
            assert_eq!(
                probe_process_error(probe_path, timeout, source),
                EssentiaSetupError::new(kind, message)
            );
        }

        let generic_cases = [
            (
                ProcessError::new(ProcessErrorKind::Start(std::io::Error::other("sentinel"))),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to start python3.14: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::MissingCapture(OutputStream::Stderr)),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to capture stderr for python3.14",
            ),
            (
                ProcessError::new(ProcessErrorKind::ReaderStart {
                    stream: OutputStream::Stdout,
                    source: std::io::Error::other("sentinel"),
                }),
                EssentiaSetupErrorKind::ImportFailure,
                "failed to start stdout reader for python3.14: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::Wait(std::io::Error::other("sentinel"))),
                EssentiaSetupErrorKind::ImportFailure,
                "failed while waiting for python3.14: sentinel",
            ),
            (
                ProcessError::new(ProcessErrorKind::ReaderPanicked(OutputStream::Stdout)),
                EssentiaSetupErrorKind::ImportFailure,
                "stdout reader panicked for python3.14",
            ),
            (
                ProcessError::new(ProcessErrorKind::Timeout),
                EssentiaSetupErrorKind::ProbeTimeout,
                "python3.14 timed out after 0.02 seconds",
            ),
        ];
        for (source, kind, message) in generic_cases {
            assert_eq!(
                generic_process_error("python3.14", timeout, source),
                EssentiaSetupError::new(kind, message)
            );
        }
    }

    #[test]
    fn essentia_environment_managed_path_is_never_repository_local() {
        assert_eq!(
            ESSENTIA_VENV_RELPATH,
            ".local/share/reklawdbox/essentia-venv"
        );
        assert!(ESSENTIA_VENV_RELPATH.starts_with(".local/"));
        assert!(!ESSENTIA_VENV_RELPATH.contains(".venvs/"));
    }

    #[test]
    fn essentia_environment_lock_times_out_then_reacquires_after_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup.lock");
        let first =
            AdvisoryLock::acquire(&path, Duration::from_millis(20), Duration::from_millis(1))
                .unwrap();
        let error =
            AdvisoryLock::acquire(&path, Duration::from_millis(20), Duration::from_millis(1))
                .unwrap_err();
        assert_eq!(error.kind, EssentiaSetupErrorKind::LockTimeout);
        drop(first);
        AdvisoryLock::acquire(&path, Duration::from_millis(20), Duration::from_millis(1)).unwrap();
    }

    #[test]
    fn essentia_environment_uses_exact_candidate_order_and_wheel_only_manifest() {
        assert_eq!(ESSENTIA_PROBE_TIMEOUT_SECS, 30);
        assert_eq!(PYTHON_CANDIDATES, &["python3.14", "python3"]);
        assert_eq!(
            pip_install_args(),
            vec![
                "-m",
                "pip",
                "install",
                "--only-binary=:all:",
                "--no-deps",
                "essentia==2.1b6.dev1438",
                "numpy==2.5.1",
                "PyYAML==6.0.3",
                "six==1.17.0",
            ]
        );
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_installs_transactionally_and_prunes_previous_generation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let ops = FakeEnvironmentOps::new(
            paths.clone(),
            FakeConfig {
                python314: false,
                ..FakeConfig::default()
            },
        );

        let installed = install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap();

        assert_eq!(
            installed.runtime.python_path,
            paths.stable.join("bin/python").to_string_lossy()
        );
        assert_eq!(installed.python_bin_used.as_deref(), Some("python3"));
        assert!(paths.stable.is_symlink());
        let target = fs::read_link(&paths.stable).unwrap();
        assert!(
            target.is_relative(),
            "stable target must be relative: {target:?}"
        );
        assert!(target.starts_with(ESSENTIA_VENV_GENERATIONS));
        assert!(!previous.exists(), "previous generation should be pruned");

        let calls = ops.calls();
        assert_eq!(calls[0].program, "python3.14");
        assert_eq!(calls[1].program, "python3");
        let venv = &calls[2];
        assert_eq!(venv.program, "python3");
        assert_eq!(&venv.args[..3], ["-m", "venv", "--copies"]);
        let pip = &calls[3];
        assert!(pip.program.ends_with("/bin/python"));
        assert_eq!(pip.args, pip_install_args());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_never_prunes_external_previous_target() {
        let dir = tempfile::tempdir().unwrap();
        let managed_root = dir.path().join("managed");
        let paths = test_paths(&managed_root);
        fs::create_dir_all(paths.stable.parent().unwrap()).unwrap();
        let external = dir.path().join("runtime-external");
        fs::create_dir_all(external.join("bin")).unwrap();
        fs::write(external.join("bin/python"), b"external python").unwrap();
        std::os::unix::fs::symlink(&external, &paths.stable).unwrap();
        let ops = FakeEnvironmentOps::new(paths.clone(), FakeConfig::default());

        install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap();

        assert!(external.exists());
        assert_eq!(
            fs::read(external.join("bin/python")).unwrap(),
            b"external python"
        );
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_rejects_symlinked_generation_root() {
        let dir = tempfile::tempdir().unwrap();
        let managed_root = dir.path().join("managed");
        let paths = test_paths(&managed_root);
        fs::create_dir_all(paths.stable.parent().unwrap()).unwrap();
        let external = dir.path().join("external-generations");
        fs::create_dir(&external).unwrap();
        std::os::unix::fs::symlink(&external, &paths.generations).unwrap();
        let ops = FakeEnvironmentOps::new(paths.clone(), FakeConfig::default());

        let error = install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap_err();

        assert_eq!(error.kind, EssentiaSetupErrorKind::Filesystem);
        assert!(error.message.contains("must not be a symlink"));
        assert_eq!(fs::read_dir(external).unwrap().count(), 0);
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_install_failures_preserve_previous_runtime_and_clean_generation() {
        let failure_configs = [
            (
                FakeConfig {
                    venv: false,
                    ..FakeConfig::default()
                },
                EssentiaSetupErrorKind::VenvCreation,
            ),
            (
                FakeConfig {
                    pip: false,
                    ..FakeConfig::default()
                },
                EssentiaSetupErrorKind::PipFailure,
            ),
            (
                FakeConfig {
                    pip: false,
                    pip_wheel_unavailable: true,
                    ..FakeConfig::default()
                },
                EssentiaSetupErrorKind::WheelUnavailable,
            ),
            (
                FakeConfig {
                    direct_probe: false,
                    ..FakeConfig::default()
                },
                EssentiaSetupErrorKind::ManifestMismatch,
            ),
        ];

        for (config, expected_kind) in failure_configs {
            let dir = tempfile::tempdir().unwrap();
            let paths = test_paths(dir.path());
            let previous = make_previous_symlink(&paths);
            let previous_target = fs::read_link(&paths.stable).unwrap();
            let ops = FakeEnvironmentOps::new(paths.clone(), config);

            let error = install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap_err();
            assert_eq!(error.kind, expected_kind);
            assert_eq!(fs::read_link(&paths.stable).unwrap(), previous_target);
            assert!(previous.exists());
            let generations: Vec<_> = fs::read_dir(&paths.generations)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect();
            assert_eq!(generations, vec![previous]);
            assert_no_switch_artifacts(dir.path());
        }
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_candidate_not_found_is_typed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let ops = FakeEnvironmentOps::new(
            paths.clone(),
            FakeConfig {
                python314: false,
                python3: false,
                ..FakeConfig::default()
            },
        );

        let error = install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap_err();

        assert_eq!(error.kind, EssentiaSetupErrorKind::CandidateNotFound);
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_stable_probe_failure_restores_previous_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let previous_target = fs::read_link(&paths.stable).unwrap();
        let ops = FakeEnvironmentOps::new(
            paths.clone(),
            FakeConfig {
                stable_probe: false,
                ..FakeConfig::default()
            },
        );

        assert!(install_managed_essentia_at(&paths, &ops, test_lock_config()).is_err());
        assert_eq!(fs::read_link(&paths.stable).unwrap(), previous_target);
        assert!(previous.exists());
        let generations: Vec<_> = fs::read_dir(&paths.generations)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(generations, vec![previous]);
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_stable_probe_failure_restores_legacy_directory() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        fs::create_dir_all(paths.stable.join("bin")).unwrap();
        fs::write(paths.stable.join("bin/python"), b"legacy python").unwrap();
        let ops = FakeEnvironmentOps::new(
            paths.clone(),
            FakeConfig {
                stable_probe: false,
                ..FakeConfig::default()
            },
        );

        assert!(install_managed_essentia_at(&paths, &ops, test_lock_config()).is_err());
        assert!(paths.stable.is_dir());
        assert_eq!(
            fs::read(paths.stable.join("bin/python")).unwrap(),
            b"legacy python"
        );
        let generations: Vec<_> = fs::read_dir(&paths.generations)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert!(generations.is_empty());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn essentia_environment_successfully_migrates_and_prunes_legacy_directory() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        fs::create_dir_all(paths.stable.join("bin")).unwrap();
        fs::write(paths.stable.join("bin/python"), b"legacy python").unwrap();
        let ops = FakeEnvironmentOps::new(paths.clone(), FakeConfig::default());

        install_managed_essentia_at(&paths, &ops, test_lock_config()).unwrap();

        assert!(paths.stable.is_symlink());
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            names
                .iter()
                .all(|name| !name.starts_with("essentia-venv.legacy-"))
        );
        assert_no_switch_artifacts(dir.path());
    }
}
