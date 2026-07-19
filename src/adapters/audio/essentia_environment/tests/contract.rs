use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use super::super::contract::{
    ESSENTIA_CONTRACT_ID, ESSENTIA_VENV_RELPATH, EssentiaSetupError, EssentiaSetupErrorKind,
    generic_process_error, inspect_essentia_python_with_timeout,
    inspect_essentia_python_with_timeout_result, probe_essentia_runtime_from_sources,
    probe_process_error,
};

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
    use super::super::process::{OutputStream, ProcessError, ProcessErrorKind};

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
