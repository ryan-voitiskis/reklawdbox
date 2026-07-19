use std::process::ExitStatus;
use std::time::Duration;

use crate::adapters::platform::process_group as platform;

const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(5);

pub(super) struct ProcessGroupOwnership {
    owned: platform::ProcessGroupOwnership,
}

impl ProcessGroupOwnership {
    pub(super) fn new(child_id: Option<u32>) -> Result<Self, String> {
        platform::ProcessGroupOwnership::new(child_id)
            .map(|owned| Self { owned })
            .map_err(render_process_group_error)
    }

    #[cfg(unix)]
    pub(super) async fn wait_for_leader_exit_without_reaping(&self) -> Result<(), String> {
        loop {
            if self
                .owned
                .leader_exit_observed_without_reaping()
                .map_err(render_process_group_error)?
            {
                return Ok(());
            }
            tokio::time::sleep(CHILD_EXIT_POLL_INTERVAL).await;
        }
    }

    #[cfg(unix)]
    pub(super) fn inspect_and_release_before_reap(&mut self) -> Result<bool, String> {
        self.owned
            .inspect_and_release_before_reap()
            .map_err(render_process_group_error)
    }

    pub(super) fn terminate_owned(&mut self, context: &mut Vec<String>) {
        context.extend(
            self.owned
                .terminate_owned()
                .into_iter()
                .map(render_cleanup_error),
        );
    }

    #[cfg(test)]
    pub(super) fn is_released(&self) -> bool {
        self.owned.is_released()
    }
}

fn render_process_group_error(error: platform::ProcessGroupError) -> String {
    use platform::ProcessGroupErrorKind as Kind;

    let primary = match error.kind {
        Kind::LeaderPidUnavailable => "backup child PID was unavailable".to_string(),
        Kind::LeaderPidConversion(error) => format!("child PID conversion failed: {error}"),
        Kind::ObservationPidConversion(error) => {
            format!("backup child PID conversion failed: {error}")
        }
        Kind::UnexpectedObservedPid { observed, expected } => {
            format!("backup exit observation returned PID {observed}, expected {expected}")
        }
        Kind::ObservationFailed(error) => {
            format!("backup exit observation failed before leader reap: {error}")
        }
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "linux", target_os = "android"))
        ))]
        Kind::ObservationUnsupported(os) => {
            format!("safe pre-reap backup exit observation is unsupported on {os}")
        }
        Kind::ReleasedBeforeInspection => {
            "backup process group was released before inspection".to_string()
        }
        Kind::FreezeFailed(error) => format!("backup process-group freeze failed: {error}"),
        Kind::DescendantCleanup(cleanup) => format!(
            "backup process-group cleanup failed: {}",
            cleanup
                .into_iter()
                .map(render_cleanup_error)
                .collect::<Vec<_>>()
                .join("; ")
        ),
        #[cfg(target_os = "macos")]
        Kind::MemberBufferTooLarge => {
            "backup process-group member buffer was too large".to_string()
        }
        #[cfg(target_os = "macos")]
        Kind::MemberInspection(error) => {
            format!("backup process-group member inspection failed: {error}")
        }
        #[cfg(target_os = "macos")]
        Kind::InvalidMemberResult(error) => {
            format!("backup process-group result was invalid: {error}")
        }
        #[cfg(target_os = "macos")]
        Kind::SnapshotOmittedLeader => {
            "backup process-group snapshot omitted the unreaped leader".to_string()
        }
        #[cfg(target_os = "macos")]
        Kind::GroupTooLarge => "backup process group was unexpectedly large".to_string(),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::ProcInspection(error) => {
            format!("backup process-group inspection failed: {error}")
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::ProcInspectionForPid { pid, source } => {
            format!("backup process-group inspection failed for PID {pid}: {source}")
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::ProcStatForPid { pid, malformed } => {
            let detail = if malformed {
                "malformed stat"
            } else {
                "invalid stat"
            };
            format!("backup process-group inspection returned {detail} for PID {pid}")
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::MalformedStat => {
            "backup process-group inspection returned malformed stat".to_string()
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::InvalidStat => "backup process-group inspection returned invalid stat".to_string(),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::LeaderSnapshot(error) => {
            format!("backup process-group snapshot omitted the leader: {error}")
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Kind::LeaderMoved { expected, observed } => {
            format!("backup leader moved from process group {expected} to {observed}")
        }
        #[cfg(all(
            unix,
            not(any(target_os = "macos", target_os = "linux", target_os = "android"))
        ))]
        Kind::InspectionUnsupported(os) => {
            format!("safe backup process-group inspection is unsupported on {os}")
        }
    };
    if error.cleanup.is_empty() {
        primary
    } else {
        format!(
            "{primary}; cleanup: {}",
            error
                .cleanup
                .into_iter()
                .map(render_cleanup_error)
                .collect::<Vec<_>>()
                .join("; ")
        )
    }
}

fn render_cleanup_error(error: platform::ProcessGroupCleanupError) -> String {
    match error {
        platform::ProcessGroupCleanupError::Termination(error) => {
            format!("process-group termination failed: {error}")
        }
    }
}

#[cfg(unix)]
pub(super) async fn reap_leader_after_group_release(
    child: &mut tokio::process::Child,
    process_group: &mut ProcessGroupOwnership,
) -> Result<ExitStatus, String> {
    if !process_group.owned.is_released() {
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
pub(super) async fn reap_leader_after_group_release(
    child: &mut tokio::process::Child,
    _process_group: &mut ProcessGroupOwnership,
) -> Result<ExitStatus, String> {
    child
        .wait()
        .await
        .map_err(|error| format!("backup wait failed: {error}"))
}
