//! Managed runtime paths, locking, activation, rollback, and pruning.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::contract::{EssentiaSetupError, EssentiaSetupErrorKind};
use super::platform;

pub(crate) const ESSENTIA_VENV_GENERATIONS: &str = "essentia-venv.generations";

#[derive(Debug, Clone)]
pub(super) struct ManagedEnvironmentPaths {
    pub(super) stable: PathBuf,
    pub(super) generations: PathBuf,
    pub(super) lock: PathBuf,
}

impl ManagedEnvironmentPaths {
    pub(super) fn from_stable(stable: PathBuf) -> Result<Self, EssentiaSetupError> {
        let parent = stable
            .parent()
            .ok_or("Managed Essentia path has no parent")?;
        Ok(Self {
            generations: parent.join(ESSENTIA_VENV_GENERATIONS),
            lock: parent.join("essentia-venv.lock"),
            stable,
        })
    }

    pub(super) fn prepare_parent(&self) -> Result<(), EssentiaSetupError> {
        let parent = self
            .stable
            .parent()
            .ok_or("Managed Essentia path has no parent")?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create {}: {error}", parent.display()).into())
    }

    pub(super) fn prepare_generation(&self) -> Result<IncompleteGeneration, EssentiaSetupError> {
        if fs::symlink_metadata(&self.generations)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(EssentiaSetupError::new(
                EssentiaSetupErrorKind::Filesystem,
                format!(
                    "Managed generation directory must not be a symlink: {}",
                    self.generations.display()
                ),
            ));
        }
        fs::create_dir_all(&self.generations).map_err(|error| {
            format!(
                "Failed to create managed generation directory {}: {error}",
                self.generations.display()
            )
        })?;
        let metadata = fs::symlink_metadata(&self.generations).map_err(|error| {
            format!(
                "Failed to inspect managed generation directory {}: {error}",
                self.generations.display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(EssentiaSetupError::new(
                EssentiaSetupErrorKind::Filesystem,
                format!(
                    "Managed generation path is not a real directory: {}",
                    self.generations.display()
                ),
            ));
        }
        Ok(IncompleteGeneration::new(self.generations.join(format!(
            "runtime-{}-{}",
            std::process::id(),
            unique_suffix()
        ))))
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct LockConfig {
    pub(super) timeout: Duration,
    pub(super) poll: Duration,
}

/// A process-owned advisory lock. Unlike a create-new sentinel, the kernel
/// releases this lock when a process exits unexpectedly.
#[derive(Debug)]
pub(super) struct AdvisoryLock {
    file: File,
}

impl AdvisoryLock {
    pub(super) fn acquire(
        path: &Path,
        timeout: Duration,
        poll: Duration,
    ) -> Result<Self, EssentiaSetupError> {
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

pub(super) struct IncompleteGeneration {
    path: PathBuf,
    committed: bool,
}

impl IncompleteGeneration {
    pub(super) fn new(path: PathBuf) -> Self {
        Self {
            path,
            committed: false,
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for IncompleteGeneration {
    fn drop(&mut self) {
        if !self.committed
            && self.path.exists()
            && let Err(error) = fs::remove_dir_all(&self.path)
        {
            tracing::warn!(
                "Failed to remove incomplete Essentia generation {}: {error}",
                self.path.display()
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActivationPhase {
    Prepared,
    Switched,
    StableValidated,
    Committed,
    RolledBack,
    RollbackFailed,
}

pub(super) struct ActivationTransaction<'a> {
    paths: &'a ManagedEnvironmentPaths,
    generation: IncompleteGeneration,
    previous: Option<PathBuf>,
    phase: ActivationPhase,
}

impl<'a> ActivationTransaction<'a> {
    pub(super) fn prepare(
        paths: &'a ManagedEnvironmentPaths,
        generation: IncompleteGeneration,
    ) -> Self {
        Self {
            paths,
            generation,
            previous: None,
            phase: ActivationPhase::Prepared,
        }
    }

    pub(super) fn switch(&mut self) -> Result<(), ActivationError> {
        self.require_phase("switch", ActivationPhase::Prepared)?;
        self.previous = switch_stable_generation(&self.paths.stable, self.generation.path())?;
        self.phase = ActivationPhase::Switched;
        Ok(())
    }

    pub(super) fn stable_validated(&mut self) -> Result<(), ActivationError> {
        self.require_phase("record stable validation", ActivationPhase::Switched)?;
        self.phase = ActivationPhase::StableValidated;
        Ok(())
    }

    pub(super) fn rollback(&mut self) -> Result<(), ActivationError> {
        self.require_phase("rollback", ActivationPhase::Switched)?;
        match restore_stable_generation(&self.paths.stable, self.previous.clone()) {
            Ok(()) => {
                self.phase = ActivationPhase::RolledBack;
                Ok(())
            }
            Err(error) => {
                self.generation.commit();
                self.phase = ActivationPhase::RollbackFailed;
                Err(error)
            }
        }
    }

    pub(super) fn commit(&mut self) -> Result<(), ActivationError> {
        self.require_phase("commit", ActivationPhase::StableValidated)?;
        self.generation.commit();
        prune_superseded_runtime(&self.paths.stable, self.previous.clone());
        self.phase = ActivationPhase::Committed;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn phase(&self) -> ActivationPhase {
        self.phase
    }

    fn require_phase(
        &self,
        operation: &'static str,
        expected: ActivationPhase,
    ) -> Result<(), ActivationError> {
        if self.phase == expected {
            Ok(())
        } else {
            Err(ActivationError::InvalidPhase {
                operation,
                expected,
                actual: self.phase,
            })
        }
    }
}

impl Drop for ActivationTransaction<'_> {
    fn drop(&mut self) {
        if !matches!(
            self.phase,
            ActivationPhase::Switched | ActivationPhase::StableValidated
        ) {
            return;
        }
        match restore_stable_generation(&self.paths.stable, self.previous.clone()) {
            Ok(()) => {
                self.phase = ActivationPhase::RolledBack;
                tracing::warn!(
                    "Emergency rollback restored the prior managed Essentia runtime during drop"
                );
            }
            Err(error) => {
                self.generation.commit();
                self.phase = ActivationPhase::RollbackFailed;
                tracing::warn!(
                    "Emergency rollback could not restore the prior managed Essentia runtime; preserving {}: {error}",
                    self.generation.path().display()
                );
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum ActivationError {
    MissingParent,
    #[cfg(not(unix))]
    SymlinkUnsupported,
    CreateSwitch(std::io::Error),
    ActivateLegacy(platform::AtomicExchangeError),
    ActivateValidated(std::io::Error),
    RestoreLegacy(platform::AtomicExchangeError),
    RemoveRejectedLegacyLink {
        path: PathBuf,
        source: std::io::Error,
    },
    PrepareRestoration(std::io::Error),
    RestoreSymlink(std::io::Error),
    RemoveRejected(std::io::Error),
    InvalidPhase {
        operation: &'static str,
        expected: ActivationPhase,
        actual: ActivationPhase,
    },
}

impl std::fmt::Display for ActivationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingParent => formatter.write_str("Managed Essentia path has no parent"),
            #[cfg(not(unix))]
            Self::SymlinkUnsupported => {
                formatter.write_str("Managed Essentia generations require Unix symlink support")
            }
            Self::CreateSwitch(error) => {
                write!(
                    formatter,
                    "Failed to create managed runtime switch: {error}"
                )
            }
            Self::ActivateLegacy(error) => write!(
                formatter,
                "Failed to atomically activate the managed runtime over the legacy directory: {error}"
            ),
            Self::ActivateValidated(error) => write!(
                formatter,
                "Failed to atomically activate validated managed runtime: {error}"
            ),
            Self::RestoreLegacy(error) => {
                write!(
                    formatter,
                    "Failed to restore legacy managed runtime: {error}"
                )
            }
            Self::RemoveRejectedLegacyLink { path, source } => write!(
                formatter,
                "Legacy runtime was restored, but the failed runtime link {} could not be removed: {source}",
                path.display()
            ),
            Self::PrepareRestoration(error) => write!(
                formatter,
                "Failed to prepare managed runtime restoration: {error}"
            ),
            Self::RestoreSymlink(error) => {
                write!(
                    formatter,
                    "Failed to restore managed runtime symlink: {error}"
                )
            }
            Self::RemoveRejected(error) => {
                write!(
                    formatter,
                    "Failed to remove rejected managed runtime: {error}"
                )
            }
            Self::InvalidPhase {
                operation,
                expected,
                actual,
            } => write!(
                formatter,
                "Cannot {operation} managed runtime while activation is {actual:?}; expected {expected:?}"
            ),
        }
    }
}

impl std::error::Error for ActivationError {}

pub(super) fn setup_activation_error(error: ActivationError) -> EssentiaSetupError {
    EssentiaSetupError::new(EssentiaSetupErrorKind::Activation, error.to_string())
}

fn switch_stable_generation(
    stable: &Path,
    generation: &Path,
) -> Result<Option<PathBuf>, ActivationError> {
    let parent = stable.parent().ok_or(ActivationError::MissingParent)?;
    let legacy = (stable.exists() && !stable.is_symlink())
        .then(|| parent.join(format!("essentia-venv.legacy-{}", unique_suffix())));
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
    std::os::unix::fs::symlink(relative, &replacement).map_err(ActivationError::CreateSwitch)?;
    #[cfg(not(unix))]
    return Err(ActivationError::SymlinkUnsupported);
    if let Some(legacy) = legacy {
        if let Err(error) = platform::atomic_exchange_paths(stable, &replacement) {
            let _ = fs::remove_file(&replacement);
            return Err(ActivationError::ActivateLegacy(error));
        }
        debug_assert_eq!(replacement, legacy);
        return Ok(Some(legacy));
    }
    if let Err(error) = fs::rename(&replacement, stable) {
        let _ = fs::remove_file(&replacement);
        return Err(ActivationError::ActivateValidated(error));
    }
    Ok(previous)
}

fn restore_stable_generation(
    stable: &Path,
    previous: Option<PathBuf>,
) -> Result<(), ActivationError> {
    let parent = stable.parent().ok_or(ActivationError::MissingParent)?;
    match previous {
        Some(previous)
            if previous.file_name().is_some_and(|name| {
                name.to_string_lossy().starts_with("essentia-venv.legacy-")
            }) =>
        {
            platform::atomic_exchange_paths(stable, &previous)
                .map_err(ActivationError::RestoreLegacy)?;
            fs::remove_file(&previous).map_err(|source| {
                ActivationError::RemoveRejectedLegacyLink {
                    path: previous,
                    source,
                }
            })?;
        }
        Some(target) => {
            #[cfg(unix)]
            {
                let replacement =
                    parent.join(format!(".essentia-venv-restore-{}", unique_suffix()));
                std::os::unix::fs::symlink(target, &replacement)
                    .map_err(ActivationError::PrepareRestoration)?;
                if let Err(error) = fs::rename(&replacement, stable) {
                    let _ = fs::remove_file(&replacement);
                    return Err(ActivationError::RestoreSymlink(error));
                }
            }
            #[cfg(not(unix))]
            return Err(ActivationError::SymlinkUnsupported);
        }
        None => {
            fs::remove_file(stable).map_err(ActivationError::RemoveRejected)?;
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

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}
