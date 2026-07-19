use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[path = "backup/output.rs"]
mod output;

use output::{BoundedOutput, OutputReaderTask};

const PRE_OP_BACKUP_TIMEOUT: Duration = Duration::from_secs(120);
const CHILD_TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);
const OUTPUT_READER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);
const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

#[cfg(test)]
static PRE_OP_BACKUP_TIMEOUT_OVERRIDE_MILLIS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// The embedded backup script, compiled into the binary.
const BACKUP_SCRIPT: &str = include_str!("../../../scripts/backup.sh");

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
    run_pre_op_backup_with_timeout(db_path, effective_pre_op_backup_timeout()).await
}

async fn run_pre_op_backup_with_timeout(
    db_path: &Path,
    timeout: Duration,
) -> Result<BackupStatus, String> {
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
        return execute_script_with_timeout(script_path, &["--pre-op"], &path_env, timeout)
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

fn effective_pre_op_backup_timeout() -> Duration {
    #[cfg(test)]
    {
        let millis =
            PRE_OP_BACKUP_TIMEOUT_OVERRIDE_MILLIS.load(std::sync::atomic::Ordering::Acquire);
        if millis > 0 {
            return Duration::from_millis(millis);
        }
    }
    PRE_OP_BACKUP_TIMEOUT
}

#[cfg(test)]
pub(crate) struct PreOpBackupTimeoutOverride {
    previous_millis: u64,
}

#[cfg(test)]
impl Drop for PreOpBackupTimeoutOverride {
    fn drop(&mut self) {
        PRE_OP_BACKUP_TIMEOUT_OVERRIDE_MILLIS
            .store(self.previous_millis, std::sync::atomic::Ordering::Release);
    }
}

#[cfg(test)]
pub(crate) fn override_pre_op_backup_timeout_for_test(
    timeout: Duration,
) -> PreOpBackupTimeoutOverride {
    let millis = u64::try_from(timeout.as_millis()).expect("test timeout should fit in u64");
    assert!(millis > 0, "test timeout must be at least one millisecond");
    let previous_millis =
        PRE_OP_BACKUP_TIMEOUT_OVERRIDE_MILLIS.swap(millis, std::sync::atomic::Ordering::AcqRel);
    PreOpBackupTimeoutOverride { previous_millis }
}

/// Execute backup using the embedded script (captures stdout/stderr).
/// Used by the MCP pre-op path.
pub(crate) async fn execute_embedded_with_env(
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
) -> Result<(), String> {
    execute_embedded_with_env_and_timeout(args, env_additions, PRE_OP_BACKUP_TIMEOUT).await
}

async fn execute_embedded_with_env_and_timeout(
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
) -> Result<(), String> {
    let (script_path, _tmp_dir) = write_embedded_script()?;
    execute_script_with_timeout(&script_path, args, env_additions, timeout).await
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
async fn execute_script_with_timeout(
    script_path: &Path,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
) -> Result<(), String> {
    execute_script_with_timeout_and_activity(
        script_path,
        args,
        env_additions,
        timeout,
        Arc::new(AtomicUsize::new(0)),
    )
    .await
}

async fn execute_script_with_timeout_and_activity(
    script_path: &Path,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
) -> Result<(), String> {
    let script_path = script_path.to_path_buf();
    let args = args.iter().map(|arg| (*arg).to_string()).collect();
    let env_additions = env_additions
        .iter()
        .map(|(key, value)| (OsString::from(key), (*value).to_os_string()))
        .collect();
    let supervisor = tokio::spawn(supervise_backup_script(
        script_path,
        args,
        env_additions,
        timeout,
        reader_activity,
    ));
    supervisor
        .await
        .map_err(|error| format!("backup supervisor task failed: {error}"))?
}

async fn supervise_backup_script(
    script_path: PathBuf,
    args: Vec<String>,
    env_additions: Vec<(OsString, OsString)>,
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new("bash");
    command.arg(&script_path).args(&args);
    for (key, value) in env_additions {
        command.env(key, value);
    }
    command.kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.as_std_mut().process_group(0);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("backup launch failed: {error}"))?;
    let mut process_group = match ProcessGroupOwnership::new(child.id()) {
        Ok(process_group) => process_group,
        Err(error) => {
            return Err(cleanup_spawned_child_after_setup_failure(&mut child, None, error).await);
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            return Err(cleanup_spawned_child_after_setup_failure(
                &mut child,
                Some(&mut process_group),
                "backup stdout capture was unavailable".to_string(),
            )
            .await);
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            return Err(cleanup_spawned_child_after_setup_failure(
                &mut child,
                Some(&mut process_group),
                "backup stderr capture was unavailable".to_string(),
            )
            .await);
        }
    };
    let mut stdout_task = OutputReaderTask::spawn("stdout", stdout, Arc::clone(&reader_activity));
    let mut stderr_task = OutputReaderTask::spawn("stderr", stderr, Arc::clone(&reader_activity));
    let deadline = tokio::time::Instant::now() + timeout;

    let operation = async {
        #[cfg(unix)]
        let status = {
            process_group.wait_for_leader_exit_without_reaping().await?;
            let descendants = process_group.inspect_and_release_before_reap()?;
            let status = reap_leader_after_group_release(&mut child, &mut process_group).await?;
            if descendants {
                return Err(
                    "backup script exited while descendant processes were still running"
                        .to_string(),
                );
            }
            status
        };
        #[cfg(not(unix))]
        let status = child
            .wait()
            .await
            .map_err(|error| format!("backup wait failed: {error}"))?;
        let (stdout, stderr) = tokio::try_join!(stdout_task.finish(), stderr_task.finish())?;
        Ok(BoundedChildOutput {
            status,
            stdout,
            stderr,
        })
    };

    let output = match strict_timeout_at(deadline, operation).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let context = cleanup_child_and_readers(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
                &reader_activity,
            )
            .await;
            return Err(with_cleanup_context(error, context));
        }
        Err(()) => {
            let context = cleanup_child_and_readers(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
                &reader_activity,
            )
            .await;
            return Err(with_cleanup_context(
                format!(
                    "pre-operation backup timed out after {}",
                    duration_label(timeout)
                ),
                context,
            ));
        }
    };
    debug_assert_eq!(reader_activity.load(Ordering::Acquire), 0);

    match output {
        output if output.status.success() => {
            tracing::info!("Backup completed.");
            Ok(())
        }
        output => {
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
    }
}

#[cfg(test)]
pub(crate) async fn execute_script_with_timeout_for_test(
    script_path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    execute_script_with_timeout(script_path, &["--pre-op"], &[], timeout).await
}

#[cfg(test)]
pub(crate) async fn execute_script_with_timeout_and_activity_for_test(
    script_path: &Path,
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
) -> Result<(), String> {
    execute_script_with_timeout_and_activity(
        script_path,
        &["--pre-op"],
        &[],
        timeout,
        reader_activity,
    )
    .await
}

async fn strict_timeout_at<F, T>(deadline: tokio::time::Instant, future: F) -> Result<T, ()>
where
    F: std::future::Future<Output = T>,
{
    let result = tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| ())?;
    if tokio::time::Instant::now() >= deadline {
        Err(())
    } else {
        Ok(result)
    }
}

#[cfg(unix)]
struct ProcessGroupOwnership {
    leader_pid: i32,
    process_group: Option<i32>,
}

#[cfg(unix)]
impl ProcessGroupOwnership {
    fn new(child_id: Option<u32>) -> Result<Self, String> {
        let child_id = child_id.ok_or_else(|| "backup child PID was unavailable".to_string())?;
        let process_group = i32::try_from(child_id)
            .map_err(|error| format!("child PID conversion failed: {error}"))?;
        Ok(Self {
            leader_pid: process_group,
            process_group: Some(process_group),
        })
    }

    async fn wait_for_leader_exit_without_reaping(&self) -> Result<(), String> {
        loop {
            if leader_exit_observed_without_reaping(self.leader_pid)? {
                return Ok(());
            }
            tokio::time::sleep(CHILD_EXIT_POLL_INTERVAL).await;
        }
    }

    fn inspect_and_release_before_reap(&mut self) -> Result<bool, String> {
        let Some(process_group) = self.process_group else {
            return Err("backup process group was released before inspection".to_string());
        };
        // Freeze any live descendants while the unreaped leader still reserves
        // the numeric PGID. This makes the membership snapshot stable enough
        // to decide whether the group can be relinquished without signalling.
        let stop_result = signal_process_group(process_group, libc::SIGSTOP);
        match process_group_has_other_members(process_group, self.leader_pid) {
            Ok(false)
                if stop_result.is_ok()
                    || stop_result.as_ref().is_err_and(|error| {
                        matches!(error.raw_os_error(), Some(libc::EPERM) | Some(libc::ESRCH))
                    }) =>
            {
                self.relinquish_without_signal();
                Ok(false)
            }
            Ok(false) => {
                let stop_error = stop_result.expect_err("failed stop should carry an error");
                let mut context = Vec::new();
                self.terminate_owned(&mut context);
                Err(with_cleanup_context(
                    format!("backup process-group freeze failed: {stop_error}"),
                    context,
                ))
            }
            Ok(true) => {
                let mut context = Vec::new();
                self.terminate_owned(&mut context);
                if context.is_empty() {
                    Ok(true)
                } else {
                    Err(format!(
                        "backup process-group cleanup failed: {}",
                        context.join("; ")
                    ))
                }
            }
            Err(error) => {
                let mut context = Vec::new();
                self.terminate_owned(&mut context);
                Err(with_cleanup_context(error, context))
            }
        }
    }

    fn relinquish_without_signal(&mut self) {
        self.process_group = None;
    }

    fn terminate_owned(&mut self, context: &mut Vec<String>) {
        let Some(process_group) = self.process_group.take() else {
            return;
        };
        if let Err(error) = signal_process_group(process_group, libc::SIGKILL)
            && error.raw_os_error() != Some(libc::ESRCH)
        {
            context.push(format!("process-group termination failed: {error}"));
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) -> std::io::Result<()> {
    let result = unsafe { libc::kill(-process_group, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupOwnership {
    fn drop(&mut self) {
        let mut context = Vec::new();
        self.terminate_owned(&mut context);
        for error in context {
            tracing::warn!("Backup process-group cleanup on drop: {error}");
        }
    }
}

#[cfg(not(unix))]
struct ProcessGroupOwnership;

#[cfg(not(unix))]
impl ProcessGroupOwnership {
    fn new(_child_id: Option<u32>) -> Result<Self, String> {
        Ok(Self)
    }

    fn terminate_owned(&mut self, _context: &mut Vec<String>) {}
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn leader_exit_observed_without_reaping(leader_pid: i32) -> Result<bool, String> {
    let leader_id = libc::id_t::try_from(leader_pid)
        .map_err(|error| format!("backup child PID conversion failed: {error}"))?;
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                leader_id,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            let info = unsafe { info.assume_init() };
            let observed_pid = unsafe { info.si_pid() };
            if observed_pid == 0 {
                return Ok(false);
            }
            if observed_pid != leader_pid {
                return Err(format!(
                    "backup exit observation returned PID {observed_pid}, expected {leader_pid}"
                ));
            }
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(format!(
                "backup exit observation failed before leader reap: {error}"
            ));
        }
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
fn leader_exit_observed_without_reaping(_leader_pid: i32) -> Result<bool, String> {
    Err(format!(
        "safe pre-reap backup exit observation is unsupported on {}",
        std::env::consts::OS
    ))
}

#[cfg(target_os = "macos")]
fn process_group_has_other_members(process_group: i32, leader_pid: i32) -> Result<bool, String> {
    unsafe extern "C" {
        fn proc_listpgrppids(
            process_group: libc::pid_t,
            buffer: *mut libc::c_void,
            buffer_size: libc::c_int,
        ) -> libc::c_int;
    }

    let mut capacity = 16_usize;
    loop {
        let mut pids = vec![0; capacity];
        let buffer_size = pids
            .len()
            .checked_mul(std::mem::size_of::<libc::pid_t>())
            .and_then(|bytes| libc::c_int::try_from(bytes).ok())
            .ok_or_else(|| "backup process-group member buffer was too large".to_string())?;
        let pid_count = unsafe {
            proc_listpgrppids(
                process_group,
                pids.as_mut_ptr().cast::<libc::c_void>(),
                buffer_size,
            )
        };
        if pid_count < 0 {
            return Err(format!(
                "backup process-group member inspection failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        let pid_count = usize::try_from(pid_count)
            .map_err(|error| format!("backup process-group result was invalid: {error}"))?;
        if pid_count < capacity {
            pids.truncate(pid_count);
            let mut saw_leader = false;
            for pid in pids.into_iter().filter(|pid| *pid > 0) {
                if pid == leader_pid {
                    saw_leader = true;
                } else {
                    return Ok(true);
                }
            }
            if !saw_leader {
                return Err("backup process-group snapshot omitted the unreaped leader".to_string());
            }
            return Ok(false);
        }
        capacity = capacity
            .checked_mul(2)
            .filter(|capacity| *capacity <= 65_536)
            .ok_or_else(|| "backup process group was unexpectedly large".to_string())?;
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_group_has_other_members(process_group: i32, leader_pid: i32) -> Result<bool, String> {
    for entry in std::fs::read_dir("/proc")
        .map_err(|error| format!("backup process-group inspection failed: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("backup process-group inspection failed: {error}"))?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "backup process-group inspection failed for PID {pid}: {error}"
                ));
            }
        };
        let observed_group = linux_process_group_from_stat(&stat)
            .map_err(|error| format!("{error} for PID {pid}"))?;
        if pid != leader_pid && observed_group == process_group {
            return Ok(true);
        }
    }
    // The leader must remain visible as a zombie until the guarded reap. Its
    // presence proves the procfs scan still refers to the owned PGID identity.
    let leader_stat = std::fs::read_to_string(format!("/proc/{leader_pid}/stat"))
        .map_err(|error| format!("backup process-group snapshot omitted the leader: {error}"))?;
    let leader_group = linux_process_group_from_stat(&leader_stat)?;
    if leader_group != process_group {
        return Err(format!(
            "backup leader moved from process group {process_group} to {leader_group}"
        ));
    }
    Ok(false)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_process_group_from_stat(stat: &str) -> Result<i32, String> {
    let fields = stat
        .rsplit_once(") ")
        .map(|(_, fields)| fields)
        .ok_or_else(|| "backup process-group inspection returned malformed stat".to_string())?;
    fields
        .split_whitespace()
        .nth(2)
        .and_then(|field| field.parse::<i32>().ok())
        .ok_or_else(|| "backup process-group inspection returned invalid stat".to_string())
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
fn process_group_has_other_members(_process_group: i32, _leader_pid: i32) -> Result<bool, String> {
    Err(format!(
        "safe backup process-group inspection is unsupported on {}",
        std::env::consts::OS
    ))
}

#[cfg(unix)]
async fn reap_leader_after_group_release(
    child: &mut tokio::process::Child,
    process_group: &mut ProcessGroupOwnership,
) -> Result<ExitStatus, String> {
    if process_group.process_group.is_some() {
        return Err(
            "refusing to reap backup leader before releasing its process group".to_string(),
        );
    }
    child
        .wait()
        .await
        .map_err(|error| format!("backup wait failed: {error}"))
}

#[cfg(not(unix))]
async fn reap_leader_after_group_release(
    child: &mut tokio::process::Child,
    _process_group: &mut ProcessGroupOwnership,
) -> Result<ExitStatus, String> {
    child
        .wait()
        .await
        .map_err(|error| format!("backup wait failed: {error}"))
}

async fn cleanup_spawned_child_after_setup_failure(
    child: &mut tokio::process::Child,
    mut process_group: Option<&mut ProcessGroupOwnership>,
    error: String,
) -> String {
    let mut context = Vec::new();
    if let Some(process_group) = process_group.as_deref_mut() {
        process_group.terminate_owned(&mut context);
    }
    if let Err(error) = child.start_kill()
        && !child_already_exited_error(&error)
    {
        context.push(format!("direct-child termination failed: {error}"));
    }
    let reap = async {
        if let Some(process_group) = process_group {
            reap_leader_after_group_release(child, process_group).await
        } else {
            child
                .wait()
                .await
                .map_err(|error| format!("backup wait failed: {error}"))
        }
    };
    match tokio::time::timeout(CHILD_TERMINATION_TIMEOUT, reap).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => context.push(format!("direct-child reap failed: {error}")),
        Err(_) => context.push(format!(
            "direct child did not exit within {} after termination",
            duration_label(CHILD_TERMINATION_TIMEOUT)
        )),
    }
    with_cleanup_context(error, context)
}

async fn cleanup_child_and_readers(
    child: &mut tokio::process::Child,
    process_group: &mut ProcessGroupOwnership,
    stdout_task: &mut OutputReaderTask,
    stderr_task: &mut OutputReaderTask,
    reader_activity: &AtomicUsize,
) -> Vec<String> {
    let mut context = Vec::new();
    process_group.terminate_owned(&mut context);
    if let Err(error) = child.start_kill()
        && !child_already_exited_error(&error)
    {
        context.push(format!("direct-child termination failed: {error}"));
    }
    match tokio::time::timeout(
        CHILD_TERMINATION_TIMEOUT,
        reap_leader_after_group_release(child, process_group),
    )
    .await
    {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => context.push(format!("direct-child reap failed: {error}")),
        Err(_) => context.push(format!(
            "direct child did not exit within {} after termination",
            duration_label(CHILD_TERMINATION_TIMEOUT)
        )),
    }

    stdout_task.abort_if_running();
    stderr_task.abort_if_running();
    let reader_results = tokio::time::timeout(OUTPUT_READER_CLEANUP_TIMEOUT, async {
        tokio::join!(
            stdout_task.finish_after_abort(),
            stderr_task.finish_after_abort()
        )
    })
    .await;
    match reader_results {
        Ok((stdout, stderr)) => {
            append_output_cleanup_context("stdout", stdout, &mut context);
            append_output_cleanup_context("stderr", stderr, &mut context);
        }
        Err(_) => context.push(format!(
            "backup output capture tasks did not stop within {} after cancellation",
            duration_label(OUTPUT_READER_CLEANUP_TIMEOUT)
        )),
    }
    if reader_activity.load(Ordering::Acquire) != 0 {
        context.push("backup output capture tasks remained active after cleanup".to_string());
    }
    context
}

fn append_output_cleanup_context(
    label: &str,
    result: Option<Result<BoundedOutput, String>>,
    context: &mut Vec<String>,
) {
    match result {
        Some(Ok(output)) => {
            let output = output.render();
            if !output.is_empty() {
                context.push(format!("backup {label}: {output}"));
            }
        }
        Some(Err(error)) => context.push(error),
        None => {}
    }
}

fn child_already_exited_error(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::InvalidInput {
        return true;
    }
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn with_cleanup_context(primary: String, context: Vec<String>) -> String {
    if context.is_empty() {
        primary
    } else {
        format!("{primary}; cleanup: {}", context.join("; "))
    }
}

fn duration_label(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}

struct BoundedChildOutput {
    status: ExitStatus,
    stdout: BoundedOutput,
    stderr: BoundedOutput,
}

#[cfg(test)]
mod lifecycle_tests {
    use std::time::Duration;

    use super::{duration_label, strict_timeout_at, with_cleanup_context};

    #[tokio::test]
    async fn strict_deadline_rejects_completion_observed_after_scheduler_delay() {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
        let result = strict_timeout_at(deadline, async {
            std::thread::sleep(Duration::from_millis(60));
            42
        })
        .await;
        assert_eq!(result, Err(()));
    }

    #[test]
    fn cleanup_context_and_duration_rendering_are_stable() {
        assert_eq!(
            with_cleanup_context(
                "primary failure".to_string(),
                vec!["first cleanup".to_string(), "second cleanup".to_string()],
            ),
            "primary failure; cleanup: first cleanup; second cleanup"
        );
        assert_eq!(duration_label(Duration::from_secs(120)), "120s");
        assert_eq!(duration_label(Duration::from_millis(250)), "250ms");
    }

    #[test]
    #[cfg(unix)]
    fn missing_child_pid_setup_error_is_stable() {
        let error = match super::ProcessGroupOwnership::new(None) {
            Ok(_) => panic!("missing child PID should fail setup"),
            Err(error) => error,
        };
        assert_eq!(error, "backup child PID was unavailable");
    }

    #[test]
    #[cfg(unix)]
    fn process_group_release_is_permanent_after_esrch() {
        use super::ProcessGroupOwnership;

        let mut ownership = ProcessGroupOwnership {
            leader_pid: i32::MAX,
            process_group: Some(i32::MAX),
        };
        let mut context = Vec::new();
        ownership.terminate_owned(&mut context);
        assert!(context.is_empty());
        assert!(ownership.process_group.is_none());

        ownership.terminate_owned(&mut context);
        assert!(context.is_empty());
        assert!(ownership.process_group.is_none());
    }

    #[tokio::test]
    #[cfg(target_os = "macos")]
    async fn leader_reap_is_refused_until_owned_group_is_inspected_and_released() {
        use std::os::unix::process::CommandExt as _;

        use super::{ProcessGroupOwnership, reap_leader_after_group_release};

        let mut command = tokio::process::Command::new("bash");
        command.arg("-c").arg("exit 0").kill_on_drop(true);
        command.as_std_mut().process_group(0);
        let mut child = command.spawn().expect("test child should spawn");
        let mut ownership =
            ProcessGroupOwnership::new(child.id()).expect("test child should have an owned PGID");
        tokio::time::timeout(
            Duration::from_secs(1),
            ownership.wait_for_leader_exit_without_reaping(),
        )
        .await
        .expect("test child exit should be observed")
        .expect("test child exit observation should succeed");

        let error = reap_leader_after_group_release(&mut child, &mut ownership)
            .await
            .expect_err("reap must be rejected while the PGID is owned");
        assert!(error.contains("refusing to reap backup leader"));
        assert!(
            child.id().is_some(),
            "rejected reap must preserve leader identity"
        );

        assert!(
            !ownership
                .inspect_and_release_before_reap()
                .expect("leader-only group inspection should succeed")
        );
        assert!(ownership.process_group.is_none());
        let status = reap_leader_after_group_release(&mut child, &mut ownership)
            .await
            .expect("released leader should reap");
        assert!(status.success());
        assert!(child.id().is_none());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn linux_stat_parser_handles_spaces_and_closing_parens_in_process_name() {
        assert_eq!(
            super::linux_process_group_from_stat("123 (backup ) worker) S 7 42 9 0"),
            Ok(42)
        );
        assert!(super::linux_process_group_from_stat("malformed").is_err());
    }
}
