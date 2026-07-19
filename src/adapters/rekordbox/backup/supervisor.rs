use std::ffi::{OsStr, OsString};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::error::{BackupError, BackupErrorKind, CleanupEntry, CleanupReport};
use super::output::{BoundedOutput, OutputReaderTask};
use super::process_group::{ProcessGroupOwnership, reap_leader_after_group_release};
use super::script::PreparedScript;

const PRE_OP_BACKUP_TIMEOUT: Duration = Duration::from_secs(120);
const CHILD_TERMINATION_TIMEOUT: Duration = Duration::from_secs(1);
const OUTPUT_READER_CLEANUP_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(test)]
static PRE_OP_BACKUP_TIMEOUT_OVERRIDE_MILLIS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Backup execution outcome, reported in write_xml response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupStatus {
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
/// 1. non-empty `REKLAWDBOX_BACKUP_SCRIPT`;
/// 2. the embedded script, except for the established test-build shortcut.
pub(crate) async fn run_pre_op_backup(db_path: &Path) -> Result<BackupStatus, String> {
    run_pre_op_backup_with_timeout(db_path, effective_pre_op_backup_timeout())
        .await
        .map_err(|error| error.to_string())
}

/// Execute backup using the embedded script with captured stdout and stderr.
#[allow(dead_code)]
pub(crate) async fn execute_embedded_with_env(
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
) -> Result<(), String> {
    let script = PreparedScript::embedded().map_err(|error| error.to_string())?;
    execute_prepared_script(script, args, env_additions, PRE_OP_BACKUP_TIMEOUT)
        .await
        .map_err(|error| error.to_string())
}

async fn run_pre_op_backup_with_timeout(
    db_path: &Path,
    timeout: Duration,
) -> Result<BackupStatus, BackupError> {
    let path_env = [("REKORDBOX_DB_PATH", db_path.as_os_str())];
    if let Some(script) = PreparedScript::configured_custom()? {
        tracing::info!("Running custom pre-op backup: {}", script.path().display());
        return execute_prepared_script(script, &["--pre-op"], &path_env, timeout)
            .await
            .map(|()| BackupStatus::Success);
    }

    // In test builds, don't auto-run the embedded script — tests use the
    // custom-script environment variable to exercise backup explicitly.
    if cfg!(test) {
        return Ok(BackupStatus::Success);
    }

    tracing::info!("Running embedded pre-op backup...");
    execute_prepared_script(
        PreparedScript::embedded()?,
        &["--pre-op"],
        &path_env,
        timeout,
    )
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

#[cfg(test)]
async fn execute_script_with_timeout(
    script_path: &Path,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
) -> Result<(), BackupError> {
    execute_script_with_timeout_and_activity(
        script_path,
        args,
        env_additions,
        timeout,
        Arc::new(AtomicUsize::new(0)),
    )
    .await
}

#[cfg(test)]
async fn execute_script_with_timeout_and_activity(
    script_path: &Path,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
) -> Result<(), BackupError> {
    execute_request(CapturedCommandRequest::new(
        PreparedScript::borrowed(script_path.to_path_buf()),
        args,
        env_additions,
        timeout,
        reader_activity,
    ))
    .await
}

async fn execute_prepared_script(
    script: PreparedScript,
    args: &[&str],
    env_additions: &[(&str, &OsStr)],
    timeout: Duration,
) -> Result<(), BackupError> {
    execute_request(CapturedCommandRequest::new(
        script,
        args,
        env_additions,
        timeout,
        Arc::new(AtomicUsize::new(0)),
    ))
    .await
}

async fn execute_request(request: CapturedCommandRequest) -> Result<(), BackupError> {
    DetachedCleanupTask::spawn(request).join().await
}

#[cfg(test)]
pub(crate) async fn execute_script_with_timeout_for_test(
    script_path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    execute_script_with_timeout(script_path, &["--pre-op"], &[], timeout)
        .await
        .map_err(|error| error.to_string())
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
    .map_err(|error| error.to_string())
}

#[cfg(test)]
pub(super) async fn execute_prepared_script_with_activity_for_test(
    script: PreparedScript,
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
) -> Result<(), String> {
    execute_request(CapturedCommandRequest::new(
        script,
        &["--pre-op"],
        &[],
        timeout,
        reader_activity,
    ))
    .await
    .map_err(|error| error.to_string())
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SupervisorFaultKind {
    StdoutCaptureUnavailable,
    StderrCaptureUnavailable,
    ProcessGroupInspection,
}

#[cfg(test)]
struct SupervisorFault {
    kind: SupervisorFaultKind,
    ready: PathBuf,
    readers_observed: Arc<AtomicBool>,
}

#[cfg(test)]
pub(super) async fn execute_script_with_supervisor_fault_for_test(
    script_path: &Path,
    kind: SupervisorFaultKind,
    ready: PathBuf,
    reader_activity: Arc<AtomicUsize>,
    readers_observed: Arc<AtomicBool>,
) -> Result<(), String> {
    let mut request = CapturedCommandRequest::new(
        PreparedScript::borrowed(script_path.to_path_buf()),
        &["--pre-op"],
        &[],
        Duration::from_secs(5),
        reader_activity,
    );
    request.fault = Some(SupervisorFault {
        kind,
        ready,
        readers_observed,
    });
    execute_request(request)
        .await
        .map_err(|error| error.to_string())
}

struct CapturedCommandRequest {
    script: PreparedScript,
    args: Vec<String>,
    env_additions: Vec<(OsString, OsString)>,
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
    #[cfg(test)]
    fault: Option<SupervisorFault>,
}

impl CapturedCommandRequest {
    fn new(
        script: PreparedScript,
        args: &[&str],
        env_additions: &[(&str, &OsStr)],
        timeout: Duration,
        reader_activity: Arc<AtomicUsize>,
    ) -> Self {
        Self {
            script,
            args: args
                .iter()
                .map(|argument| (*argument).to_string())
                .collect(),
            env_additions: env_additions
                .iter()
                .map(|(key, value)| (OsString::from(key), (*value).to_os_string()))
                .collect(),
            timeout,
            reader_activity,
            #[cfg(test)]
            fault: None,
        }
    }
}

/// Owns the detached supervisor join handle.
///
/// Dropping an awaiting caller deliberately drops (and therefore detaches)
/// the handle without aborting it. The supervisor retains every cleanup
/// resource and continues to its bounded terminal path.
struct DetachedCleanupTask {
    handle: Option<tokio::task::JoinHandle<Result<(), BackupError>>>,
}

impl DetachedCleanupTask {
    fn spawn(request: CapturedCommandRequest) -> Self {
        let handle = tokio::spawn(async move {
            let supervisor = BackupSupervisor::spawn(request).await?;
            supervisor.run().await
        });
        Self {
            handle: Some(handle),
        }
    }

    async fn join(mut self) -> Result<(), BackupError> {
        self.handle
            .take()
            .expect("detached backup supervisor should be joined once")
            .await
            .map_err(|error| BackupError::new(BackupErrorKind::SupervisorTask(error.to_string())))?
    }
}

impl Drop for DetachedCleanupTask {
    fn drop(&mut self) {
        // Intentionally do not abort: dropping JoinHandle detaches the task.
    }
}

struct BackupSupervisor {
    child: tokio::process::Child,
    process_group: ProcessGroupOwnership,
    stdout: OutputReaderTask,
    stderr: OutputReaderTask,
    deadline: tokio::time::Instant,
    timeout: Duration,
    reader_activity: Arc<AtomicUsize>,
    _script: PreparedScript,
    #[cfg(test)]
    fault: Option<SupervisorFault>,
}

impl BackupSupervisor {
    async fn spawn(request: CapturedCommandRequest) -> Result<Self, BackupError> {
        let mut command = tokio::process::Command::new("bash");
        command.arg(request.script.path()).args(&request.args);
        for (key, value) in &request.env_additions {
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
            .map_err(|error| BackupError::new(BackupErrorKind::Launch(error.to_string())))?;
        let mut process_group = match ProcessGroupOwnership::new(child.id()) {
            Ok(process_group) => process_group,
            Err(error) => {
                return Err(
                    cleanup_spawned_child_after_setup_failure(&mut child, None, error).await,
                );
            }
        };
        #[cfg(test)]
        if request
            .fault
            .as_ref()
            .is_some_and(|fault| fault.kind == SupervisorFaultKind::StdoutCaptureUnavailable)
        {
            wait_for_supervisor_fault_ready(
                &request
                    .fault
                    .as_ref()
                    .expect("stdout fault should exist")
                    .ready,
            )
            .await;
            return Err(cleanup_spawned_child_after_setup_failure(
                &mut child,
                Some(&mut process_group),
                BackupError::new(BackupErrorKind::OutputCaptureSetup("stdout")),
            )
            .await);
        }
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                return Err(cleanup_spawned_child_after_setup_failure(
                    &mut child,
                    Some(&mut process_group),
                    BackupError::new(BackupErrorKind::OutputCaptureSetup("stdout")),
                )
                .await);
            }
        };
        #[cfg(test)]
        if request
            .fault
            .as_ref()
            .is_some_and(|fault| fault.kind == SupervisorFaultKind::StderrCaptureUnavailable)
        {
            wait_for_supervisor_fault_ready(
                &request
                    .fault
                    .as_ref()
                    .expect("stderr fault should exist")
                    .ready,
            )
            .await;
            return Err(cleanup_spawned_child_after_setup_failure(
                &mut child,
                Some(&mut process_group),
                BackupError::new(BackupErrorKind::OutputCaptureSetup("stderr")),
            )
            .await);
        }
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                return Err(cleanup_spawned_child_after_setup_failure(
                    &mut child,
                    Some(&mut process_group),
                    BackupError::new(BackupErrorKind::OutputCaptureSetup("stderr")),
                )
                .await);
            }
        };
        let stdout =
            OutputReaderTask::spawn("stdout", stdout, Arc::clone(&request.reader_activity));
        let stderr =
            OutputReaderTask::spawn("stderr", stderr, Arc::clone(&request.reader_activity));
        let deadline = tokio::time::Instant::now() + request.timeout;

        Ok(Self {
            child,
            process_group,
            stdout,
            stderr,
            deadline,
            timeout: request.timeout,
            reader_activity: request.reader_activity,
            _script: request.script,
            #[cfg(test)]
            fault: request.fault,
        })
    }

    async fn run(mut self) -> Result<(), BackupError> {
        let output = match strict_timeout_at(self.deadline, self.finish()).await {
            Ok(Ok(output)) => output,
            Ok(Err(error)) => return Err(error.with_cleanup(self.terminate().await)),
            Err(()) => {
                return Err(
                    BackupError::new(BackupErrorKind::DeadlineExceeded(self.timeout))
                        .with_cleanup(self.terminate().await),
                );
            }
        };
        debug_assert_eq!(self.reader_activity.load(Ordering::Acquire), 0);

        if output.status.success() {
            tracing::info!("Backup completed.");
            Ok(())
        } else {
            let stderr = output.stderr.render();
            let stdout = output.stdout.render();
            let details = if !stderr.is_empty() {
                stderr
            } else if !stdout.is_empty() {
                stdout
            } else {
                "backup script exited without output".to_string()
            };
            Err(BackupError::new(BackupErrorKind::NonZeroExit {
                status: output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_string(), |code| code.to_string()),
                details,
            }))
        }
    }

    async fn finish(&mut self) -> Result<BoundedChildOutput, BackupError> {
        #[cfg(test)]
        if let Some(fault) = self
            .fault
            .as_ref()
            .filter(|fault| fault.kind == SupervisorFaultKind::ProcessGroupInspection)
        {
            wait_for_supervisor_fault_ready(&fault.ready).await;
            let observed = tokio::time::timeout(Duration::from_secs(1), async {
                while self.reader_activity.load(Ordering::Acquire) != 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_ok();
            fault.readers_observed.store(observed, Ordering::Release);
            let mut cleanup = CleanupReport::default();
            cleanup.push(CleanupEntry::ProcessGroup(
                "process-group termination failed: fixture".to_string(),
            ));
            return Err(BackupError::new(BackupErrorKind::ProcessGroup(
                "backup process-group inspection failed: fixture".to_string(),
            ))
            .with_cleanup(cleanup));
        }
        #[cfg(unix)]
        let status = {
            self.process_group
                .wait_for_leader_exit_without_reaping()
                .await?;
            let descendants = self.process_group.inspect_and_release_before_reap()?;
            let status =
                reap_leader_after_group_release(&mut self.child, &mut self.process_group).await?;
            if descendants {
                return Err(BackupError::new(BackupErrorKind::DescendantProcessDetected));
            }
            status
        };
        #[cfg(not(unix))]
        let status = self
            .child
            .wait()
            .await
            .map_err(|error| BackupError::new(BackupErrorKind::Wait(error.to_string())))?;
        let (stdout, stderr) = tokio::try_join!(self.stdout.finish(), self.stderr.finish())?;
        Ok(BoundedChildOutput {
            status,
            stdout,
            stderr,
        })
    }

    async fn terminate(&mut self) -> CleanupReport {
        let mut report = CleanupReport::default();
        self.process_group.terminate_owned(&mut report);
        if let Err(error) = self.child.start_kill()
            && !child_already_exited_error(&error)
        {
            report.push(CleanupEntry::DirectChildTermination(error.to_string()));
        }
        match tokio::time::timeout(
            CHILD_TERMINATION_TIMEOUT,
            reap_leader_after_group_release(&mut self.child, &mut self.process_group),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => report.push(CleanupEntry::DirectChildReap(error.to_string())),
            Err(_) => report.push(CleanupEntry::DirectChildTimeout(CHILD_TERMINATION_TIMEOUT)),
        }

        self.stdout.abort_if_running();
        self.stderr.abort_if_running();
        let reader_results = tokio::time::timeout(OUTPUT_READER_CLEANUP_TIMEOUT, async {
            tokio::join!(
                self.stdout.finish_after_abort(),
                self.stderr.finish_after_abort()
            )
        })
        .await;
        match reader_results {
            Ok((stdout, stderr)) => {
                append_output_cleanup("stdout", stdout, &mut report);
                append_output_cleanup("stderr", stderr, &mut report);
            }
            Err(_) => report.push(CleanupEntry::OutputTaskTimeout(
                OUTPUT_READER_CLEANUP_TIMEOUT,
            )),
        }
        if self.reader_activity.load(Ordering::Acquire) != 0 {
            report.push(CleanupEntry::OutputTasksStillActive);
        }
        report
    }
}

#[cfg(test)]
async fn wait_for_supervisor_fault_ready(path: &Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "supervisor fault ready marker did not appear: {}",
            path.display()
        )
    });
}

impl Drop for BackupSupervisor {
    fn drop(&mut self) {
        let mut report = CleanupReport::default();
        self.process_group.terminate_owned(&mut report);
        if let Err(error) = self.child.start_kill()
            && !child_already_exited_error(&error)
        {
            tracing::warn!("Backup direct-child cleanup on drop failed: {error}");
        }
        self.stdout.abort_if_running();
        self.stderr.abort_if_running();
        for entry in report.iter() {
            tracing::warn!("Backup supervisor emergency cleanup: {entry}");
        }
    }
}

async fn cleanup_spawned_child_after_setup_failure(
    child: &mut tokio::process::Child,
    mut process_group: Option<&mut ProcessGroupOwnership>,
    error: BackupError,
) -> BackupError {
    let mut report = CleanupReport::default();
    if let Some(process_group) = process_group.as_deref_mut() {
        process_group.terminate_owned(&mut report);
    }
    if let Err(error) = child.start_kill()
        && !child_already_exited_error(&error)
    {
        report.push(CleanupEntry::DirectChildTermination(error.to_string()));
    }
    let reap = async {
        if let Some(process_group) = process_group {
            reap_leader_after_group_release(child, process_group).await
        } else {
            child
                .wait()
                .await
                .map_err(|error| BackupError::new(BackupErrorKind::Wait(error.to_string())))
        }
    };
    match tokio::time::timeout(CHILD_TERMINATION_TIMEOUT, reap).await {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => report.push(CleanupEntry::DirectChildReap(error.to_string())),
        Err(_) => report.push(CleanupEntry::DirectChildTimeout(CHILD_TERMINATION_TIMEOUT)),
    }
    error.with_cleanup(report)
}

fn append_output_cleanup(
    label: &'static str,
    result: Option<Result<BoundedOutput, BackupError>>,
    report: &mut CleanupReport,
) {
    match result {
        Some(Ok(output)) => {
            let output = output.render();
            if !output.is_empty() {
                report.push(CleanupEntry::CapturedOutput { label, output });
            }
        }
        Some(Err(error)) => report.push(CleanupEntry::OutputError(error.to_string())),
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

struct BoundedChildOutput {
    status: ExitStatus,
    stdout: BoundedOutput,
    stderr: BoundedOutput,
}

#[cfg(test)]
pub(super) async fn strict_timeout_at_for_test<F, T>(
    deadline: tokio::time::Instant,
    future: F,
) -> Result<T, ()>
where
    F: std::future::Future<Output = T>,
{
    strict_timeout_at(deadline, future).await
}
