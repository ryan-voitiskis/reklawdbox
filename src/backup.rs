use std::io::Write;

/// The embedded backup script, compiled into the binary.
const BACKUP_SCRIPT: &str = include_str!("../scripts/backup.sh");

/// Backup execution outcome, reported in write_xml response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupStatus {
    /// Embedded or custom script ran successfully.
    Success,
    /// Custom script (env override) not found on disk; backup was skipped.
    CustomNotFound,
}

impl BackupStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::CustomNotFound => "skipped_custom_not_found",
        }
    }
}

/// Run a pre-operation (db-only) backup before write_xml.
///
/// Resolution order:
/// 1. `REKLAWDBOX_BACKUP_SCRIPT` env var (custom script override)
/// 2. Embedded script written to a temp file
///
/// If the env var is set but the file does not exist, returns
/// `Ok(BackupStatus::CustomNotFound)` rather than failing.
pub(crate) async fn run_pre_op_backup() -> Result<BackupStatus, String> {
    let custom_script = std::env::var("REKLAWDBOX_BACKUP_SCRIPT")
        .ok()
        .filter(|v| !v.trim().is_empty());

    if let Some(ref script_path) = custom_script {
        if !std::path::Path::new(script_path).exists() {
            tracing::warn!(
                "REKLAWDBOX_BACKUP_SCRIPT={script_path} does not exist; skipping backup"
            );
            return Ok(BackupStatus::CustomNotFound);
        }
        tracing::info!("Running custom pre-op backup: {script_path}");
        return execute_script(script_path, &["--pre-op"])
            .await
            .map(|()| BackupStatus::Success);
    }

    // In test builds, don't auto-run the embedded script — tests use
    // REKLAWDBOX_BACKUP_SCRIPT env var to test backup behavior explicitly.
    if cfg!(test) {
        return Ok(BackupStatus::Success);
    }

    tracing::info!("Running embedded pre-op backup...");
    execute_embedded(&["--pre-op"])
        .await
        .map(|()| BackupStatus::Success)
}

/// Execute backup using the embedded script (captures stdout/stderr).
/// Used by the MCP pre-op path.
pub(crate) async fn execute_embedded(args: &[&str]) -> Result<(), String> {
    let (script_path, _tmp_dir) = write_embedded_script()?;
    let path_str = script_path.to_string_lossy().to_string();
    execute_script(&path_str, args).await
}

/// Execute the embedded script interactively (stdin/stdout/stderr passthrough).
/// Used by the CLI subcommand.
pub(crate) async fn execute_embedded_interactive(args: &[&str]) -> Result<(), String> {
    let (script_path, _tmp_dir) = write_embedded_script()?;
    let path_str = script_path.to_string_lossy().to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    let status = tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&path_str)
            .args(&args)
            .status()
    })
    .await
    .map_err(|e| format!("backup task failed: {e}"))?
    .map_err(|e| format!("backup launch failed: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "backup failed with exit status {}",
            status
                .code()
                .map_or_else(|| "signal".to_string(), |c| c.to_string()),
        ))
    }
}

/// Write the embedded script to a temp file and return the path.
/// The caller must hold the returned `TempDir` to prevent cleanup.
fn write_embedded_script() -> Result<(std::path::PathBuf, tempfile::TempDir), String> {
    let tmp_dir =
        tempfile::tempdir().map_err(|e| format!("Failed to create temp dir for backup: {e}"))?;
    let script_path = tmp_dir.path().join("backup.sh");

    {
        let mut file = std::fs::File::create(&script_path)
            .map_err(|e| format!("Failed to create temp backup script: {e}"))?;
        file.write_all(BACKUP_SCRIPT.as_bytes())
            .map_err(|e| format!("Failed to write backup script: {e}"))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to chmod backup script: {e}"))?;
    }

    Ok((script_path, tmp_dir))
}

/// Execute a backup script at the given path (captures stdout/stderr).
async fn execute_script(script_path: &str, args: &[&str]) -> Result<(), String> {
    let script_path = script_path.to_string();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();

    match tokio::task::spawn_blocking(move || {
        std::process::Command::new("bash")
            .arg(&script_path)
            .args(&args)
            .output()
    })
    .await
    {
        Ok(Ok(output)) if output.status.success() => {
            tracing::info!("Backup completed.");
            Ok(())
        }
        Ok(Ok(output)) => {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let details = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                "backup script exited without output".to_string()
            };
            Err(format!(
                "backup failed with exit status {}: {}",
                output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |c| c.to_string()),
                details
            ))
        }
        Ok(Err(e)) => Err(format!("backup launch failed: {e}")),
        Err(e) => Err(format!("backup task failed: {e}")),
    }
}
