use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::activation::{LockConfig, ManagedEnvironmentPaths};

pub(super) fn test_paths(root: &Path) -> ManagedEnvironmentPaths {
    ManagedEnvironmentPaths::from_stable(root.join("essentia-venv")).unwrap()
}

pub(super) fn test_lock_config() -> LockConfig {
    LockConfig {
        timeout: Duration::from_millis(50),
        poll: Duration::from_millis(1),
    }
}

#[cfg(unix)]
pub(super) fn make_previous_symlink(paths: &ManagedEnvironmentPaths) -> PathBuf {
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

pub(super) fn assert_no_switch_artifacts(parent: &Path) {
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
