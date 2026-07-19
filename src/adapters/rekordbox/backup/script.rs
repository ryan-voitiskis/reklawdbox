use std::io::Write;
use std::path::{Path, PathBuf};

use super::error::{BackupError, BackupErrorKind};

const BACKUP_SCRIPT: &str = include_str!("../../../../scripts/backup.sh");

pub(super) struct PreparedScript {
    path: PathBuf,
    _temp_dir: Option<tempfile::TempDir>,
}

impl PreparedScript {
    pub(super) fn configured_custom() -> Result<Option<Self>, BackupError> {
        let configured = std::env::var("REKLAWDBOX_BACKUP_SCRIPT")
            .ok()
            .filter(|value| !value.trim().is_empty());
        configured
            .map(|path| {
                let path = PathBuf::from(path);
                if !path.is_file() {
                    return Err(BackupError::new(BackupErrorKind::ScriptPreparation(
                        format!(
                            "custom backup script is missing or not a file: {}",
                            path.display()
                        ),
                    )));
                }
                Ok(Self::borrowed(path))
            })
            .transpose()
    }

    pub(super) fn embedded() -> Result<Self, BackupError> {
        let (path, temp_dir) = materialize_embedded()?;
        Ok(Self {
            path,
            _temp_dir: Some(temp_dir),
        })
    }

    pub(super) fn borrowed(path: PathBuf) -> Self {
        Self {
            path,
            _temp_dir: None,
        }
    }

    #[cfg(test)]
    pub(super) fn owned_for_test(path: PathBuf, temp_dir: tempfile::TempDir) -> Self {
        Self {
            path,
            _temp_dir: Some(temp_dir),
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

fn materialize_embedded() -> Result<(PathBuf, tempfile::TempDir), BackupError> {
    let temp_dir = tempfile::tempdir().map_err(|error| {
        BackupError::new(BackupErrorKind::ScriptPreparation(format!(
            "Failed to create temp dir for backup: {error}"
        )))
    })?;
    let script_path = temp_dir.path().join("backup.sh");

    {
        let mut file = std::fs::File::create(&script_path).map_err(|error| {
            BackupError::new(BackupErrorKind::ScriptPreparation(format!(
                "Failed to create temp backup script: {error}"
            )))
        })?;
        file.write_all(BACKUP_SCRIPT.as_bytes()).map_err(|error| {
            BackupError::new(BackupErrorKind::ScriptPreparation(format!(
                "Failed to write backup script: {error}"
            )))
        })?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).map_err(
            |error| {
                BackupError::new(BackupErrorKind::ScriptPreparation(format!(
                    "Failed to chmod backup script: {error}"
                )))
            },
        )?;
    }

    Ok((script_path, temp_dir))
}

#[cfg(test)]
pub(crate) fn write_embedded_script_for_test() -> Result<(PathBuf, tempfile::TempDir), String> {
    materialize_embedded().map_err(|error| error.to_string())
}
