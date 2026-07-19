use std::time::Duration;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::super::script::PreparedScript;
use super::super::supervisor::{
    SupervisorFaultKind, execute_prepared_script_with_activity_for_test,
    execute_script_with_supervisor_fault_for_test, strict_timeout_at_for_test,
};

#[tokio::test]
async fn strict_deadline_rejects_completion_observed_after_scheduler_delay() {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(20);
    let result = strict_timeout_at_for_test(deadline, async {
        std::thread::sleep(Duration::from_millis(60));
        42
    })
    .await;
    assert_eq!(result, Err(()));
}

#[tokio::test]
#[cfg(unix)]
async fn detached_supervisor_retains_temporary_script_owner_until_cleanup() {
    let temp_dir = tempfile::tempdir().expect("owned script directory should create");
    let root = temp_dir.path().to_path_buf();
    let script_path = root.join("owned-hang.sh");
    let ready = root.join("ready");
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ntouch '{}'\nwhile :; do sleep 60; done\n",
            ready.display()
        ),
    )
    .expect("owned script should write");
    let script = PreparedScript::owned_for_test(script_path, temp_dir);
    let activity = Arc::new(AtomicUsize::new(0));
    let task_activity = Arc::clone(&activity);
    let caller = tokio::spawn(async move {
        execute_prepared_script_with_activity_for_test(
            script,
            Duration::from_millis(250),
            task_activity,
        )
        .await
    });

    tokio::time::timeout(Duration::from_secs(2), async {
        while !ready.exists() || activity.load(Ordering::Acquire) != 2 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("owned script and both readers should become active");

    caller.abort();
    assert!(
        caller
            .await
            .expect_err("cancelled caller should not complete")
            .is_cancelled()
    );
    assert!(
        root.exists(),
        "detached supervisor must retain its temporary script owner"
    );

    tokio::time::timeout(Duration::from_secs(5), async {
        while root.exists() || activity.load(Ordering::Acquire) != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("detached supervisor should clean its script root and readers");
}

#[tokio::test]
#[cfg(unix)]
async fn partial_setup_and_inspection_failures_clean_spawned_resources() {
    let cases = [
        (
            SupervisorFaultKind::StdoutCaptureUnavailable,
            "backup stdout capture was unavailable",
            false,
        ),
        (
            SupervisorFaultKind::StderrCaptureUnavailable,
            "backup stderr capture was unavailable",
            false,
        ),
        (
            SupervisorFaultKind::ProcessGroupInspection,
            "backup process-group inspection failed: fixture; cleanup: process-group termination failed: fixture",
            true,
        ),
    ];

    for (kind, expected, readers_should_start) in cases {
        let temp = tempfile::tempdir().expect("supervisor fault fixture should create");
        let parent_pid_path = temp.path().join("parent.pid");
        let descendant_pid_path = temp.path().join("descendant.pid");
        let ready = temp.path().join("ready");
        let script = temp.path().join("fault.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nsleep 60 &\nprintf '%s\\n' \"$!\" > '{}'\ntouch '{}'\nwait\n",
                parent_pid_path.display(),
                descendant_pid_path.display(),
                ready.display(),
            ),
        )
        .expect("supervisor fault script should write");
        let activity = Arc::new(AtomicUsize::new(0));
        let readers_observed = Arc::new(AtomicBool::new(false));
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            execute_script_with_supervisor_fault_for_test(
                &script,
                kind,
                ready,
                Arc::clone(&activity),
                Arc::clone(&readers_observed),
            ),
        )
        .await
        .expect("supervisor fault should finish within five seconds");
        assert_eq!(
            result.expect_err("injected supervisor fault should fail"),
            expected
        );

        let parent_pid = read_fixture_pid(&parent_pid_path);
        let descendant_pid = read_fixture_pid(&descendant_pid_path);
        wait_for_pid_exit(parent_pid).await;
        wait_for_pid_exit(descendant_pid).await;
        assert_eq!(activity.load(Ordering::Acquire), 0);
        assert_eq!(
            readers_observed.load(Ordering::Acquire),
            readers_should_start,
            "unexpected reader setup state for {kind:?}"
        );
    }
}

#[cfg(unix)]
fn read_fixture_pid(path: &std::path::Path) -> i32 {
    std::fs::read_to_string(path)
        .expect("fixture PID should be readable")
        .trim()
        .parse()
        .expect("fixture PID should parse")
}

#[cfg(unix)]
fn pid_exists(pid: i32) -> bool {
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
async fn wait_for_pid_exit(pid: i32) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while pid_exists(pid) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("fixture PID {pid} remained alive"));
}
