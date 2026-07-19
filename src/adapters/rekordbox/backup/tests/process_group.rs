use std::time::Duration;

use super::super::process_group::{ProcessGroupOwnership, reap_leader_after_group_release};

#[test]
#[cfg(unix)]
fn missing_child_pid_setup_error_is_stable() {
    let error = match ProcessGroupOwnership::new(None) {
        Ok(_) => panic!("missing child PID should fail setup"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "backup child PID was unavailable");
}

#[tokio::test]
#[cfg(target_os = "macos")]
async fn leader_reap_is_refused_until_owned_group_is_inspected_and_released() {
    use std::os::unix::process::CommandExt as _;

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
    assert!(error.to_string().contains("refusing to reap backup leader"));
    assert!(
        child.id().is_some(),
        "rejected reap must preserve leader identity"
    );

    assert!(
        !ownership
            .inspect_and_release_before_reap()
            .expect("leader-only group inspection should succeed")
    );
    assert!(ownership.is_released());
    let status = reap_leader_after_group_release(&mut child, &mut ownership)
        .await
        .expect("released leader should reap");
    assert!(status.success());
    assert!(child.id().is_none());
}
