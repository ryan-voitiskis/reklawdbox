//! Managed, reproducible Essentia runtime discovery and installation.
//!
//! The stable `essentia-venv` entry point is a symlink selected only after a
//! complete immutable generation has passed the exact runtime probe.

#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::{Duration, Instant};

mod activation;
mod contract;
mod install;
mod platform;
mod process;

#[allow(unused_imports)]
pub(crate) use self::activation::ESSENTIA_VENV_GENERATIONS;
#[cfg(test)]
use self::activation::{ActivationPhase, ActivationTransaction, IncompleteGeneration};
#[cfg(test)]
use self::activation::{AdvisoryLock, LockConfig, ManagedEnvironmentPaths};
#[cfg(test)]
use self::contract::generic_process_error;
#[allow(unused_imports)]
pub(crate) use self::contract::{
    ESSENTIA_CONTRACT_ID, ESSENTIA_PROBE_TIMEOUT_SECS, ESSENTIA_PYTHON_ENV_VAR, EssentiaRuntime,
    EssentiaSetupError, EssentiaSetupErrorKind, SUPPORTED_ESSENTIA_MODULE_VERSION,
    SUPPORTED_ESSENTIA_VERSION, SUPPORTED_NUMPY_VERSION, SUPPORTED_PYTHON_PREFIX,
    SUPPORTED_PYYAML_VERSION, SUPPORTED_SIX_VERSION, essentia_venv_dir, inspect_essentia_python,
    probe_essentia_python_path, probe_essentia_runtime_path,
};
#[cfg(test)]
use self::contract::{
    ESSENTIA_IMPORT_CHECK_SCRIPT, inspect_essentia_python_with_timeout,
    inspect_essentia_python_with_timeout_result, probe_process_error,
};
#[allow(unused_imports)]
pub(crate) use self::contract::{ESSENTIA_VENV_RELPATH, probe_essentia_runtime_from_sources};
#[cfg(test)]
pub(crate) use self::contract::{
    probe_essentia_python_from_sources, validate_essentia_python_with_timeout,
};
#[allow(unused_imports)]
pub(crate) use self::install::{ManagedEssentiaInstall, install_managed_essentia};
#[cfg(test)]
use self::install::{
    PYTHON_CANDIDATES, format_diagnostic_output, install_managed_essentia_at, pip_install_args,
};
#[cfg(test)]
use self::process::ProcessError;
#[cfg(test)]
use self::process::{CommandRequest, CommandResult, CommandRunner, SystemCommandRunner};

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedCall {
        program: String,
        args: Vec<String>,
        timeout: Duration,
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
        sabotage_stable_before_failure: bool,
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
                sabotage_stable_before_failure: false,
            }
        }
    }

    struct FakeCommandRunner {
        paths: ManagedEnvironmentPaths,
        config: FakeConfig,
        calls: Mutex<Vec<RecordedCall>>,
        new_generation: Mutex<Option<PathBuf>>,
    }

    impl FakeCommandRunner {
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

    impl CommandRunner for FakeCommandRunner {
        fn run(&self, request: CommandRequest<'_>) -> Result<CommandResult, ProcessError> {
            let program = request.program;
            let args = request.args;
            self.calls.lock().unwrap().push(RecordedCall {
                program: program.to_string(),
                args: args.to_vec(),
                timeout: request.timeout,
            });
            if args == ["-c", ESSENTIA_IMPORT_CHECK_SCRIPT] {
                let generation = self.new_generation.lock().unwrap().clone();
                let is_direct = generation
                    .as_ref()
                    .is_some_and(|generation| Path::new(program) == generation.join("bin/python"));
                let is_stable = generation.as_ref().is_some_and(|generation| {
                    Path::new(program) == self.paths.stable.join("bin/python")
                        && fs::read_link(&self.paths.stable)
                            .ok()
                            .map(|target| {
                                if target.is_absolute() {
                                    target
                                } else {
                                    self.paths.stable.parent().unwrap().join(target)
                                }
                            })
                            .is_some_and(|target| target == generation.as_path())
                });
                if !is_direct && !is_stable {
                    return Ok(CommandResult {
                        success: false,
                        stdout: Vec::new(),
                        stderr: b"scripted runtime unavailable".to_vec(),
                    });
                }
                let manifest_matches = (is_direct && self.config.direct_probe)
                    || (is_stable && self.config.stable_probe);
                if is_stable
                    && !self.config.stable_probe
                    && self.config.sabotage_stable_before_failure
                {
                    fs::remove_file(&self.paths.stable).unwrap();
                    fs::create_dir(&self.paths.stable).unwrap();
                }
                return Ok(CommandResult {
                    success: true,
                    stdout: probe_json(manifest_matches),
                    stderr: Vec::new(),
                });
            }
            let success = if args.get(1).is_some_and(|arg| arg == "venv") {
                let generation = PathBuf::from(args.last().unwrap());
                fs::create_dir_all(generation.join("bin")).unwrap();
                fs::write(generation.join("partial-build"), b"incomplete").unwrap();
                if self.config.venv {
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
    }

    fn probe_json(manifest_matches: bool) -> Vec<u8> {
        let numpy = if manifest_matches { "2.5.1" } else { "0.0.0" };
        serde_json::to_vec(&serde_json::json!({
            "python": "3.14.6",
            "implementation": "cpython",
            "essentia": "2.1b6.dev1438",
            "essentia_module": "2.1-beta6-dev",
            "numpy": numpy,
            "pyyaml": "6.0.3",
            "six": "1.17.0"
        }))
        .unwrap()
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

    fn run_system_command(
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
        let error = run_system_command(&path.to_string_lossy(), &[], Duration::from_millis(20))
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

        let result =
            run_system_command(&path.to_string_lossy(), &[], Duration::from_secs(5)).unwrap();

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

        let result =
            run_system_command(&path.to_string_lossy(), &[], Duration::from_secs(5)).unwrap();

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
    fn essentia_environment_incomplete_generation_drop_removes_uncommitted_path() {
        let dir = tempfile::tempdir().unwrap();
        let generation = dir.path().join("runtime-incomplete");
        fs::create_dir(&generation).unwrap();
        fs::write(generation.join("partial-build"), b"incomplete").unwrap();

        {
            let _guard = IncompleteGeneration::new(generation.clone());
        }

        assert!(!generation.exists());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_activation_drop_emergency_restores_and_cleans() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let previous_target = fs::read_link(&paths.stable).unwrap();
        let generation = paths.generations.join("runtime-emergency");
        fs::create_dir_all(generation.join("bin")).unwrap();
        fs::write(generation.join("bin/python"), b"validated python").unwrap();
        let guard = IncompleteGeneration::new(generation.clone());

        let mut activation = ActivationTransaction::prepare(&paths, guard);
        assert_eq!(activation.phase(), ActivationPhase::Prepared);
        activation.switch().unwrap();
        assert_eq!(activation.phase(), ActivationPhase::Switched);
        assert_ne!(fs::read_link(&paths.stable).unwrap(), previous_target);
        drop(activation);

        assert_eq!(fs::read_link(&paths.stable).unwrap(), previous_target);
        assert!(previous.exists());
        assert!(!generation.exists());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_activation_drop_preserves_generation_when_emergency_restore_fails() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let generation = paths.generations.join("runtime-emergency-failure");
        fs::create_dir_all(generation.join("bin")).unwrap();
        fs::write(generation.join("bin/python"), b"validated python").unwrap();
        let guard = IncompleteGeneration::new(generation.clone());
        let mut activation = ActivationTransaction::prepare(&paths, guard);
        activation.switch().unwrap();
        fs::remove_file(&paths.stable).unwrap();
        fs::create_dir(&paths.stable).unwrap();

        drop(activation);

        assert!(paths.stable.is_dir());
        assert!(previous.exists());
        assert!(generation.join("bin/python").is_file());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    fn essentia_environment_activation_rejects_invalid_phase_without_advancing() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let generation = dir.path().join("runtime-invalid-phase");
        fs::create_dir(&generation).unwrap();
        let guard = IncompleteGeneration::new(generation.clone());
        let mut activation = ActivationTransaction::prepare(&paths, guard);

        let error = activation.commit().unwrap_err();

        assert_eq!(
            error.to_string(),
            "Cannot commit managed runtime while activation is Prepared; expected StableValidated"
        );
        assert_eq!(activation.phase(), ActivationPhase::Prepared);
        drop(activation);
        assert!(!generation.exists());
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
    fn essentia_environment_installs_with_no_prior_stable_path_in_phase_order() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let runner = FakeCommandRunner::new(paths.clone(), FakeConfig::default());

        let installed = install_managed_essentia_at(&paths, &runner, test_lock_config()).unwrap();

        assert_eq!(installed.python_bin_used.as_deref(), Some("python3.14"));
        assert!(paths.stable.is_symlink());
        let generations = fs::read_dir(&paths.generations)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(generations.len(), 1);
        let calls = runner.calls();
        assert_eq!(calls.len(), 7);
        let stable_python = paths
            .stable
            .join("bin/python")
            .to_string_lossy()
            .into_owned();
        assert_eq!(calls[0].program, stable_python);
        assert_eq!(calls[1].program, stable_python);
        assert_eq!(calls[2].program, "python3.14");
        assert_eq!(&calls[3].args[..3], ["-m", "venv", "--copies"]);
        assert_eq!(&calls[4].args[..2], ["-m", "pip"]);
        assert_eq!(calls[5].program, calls[4].program);
        assert_eq!(calls[6].program, stable_python);
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_rejects_non_directory_generation_root() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        fs::create_dir_all(paths.stable.parent().unwrap()).unwrap();
        fs::write(&paths.generations, b"not a directory").unwrap();
        let runner = FakeCommandRunner::new(paths.clone(), FakeConfig::default());

        let error = install_managed_essentia_at(&paths, &runner, test_lock_config()).unwrap_err();

        assert_eq!(error.kind, EssentiaSetupErrorKind::Filesystem);
        assert_eq!(
            error.message,
            format!(
                "Failed to create managed generation directory {}: {}",
                paths.generations.display(),
                std::io::Error::from_raw_os_error(libc::EEXIST)
            )
        );
        assert_eq!(fs::read(&paths.generations).unwrap(), b"not a directory");
        assert!(!paths.stable.exists());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_installs_transactionally_and_prunes_previous_generation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let ops = FakeCommandRunner::new(
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
        assert_eq!(calls.len(), 8);
        let stable_python = paths
            .stable
            .join("bin/python")
            .to_string_lossy()
            .into_owned();
        for probe in [&calls[0], &calls[1]] {
            assert_eq!(probe.program, stable_python);
            assert_eq!(probe.args, ["-c", ESSENTIA_IMPORT_CHECK_SCRIPT]);
            assert_eq!(probe.timeout, Duration::from_secs(30));
        }
        assert_eq!(calls[2].program, "python3.14");
        assert_eq!(calls[2].timeout, Duration::from_secs(5));
        assert_eq!(calls[3].program, "python3");
        assert_eq!(calls[3].timeout, Duration::from_secs(5));
        let venv = &calls[4];
        assert_eq!(venv.program, "python3");
        assert_eq!(&venv.args[..3], ["-m", "venv", "--copies"]);
        assert_eq!(venv.timeout, Duration::from_secs(120));
        let pip = &calls[5];
        assert!(pip.program.ends_with("/bin/python"));
        assert_eq!(pip.args, pip_install_args());
        assert_eq!(pip.timeout, Duration::from_secs(600));
        assert_eq!(calls[6].program, pip.program);
        assert_eq!(calls[6].args, ["-c", ESSENTIA_IMPORT_CHECK_SCRIPT]);
        assert_eq!(calls[6].timeout, Duration::from_secs(30));
        assert_eq!(calls[7].program, stable_python);
        assert_eq!(calls[7].args, ["-c", ESSENTIA_IMPORT_CHECK_SCRIPT]);
        assert_eq!(calls[7].timeout, Duration::from_secs(30));
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
        let ops = FakeCommandRunner::new(paths.clone(), FakeConfig::default());

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
        let ops = FakeCommandRunner::new(paths.clone(), FakeConfig::default());

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
            let ops = FakeCommandRunner::new(paths.clone(), config);

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
        let ops = FakeCommandRunner::new(
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
        let ops = FakeCommandRunner::new(
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
    fn essentia_environment_rollback_failure_preserves_validated_generation() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        let previous = make_previous_symlink(&paths);
        let runner = FakeCommandRunner::new(
            paths.clone(),
            FakeConfig {
                stable_probe: false,
                sabotage_stable_before_failure: true,
                ..FakeConfig::default()
            },
        );

        let error = install_managed_essentia_at(&paths, &runner, test_lock_config()).unwrap_err();

        let stable_python = paths.stable.join("bin/python");
        let stable_failure = format!(
            "ManifestMismatch: runtime at {} does not match the supported manifest (implementation=cpython, python=3.14.6, essentia_distribution=2.1b6.dev1438, essentia_module=2.1-beta6-dev, numpy=0.0.0, pyyaml=6.0.3, six=1.17.0)",
            stable_python.display()
        );
        let restore_failure = format!(
            "Failed to restore managed runtime symlink: {}",
            std::io::Error::from_raw_os_error(libc::EISDIR)
        );
        assert_eq!(error.kind, EssentiaSetupErrorKind::Activation);
        assert_eq!(
            error.message,
            format!(
                "Validated generation failed through the stable managed path ({stable_failure}), and the prior runtime could not be restored; the directly validated generation was preserved: {restore_failure}"
            )
        );
        assert!(paths.stable.is_dir());
        assert!(previous.exists());
        let generations = fs::read_dir(&paths.generations)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(generations.len(), 2);
        let validated = generations
            .iter()
            .find(|generation| generation.as_path() != previous)
            .expect("directly validated generation should be retained");
        assert!(validated.join("bin/python").is_file());
        assert_no_switch_artifacts(dir.path());
    }

    #[test]
    #[cfg(unix)]
    fn essentia_environment_stable_probe_failure_restores_legacy_directory() {
        let dir = tempfile::tempdir().unwrap();
        let paths = test_paths(dir.path());
        fs::create_dir_all(paths.stable.join("bin")).unwrap();
        fs::write(paths.stable.join("bin/python"), b"legacy python").unwrap();
        let ops = FakeCommandRunner::new(
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
        let ops = FakeCommandRunner::new(paths.clone(), FakeConfig::default());

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
