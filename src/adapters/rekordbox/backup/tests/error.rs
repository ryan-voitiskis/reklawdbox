use std::time::Duration;

use super::super::error::{BackupError, BackupErrorKind, CleanupEntry, CleanupReport};

#[test]
fn cleanup_context_and_duration_rendering_are_stable() {
    let mut cleanup = CleanupReport::default();
    cleanup.push(CleanupEntry::ProcessGroup("first cleanup".to_string()));
    cleanup.push(CleanupEntry::OutputError("second cleanup".to_string()));
    let error = BackupError::new(BackupErrorKind::ProcessGroup("primary failure".to_string()))
        .with_cleanup(cleanup);
    assert_eq!(
        error.to_string(),
        "primary failure; cleanup: first cleanup; second cleanup"
    );
    assert_eq!(
        BackupError::new(BackupErrorKind::DeadlineExceeded(Duration::from_secs(120))).to_string(),
        "pre-operation backup timed out after 120s"
    );
    assert_eq!(
        BackupError::new(BackupErrorKind::DeadlineExceeded(Duration::from_millis(
            250
        )))
        .to_string(),
        "pre-operation backup timed out after 250ms"
    );
}

#[test]
fn cleanup_entry_order_and_messages_are_stable() {
    let mut cleanup = CleanupReport::default();
    cleanup.push(CleanupEntry::DirectChildTermination(
        "terminate".to_string(),
    ));
    cleanup.push(CleanupEntry::DirectChildReap("reap".to_string()));
    cleanup.push(CleanupEntry::DirectChildTimeout(Duration::from_secs(1)));
    cleanup.push(CleanupEntry::CapturedOutput {
        label: "stdout",
        output: "diagnostic".to_string(),
    });
    cleanup.push(CleanupEntry::OutputTaskTimeout(Duration::from_secs(1)));
    cleanup.push(CleanupEntry::OutputTasksStillActive);
    assert_eq!(
        cleanup.to_string(),
        "direct-child termination failed: terminate; direct-child reap failed: reap; direct child did not exit within 1s after termination; backup stdout: diagnostic; backup output capture tasks did not stop within 1s after cancellation; backup output capture tasks remained active after cleanup"
    );
}
