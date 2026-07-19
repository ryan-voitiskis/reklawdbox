//! Managed Essentia generation installation orchestration.

use std::time::Duration;

use super::activation::{
    ActivationTransaction, AdvisoryLock, IncompleteGeneration, LockConfig, ManagedEnvironmentPaths,
    setup_activation_error,
};
use super::contract::{
    ESSENTIA_PROBE_TIMEOUT_SECS, EssentiaRuntime, EssentiaSetupError, EssentiaSetupErrorKind,
    diagnostic_text, essentia_venv_dir, generic_process_error, inspect_essentia_python_with_runner,
};
use super::process::{CommandRequest, CommandResult, CommandRunner, SystemCommandRunner};

const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const LOCK_POLL: Duration = Duration::from_millis(100);
const PYTHON_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const VENV_CREATE_TIMEOUT: Duration = Duration::from_secs(120);
const PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);
pub(super) const PYTHON_CANDIDATES: &[&str] = &["python3.14", "python3"];
const PACKAGE_SPECS: &[&str] = &[
    "essentia==2.1b6.dev1438",
    "numpy==2.5.1",
    "PyYAML==6.0.3",
    "six==1.17.0",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManagedEssentiaInstall {
    pub runtime: EssentiaRuntime,
    pub python_bin_used: Option<String>,
}

pub(crate) fn install_managed_essentia() -> Result<ManagedEssentiaInstall, EssentiaSetupError> {
    let stable = essentia_venv_dir()
        .ok_or("Cannot determine home directory for managed Essentia location")?;
    let runner = SystemCommandRunner::default();
    install_managed_essentia_at(
        &ManagedEnvironmentPaths::from_stable(stable)?,
        &runner,
        LockConfig {
            timeout: LOCK_TIMEOUT,
            poll: LOCK_POLL,
        },
    )
}

pub(super) fn install_managed_essentia_at(
    paths: &ManagedEnvironmentPaths,
    runner: &dyn CommandRunner,
    lock_config: LockConfig,
) -> Result<ManagedEssentiaInstall, EssentiaSetupError> {
    let stable_python = paths.stable.join("bin/python");
    let probe_timeout = Duration::from_secs(ESSENTIA_PROBE_TIMEOUT_SECS);
    if let Some(runtime) = probe_stable_runtime(runner, &stable_python, probe_timeout) {
        return Ok(ManagedEssentiaInstall {
            runtime,
            python_bin_used: None,
        });
    }

    paths.prepare_parent()?;
    let _lock = AdvisoryLock::acquire(&paths.lock, lock_config.timeout, lock_config.poll)?;

    if let Some(runtime) = probe_stable_runtime(runner, &stable_python, probe_timeout) {
        return Ok(ManagedEssentiaInstall {
            runtime,
            python_bin_used: None,
        });
    }

    let python = find_python_314(runner)?;
    let generation = build_and_validate_generation(paths, runner, &python, probe_timeout)?;
    let runtime =
        activate_and_validate_stable(paths, runner, &stable_python, probe_timeout, generation)?;
    Ok(ManagedEssentiaInstall {
        runtime,
        python_bin_used: Some(python),
    })
}

fn probe_stable_runtime(
    runner: &dyn CommandRunner,
    stable_python: &std::path::Path,
    probe_timeout: Duration,
) -> Option<EssentiaRuntime> {
    let mut runtime = inspect_essentia_python_with_runner(
        runner,
        &stable_python.to_string_lossy(),
        probe_timeout,
    )
    .ok()?;
    runtime.python_path = stable_python.to_string_lossy().into_owned();
    Some(runtime)
}

fn build_and_validate_generation(
    paths: &ManagedEnvironmentPaths,
    runner: &dyn CommandRunner,
    python: &str,
    probe_timeout: Duration,
) -> Result<IncompleteGeneration, EssentiaSetupError> {
    let generation_guard = paths.prepare_generation()?;
    let generation = generation_guard.path().to_path_buf();
    let venv_args = vec![
        "-m".to_string(),
        "venv".to_string(),
        "--copies".to_string(),
        generation.to_string_lossy().into_owned(),
    ];
    run_checked(
        runner,
        python,
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
        runner,
        &candidate.to_string_lossy(),
        &args,
        PIP_INSTALL_TIMEOUT,
        "wheel-only Essentia installation",
        InstallCommand::Pip,
    )?;
    inspect_essentia_python_with_runner(runner, &candidate.to_string_lossy(), probe_timeout)?;
    Ok(generation_guard)
}

fn activate_and_validate_stable(
    paths: &ManagedEnvironmentPaths,
    runner: &dyn CommandRunner,
    stable_python: &std::path::Path,
    probe_timeout: Duration,
    generation: IncompleteGeneration,
) -> Result<EssentiaRuntime, EssentiaSetupError> {
    let mut activation = ActivationTransaction::prepare(paths, generation);
    activation.switch().map_err(setup_activation_error)?;
    let stable_runtime = inspect_essentia_python_with_runner(
        runner,
        &stable_python.to_string_lossy(),
        probe_timeout,
    );
    let mut stable_runtime = match stable_runtime {
        Ok(runtime) => runtime,
        Err(stable_error) => {
            if let Err(restore_error) = activation.rollback() {
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
    activation
        .stable_validated()
        .map_err(setup_activation_error)?;
    activation.commit().map_err(setup_activation_error)?;
    Ok(stable_runtime)
}

fn find_python_314(runner: &dyn CommandRunner) -> Result<String, EssentiaSetupError> {
    let mut diagnostics = Vec::new();
    for candidate in PYTHON_CANDIDATES {
        let args = vec![
            "-c".to_string(),
            "import platform, sys; raise SystemExit(0 if platform.python_implementation() == 'CPython' and sys.version_info[:2] == (3, 14) else 1)".to_string(),
        ];
        match runner.run(CommandRequest {
            program: candidate,
            args: &args,
            timeout: PYTHON_CHECK_TIMEOUT,
        }) {
            Ok(result) if result.success => return Ok(candidate.to_string()),
            Ok(result) => diagnostics.push(format!(
                "{candidate}: not CPython 3.14{}",
                format_diagnostic_output(&result)
            )),
            Err(error) => diagnostics.push(format!(
                "{candidate}: {}",
                generic_process_error(candidate, PYTHON_CHECK_TIMEOUT, error)
            )),
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

pub(super) fn pip_install_args() -> Vec<String> {
    let mut args: Vec<String> = ["-m", "pip", "install", "--only-binary=:all:", "--no-deps"]
        .into_iter()
        .map(str::to_string)
        .collect();
    args.extend(PACKAGE_SPECS.iter().map(|spec| (*spec).to_string()));
    args
}

fn run_checked(
    runner: &dyn CommandRunner,
    program: &str,
    args: &[String],
    timeout: Duration,
    context: &str,
    stage: InstallCommand,
) -> Result<(), EssentiaSetupError> {
    let output = runner
        .run(CommandRequest {
            program,
            args,
            timeout,
        })
        .map_err(|error| {
            EssentiaSetupError::new(
                stage.default_error_kind(),
                format!(
                    "Failed to run {context}: {}",
                    generic_process_error(program, timeout, error)
                ),
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

pub(super) fn format_diagnostic_output(output: &CommandResult) -> String {
    diagnostic_text(
        &String::from_utf8_lossy(&output.stderr),
        &String::from_utf8_lossy(&output.stdout),
    )
}
