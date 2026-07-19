use std::fs;

use super::super::contract::{ESSENTIA_PROBE_TIMEOUT_SECS, EssentiaSetupErrorKind};
use super::super::install::{PYTHON_CANDIDATES, install_managed_essentia_at, pip_install_args};
use super::fake_command::{FakeCommandRunner, FakeConfig};
use super::support::{
    assert_no_switch_artifacts, make_previous_symlink, test_lock_config, test_paths,
};

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
