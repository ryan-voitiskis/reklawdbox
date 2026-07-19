use std::fs;
use std::time::Duration;

use super::super::activation::{
    ActivationPhase, ActivationTransaction, AdvisoryLock, ESSENTIA_VENV_GENERATIONS,
    IncompleteGeneration,
};
use super::super::contract::{ESSENTIA_IMPORT_CHECK_SCRIPT, EssentiaSetupErrorKind};
use super::super::install::{install_managed_essentia_at, pip_install_args};
use super::fake_command::{FakeCommandRunner, FakeConfig};
use super::support::{
    assert_no_switch_artifacts, make_previous_symlink, test_lock_config, test_paths,
};

#[test]
fn essentia_environment_lock_times_out_then_reacquires_after_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("setup.lock");
    let first =
        AdvisoryLock::acquire(&path, Duration::from_millis(20), Duration::from_millis(1)).unwrap();
    let error = AdvisoryLock::acquire(&path, Duration::from_millis(20), Duration::from_millis(1))
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
