use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

const FAILURE_OUTPUT_LIMIT: usize = 8 * 1024;

/// The embedded backup script, compiled into the binary.
const BACKUP_SCRIPT: &str = include_str!("../scripts/backup.sh");

/// Backup execution outcome, reported in write_xml response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupStatus {
    /// Embedded or custom script ran successfully.
    Success,
}

impl BackupStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
        }
    }
}

/// Run a pre-operation (db-only) backup before write_xml.
///
/// Resolution order:
/// 1. `REKLAWDBOX_BACKUP_SCRIPT` env var (custom script override)
/// 2. Embedded script written to a temp file
///
pub(crate) async fn run_pre_op_backup(db_path: &Path) -> Result<BackupStatus, String> {
    let custom_script = std::env::var("REKLAWDBOX_BACKUP_SCRIPT")
        .ok()
        .filter(|v| !v.trim().is_empty());
    let path_env = [("REKORDBOX_DB_PATH", db_path.as_os_str())];

    if let Some(ref script_path) = custom_script {
        let script_path = Path::new(script_path);
        if !script_path.is_file() {
            return Err(format!(
                "custom backup script is missing or not a file: {}",
                script_path.display()
            ));
        }
        tracing::info!("Running custom pre-op backup: {}", script_path.display());
        return execute_script(script_path, &["--pre-op"], &path_env)
            .await
            .map(|()| BackupStatus::Success);
    }

    // In test builds, don't auto-run the embedded script — tests use
    // REKLAWDBOX_BACKUP_SCRIPT env var to test backup behavior explicitly.
    if cfg!(test) {
        return Ok(BackupStatus::Success);
    }

    tracing::info!("Running embedded pre-op backup...");
    execute_embedded_with_env(&["--pre-op"], &path_env)
        .await
        .map(|()| BackupStatus::Success)
}

/// Execute backup using the embedded script (captures stdout/stderr).
/// Used by the MCP pre-op path.
pub(crate) async fn execute_embedded_with_env(
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
) -> Result<(), String> {
    let (script_path, _tmp_dir) = write_embedded_script()?;
    execute_script(&script_path, args, env_additions).await
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
fn write_embedded_script() -> Result<(PathBuf, tempfile::TempDir), String> {
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

#[cfg(test)]
pub(crate) fn write_embedded_script_for_test() -> Result<(PathBuf, tempfile::TempDir), String> {
    write_embedded_script()
}

/// Execute a backup script at the given path (captures stdout/stderr).
async fn execute_script(
    script_path: &Path,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
) -> Result<(), String> {
    let script_path = script_path.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let env_additions: Vec<(OsString, OsString)> = env_additions
        .iter()
        .map(|(key, value)| (OsString::from(key), (*value).to_os_string()))
        .collect();

    match tokio::task::spawn_blocking(move || -> Result<BoundedChildOutput, String> {
        let mut command = std::process::Command::new("bash");
        command.arg(&script_path).args(&args);
        for (key, value) in env_additions {
            command.env(key, value);
        }
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("backup launch failed: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "backup stdout capture was unavailable".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "backup stderr capture was unavailable".to_string())?;
        let stdout_task = std::thread::spawn(move || read_bounded_output(stdout));
        let stderr_task = std::thread::spawn(move || read_bounded_output(stderr));
        let status = child
            .wait()
            .map_err(|error| format!("backup wait failed: {error}"))?;
        let stdout = stdout_task
            .join()
            .map_err(|_| "backup stdout capture task panicked".to_string())?
            .map_err(|error| format!("backup stdout capture failed: {error}"))?;
        let stderr = stderr_task
            .join()
            .map_err(|_| "backup stderr capture task panicked".to_string())?
            .map_err(|error| format!("backup stderr capture failed: {error}"))?;
        Ok(BoundedChildOutput {
            status,
            stdout,
            stderr,
        })
    })
    .await
    {
        Ok(Ok(output)) if output.status.success() => {
            tracing::info!("Backup completed.");
            Ok(())
        }
        Ok(Ok(output)) => {
            let stderr = output.stderr.render();
            let stdout = output.stdout.render();
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
        Ok(Err(error)) => Err(error),
        Err(e) => Err(format!("backup task failed: {e}")),
    }
}

struct BoundedChildOutput {
    status: ExitStatus,
    stdout: BoundedOutput,
    stderr: BoundedOutput,
}

struct BoundedOutput {
    bytes: Vec<u8>,
    truncated: bool,
}

impl BoundedOutput {
    fn render(self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).trim().to_string();
        if self.truncated {
            text.push_str(" …[truncated]");
        }
        text
    }
}

fn read_bounded_output(mut reader: impl Read) -> std::io::Result<BoundedOutput> {
    let mut bytes = Vec::with_capacity(FAILURE_OUTPUT_LIMIT);
    let mut truncated = false;
    let mut chunk = [0_u8; 4 * 1024];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        let remaining = FAILURE_OUTPUT_LIMIT.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok(BoundedOutput { bytes, truncated })
}
