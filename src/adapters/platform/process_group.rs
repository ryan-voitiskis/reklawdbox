//! Synchronous ownership of one process-group identity.
//!
//! This module deliberately owns no child handle, command, executor, timeout,
//! polling loop, reader, or output policy. Callers must retain the unreaped
//! leader until [`ProcessGroupOwnership::is_released`] reports that guarded
//! reap is safe.

#[derive(Debug)]
pub(crate) struct ProcessGroupError {
    pub(crate) kind: ProcessGroupErrorKind,
    pub(crate) cleanup: Vec<ProcessGroupCleanupError>,
}

impl ProcessGroupError {
    fn new(kind: ProcessGroupErrorKind) -> Self {
        Self {
            kind,
            cleanup: Vec::new(),
        }
    }

    fn with_cleanup(kind: ProcessGroupErrorKind, cleanup: Vec<ProcessGroupCleanupError>) -> Self {
        Self { kind, cleanup }
    }

    #[cfg(test)]
    pub(crate) fn injected_inspection_failure() -> Self {
        Self::new(ProcessGroupErrorKind::ObservationFailed(
            std::io::Error::other("scripted process-group inspection failure"),
        ))
    }
}

#[derive(Debug)]
pub(crate) enum ProcessGroupErrorKind {
    LeaderPidUnavailable,
    LeaderPidConversion(String),
    ObservationPidConversion(String),
    UnexpectedObservedPid {
        observed: i32,
        expected: i32,
    },
    ObservationFailed(std::io::Error),
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "linux", target_os = "android"))
    ))]
    ObservationUnsupported(&'static str),
    ReleasedBeforeInspection,
    FreezeFailed(std::io::Error),
    DescendantCleanup(Vec<ProcessGroupCleanupError>),
    #[cfg(target_os = "macos")]
    MemberBufferTooLarge,
    #[cfg(target_os = "macos")]
    MemberInspection(std::io::Error),
    #[cfg(target_os = "macos")]
    InvalidMemberResult(String),
    #[cfg(target_os = "macos")]
    SnapshotOmittedLeader,
    #[cfg(target_os = "macos")]
    GroupTooLarge,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    ProcInspection(std::io::Error),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    ProcInspectionForPid {
        pid: i32,
        source: std::io::Error,
    },
    #[cfg(any(target_os = "linux", target_os = "android"))]
    ProcStatForPid {
        pid: i32,
        malformed: bool,
    },
    #[cfg(any(target_os = "linux", target_os = "android"))]
    MalformedStat,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    InvalidStat,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    LeaderSnapshot(std::io::Error),
    #[cfg(any(target_os = "linux", target_os = "android"))]
    LeaderMoved {
        expected: i32,
        observed: i32,
    },
    #[cfg(all(
        unix,
        not(any(target_os = "macos", target_os = "linux", target_os = "android"))
    ))]
    InspectionUnsupported(&'static str),
}

#[derive(Debug)]
pub(crate) enum ProcessGroupCleanupError {
    Termination(std::io::Error),
}

#[cfg(unix)]
pub(crate) struct ProcessGroupOwnership {
    leader_pid: i32,
    process_group: Option<i32>,
}

#[cfg(unix)]
impl ProcessGroupOwnership {
    pub(crate) fn new(child_id: Option<u32>) -> Result<Self, ProcessGroupError> {
        let child_id = child_id
            .ok_or_else(|| ProcessGroupError::new(ProcessGroupErrorKind::LeaderPidUnavailable))?;
        let process_group = i32::try_from(child_id).map_err(|error| {
            ProcessGroupError::new(ProcessGroupErrorKind::LeaderPidConversion(
                error.to_string(),
            ))
        })?;
        Ok(Self {
            leader_pid: process_group,
            process_group: Some(process_group),
        })
    }

    pub(crate) fn leader_exit_observed_without_reaping(&self) -> Result<bool, ProcessGroupError> {
        leader_exit_observed_without_reaping(self.leader_pid)
    }

    pub(crate) fn inspect_and_release_before_reap(&mut self) -> Result<bool, ProcessGroupError> {
        let Some(process_group) = self.process_group else {
            return Err(ProcessGroupError::new(
                ProcessGroupErrorKind::ReleasedBeforeInspection,
            ));
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
                let cleanup = self.terminate_owned();
                Err(ProcessGroupError::with_cleanup(
                    ProcessGroupErrorKind::FreezeFailed(stop_error),
                    cleanup,
                ))
            }
            Ok(true) => {
                let cleanup = self.terminate_owned();
                if cleanup.is_empty() {
                    Ok(true)
                } else {
                    Err(ProcessGroupError::new(
                        ProcessGroupErrorKind::DescendantCleanup(cleanup),
                    ))
                }
            }
            Err(error) => {
                let cleanup = self.terminate_owned();
                Err(ProcessGroupError::with_cleanup(error.kind, cleanup))
            }
        }
    }

    pub(crate) fn is_released(&self) -> bool {
        self.process_group.is_none()
    }

    fn relinquish_without_signal(&mut self) {
        self.process_group = None;
    }

    pub(crate) fn terminate_owned(&mut self) -> Vec<ProcessGroupCleanupError> {
        let mut cleanup = Vec::new();
        let Some(process_group) = self.process_group.take() else {
            return cleanup;
        };
        if let Err(error) = signal_process_group(process_group, libc::SIGKILL)
            && error.raw_os_error() != Some(libc::ESRCH)
        {
            cleanup.push(ProcessGroupCleanupError::Termination(error));
        }
        cleanup
    }

    #[cfg(test)]
    fn owned_for_test(leader_pid: i32) -> Self {
        Self {
            leader_pid,
            process_group: Some(leader_pid),
        }
    }
}

#[cfg(unix)]
fn signal_process_group(process_group: i32, signal: i32) -> std::io::Result<()> {
    // SAFETY: a negative PID targets the owned process group. Ownership is
    // retained until this signal completes or the group is known absent.
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
        for error in self.terminate_owned() {
            tracing::warn!("Process-group cleanup on drop: {error:?}");
        }
    }
}

#[cfg(not(unix))]
pub(crate) struct ProcessGroupOwnership;

#[cfg(not(unix))]
impl ProcessGroupOwnership {
    pub(crate) fn new(_child_id: Option<u32>) -> Result<Self, ProcessGroupError> {
        Ok(Self)
    }

    pub(crate) fn is_released(&self) -> bool {
        true
    }

    pub(crate) fn terminate_owned(&mut self) -> Vec<ProcessGroupCleanupError> {
        Vec::new()
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn leader_exit_observed_without_reaping(leader_pid: i32) -> Result<bool, ProcessGroupError> {
    let leader_id = libc::id_t::try_from(leader_pid).map_err(|error| {
        ProcessGroupError::new(ProcessGroupErrorKind::ObservationPidConversion(
            error.to_string(),
        ))
    })?;
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        // SAFETY: `info` points to writable storage for one siginfo_t and
        // WNOWAIT deliberately observes without releasing the leader PID.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                leader_id,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            // SAFETY: waitid initialized the siginfo_t on success.
            let info = unsafe { info.assume_init() };
            // SAFETY: libc exposes si_pid through the platform accessor.
            let observed_pid = unsafe { info.si_pid() };
            if observed_pid == 0 {
                return Ok(false);
            }
            if observed_pid != leader_pid {
                return Err(ProcessGroupError::new(
                    ProcessGroupErrorKind::UnexpectedObservedPid {
                        observed: observed_pid,
                        expected: leader_pid,
                    },
                ));
            }
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(ProcessGroupError::new(
                ProcessGroupErrorKind::ObservationFailed(error),
            ));
        }
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
fn leader_exit_observed_without_reaping(_leader_pid: i32) -> Result<bool, ProcessGroupError> {
    Err(ProcessGroupError::new(
        ProcessGroupErrorKind::ObservationUnsupported(std::env::consts::OS),
    ))
}

#[cfg(target_os = "macos")]
fn process_group_has_other_members(
    process_group: i32,
    leader_pid: i32,
) -> Result<bool, ProcessGroupError> {
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
            .ok_or_else(|| ProcessGroupError::new(ProcessGroupErrorKind::MemberBufferTooLarge))?;
        // SAFETY: the allocated PID buffer is valid for `buffer_size` bytes.
        let pid_count = unsafe {
            proc_listpgrppids(
                process_group,
                pids.as_mut_ptr().cast::<libc::c_void>(),
                buffer_size,
            )
        };
        if pid_count < 0 {
            return Err(ProcessGroupError::new(
                ProcessGroupErrorKind::MemberInspection(std::io::Error::last_os_error()),
            ));
        }
        let pid_count = usize::try_from(pid_count).map_err(|error| {
            ProcessGroupError::new(ProcessGroupErrorKind::InvalidMemberResult(
                error.to_string(),
            ))
        })?;
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
                return Err(ProcessGroupError::new(
                    ProcessGroupErrorKind::SnapshotOmittedLeader,
                ));
            }
            return Ok(false);
        }
        capacity = capacity
            .checked_mul(2)
            .filter(|capacity| *capacity <= 65_536)
            .ok_or_else(|| ProcessGroupError::new(ProcessGroupErrorKind::GroupTooLarge))?;
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_group_has_other_members(
    process_group: i32,
    leader_pid: i32,
) -> Result<bool, ProcessGroupError> {
    for entry in std::fs::read_dir("/proc")
        .map_err(|error| ProcessGroupError::new(ProcessGroupErrorKind::ProcInspection(error)))?
    {
        let entry = entry.map_err(|error| {
            ProcessGroupError::new(ProcessGroupErrorKind::ProcInspection(error))
        })?;
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
                return Err(ProcessGroupError::new(
                    ProcessGroupErrorKind::ProcInspectionForPid { pid, source: error },
                ));
            }
        };
        let observed_group = linux_process_group_from_stat(&stat).map_err(|error| {
            ProcessGroupError::new(match error.kind {
                ProcessGroupErrorKind::MalformedStat => ProcessGroupErrorKind::ProcStatForPid {
                    pid,
                    malformed: true,
                },
                ProcessGroupErrorKind::InvalidStat => ProcessGroupErrorKind::ProcStatForPid {
                    pid,
                    malformed: false,
                },
                kind => kind,
            })
        })?;
        if pid != leader_pid && observed_group == process_group {
            return Ok(true);
        }
    }
    // The leader must remain visible as a zombie until the guarded reap. Its
    // presence proves the procfs scan still refers to the owned PGID identity.
    let leader_stat = std::fs::read_to_string(format!("/proc/{leader_pid}/stat"))
        .map_err(|error| ProcessGroupError::new(ProcessGroupErrorKind::LeaderSnapshot(error)))?;
    let leader_group = linux_process_group_from_stat(&leader_stat)?;
    if leader_group != process_group {
        return Err(ProcessGroupError::new(ProcessGroupErrorKind::LeaderMoved {
            expected: process_group,
            observed: leader_group,
        }));
    }
    Ok(false)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn linux_process_group_from_stat(stat: &str) -> Result<i32, ProcessGroupError> {
    let fields = stat
        .rsplit_once(") ")
        .map(|(_, fields)| fields)
        .ok_or_else(|| ProcessGroupError::new(ProcessGroupErrorKind::MalformedStat))?;
    fields
        .split_whitespace()
        .nth(2)
        .and_then(|field| field.parse::<i32>().ok())
        .ok_or_else(|| ProcessGroupError::new(ProcessGroupErrorKind::InvalidStat))
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
fn process_group_has_other_members(
    _process_group: i32,
    _leader_pid: i32,
) -> Result<bool, ProcessGroupError> {
    Err(ProcessGroupError::new(
        ProcessGroupErrorKind::InspectionUnsupported(std::env::consts::OS),
    ))
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(unix)]
    fn process_group_release_is_permanent_after_esrch() {
        let mut ownership = super::ProcessGroupOwnership::owned_for_test(i32::MAX);
        let cleanup = ownership.terminate_owned();
        assert!(cleanup.is_empty());
        assert!(ownership.is_released());

        let cleanup = ownership.terminate_owned();
        assert!(cleanup.is_empty());
        assert!(ownership.is_released());
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn linux_stat_parser_handles_spaces_and_closing_parens_in_process_name() {
        assert_eq!(
            super::linux_process_group_from_stat("123 (backup ) worker) S 7 42 9 0")
                .expect("valid stat should parse"),
            42
        );
        assert!(super::linux_process_group_from_stat("malformed").is_err());
    }
}
