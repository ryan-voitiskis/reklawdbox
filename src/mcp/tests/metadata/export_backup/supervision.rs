use super::support::{EnvVarGuard, backup_script_env_lock, write_executable_script};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[cfg(unix)]
pub(super) struct HangingBackupFixture {
    pub(super) script: std::path::PathBuf,
    pub(super) ready: std::path::PathBuf,
    pub(super) parent_pid: std::path::PathBuf,
    pub(super) descendant_pid: Option<std::path::PathBuf>,
    pub(super) delayed_sentinel: std::path::PathBuf,
}

#[cfg(unix)]
pub(super) fn shell_single_quote(path: &std::path::Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

#[cfg(unix)]
pub(super) fn write_hanging_backup_fixture(
    directory: &std::path::Path,
    with_descendant: bool,
) -> HangingBackupFixture {
    let script = directory.join(if with_descendant {
        "descendant-hang.sh"
    } else {
        "direct-hang.sh"
    });
    let ready = directory.join("ready");
    let parent_pid = directory.join("parent.pid");
    let descendant_pid = with_descendant.then(|| directory.join("descendant.pid"));
    let delayed_sentinel = directory.join("survived-timeout");
    let block_fifo = directory.join("block.fifo");
    let ready_fifo = directory.join("ready.fifo");

    let contents = if let Some(descendant_pid) = descendant_pid.as_ref() {
        format!(
            "#!/bin/bash\n\
             set -eu\n\
             mkfifo {block_fifo} {ready_fifo}\n\
             exec 3<> {block_fifo}\n\
             exec 4<> {ready_fifo}\n\
             (\n\
               printf '%s\\n' \"$BASHPID\" > {descendant_pid}\n\
               printf 'descendant-ready\\n'\n\
               printf 'descendant-stderr-ready\\n' >&2\n\
               printf 'ready\\n' >&4\n\
               IFS= read -r -t 1 _ <&3 || true\n\
               printf 'descendant survived timeout\\n' > {delayed_sentinel}\n\
               while :; do IFS= read -r -t 60 _ <&3 || true; done\n\
             ) &\n\
             descendant=$!\n\
             printf '%s\\n' \"$$\" > {parent_pid}\n\
             IFS= read -r _ <&4\n\
             printf 'ready\\n' > {ready}\n\
             wait \"$descendant\"\n",
            block_fifo = shell_single_quote(&block_fifo),
            ready_fifo = shell_single_quote(&ready_fifo),
            descendant_pid = shell_single_quote(descendant_pid),
            delayed_sentinel = shell_single_quote(&delayed_sentinel),
            parent_pid = shell_single_quote(&parent_pid),
            ready = shell_single_quote(&ready),
        )
    } else {
        format!(
            "#!/bin/bash\n\
             set -eu\n\
             mkfifo {block_fifo}\n\
             exec 3<> {block_fifo}\n\
             printf '%s\\n' \"$$\" > {parent_pid}\n\
             printf 'parent-ready\\n'\n\
             printf 'parent-stderr-ready\\n' >&2\n\
             printf 'ready\\n' > {ready}\n\
             IFS= read -r -t 1 _ <&3 || true\n\
             printf 'parent survived timeout\\n' > {delayed_sentinel}\n\
             while :; do IFS= read -r -t 60 _ <&3 || true; done\n",
            block_fifo = shell_single_quote(&block_fifo),
            parent_pid = shell_single_quote(&parent_pid),
            ready = shell_single_quote(&ready),
            delayed_sentinel = shell_single_quote(&delayed_sentinel),
        )
    };
    write_executable_script(&script, &contents);
    HangingBackupFixture {
        script,
        ready,
        parent_pid,
        descendant_pid,
        delayed_sentinel,
    }
}

#[cfg(unix)]
pub(super) fn write_early_exit_backup_fixture(directory: &std::path::Path) -> HangingBackupFixture {
    write_early_exit_backup_fixture_with_output(directory, false)
}

#[cfg(unix)]
pub(super) fn write_early_exit_backup_fixture_with_closed_output(
    directory: &std::path::Path,
) -> HangingBackupFixture {
    write_early_exit_backup_fixture_with_output(directory, true)
}

#[cfg(unix)]
pub(super) fn write_early_exit_backup_fixture_with_output(
    directory: &std::path::Path,
    close_output: bool,
) -> HangingBackupFixture {
    let script = directory.join("early-exit-descendant.sh");
    let ready = directory.join("ready");
    let parent_pid = directory.join("parent.pid");
    let descendant_pid = directory.join("descendant.pid");
    let delayed_sentinel = directory.join("survived-parent-exit");
    let block_fifo = directory.join("block.fifo");
    let ready_fifo = directory.join("ready.fifo");
    let close_output = if close_output {
        "exec 1>/dev/null 2>/dev/null\n"
    } else {
        ""
    };
    let contents = format!(
        "#!/bin/bash\n\
         set -eu\n\
         mkfifo {block_fifo} {ready_fifo}\n\
         exec 3<> {block_fifo}\n\
         exec 4<> {ready_fifo}\n\
         (\n\
           printf '%s\\n' \"$BASHPID\" > {descendant_pid}\n\
           printf 'early-exit-descendant-ready\\n'\n\
           printf 'early-exit-descendant-stderr-ready\\n' >&2\n\
           printf 'ready\\n' >&4\n\
           {close_output}\
           IFS= read -r -t 1 _ <&3 || true\n\
           printf 'descendant survived parent exit\\n' > {delayed_sentinel}\n\
           while :; do IFS= read -r -t 60 _ <&3 || true; done\n\
         ) &\n\
         printf '%s\\n' \"$$\" > {parent_pid}\n\
         IFS= read -r _ <&4\n\
         printf 'ready\\n' > {ready}\n\
         exit 0\n",
        block_fifo = shell_single_quote(&block_fifo),
        ready_fifo = shell_single_quote(&ready_fifo),
        close_output = close_output,
        descendant_pid = shell_single_quote(&descendant_pid),
        ready = shell_single_quote(&ready),
        delayed_sentinel = shell_single_quote(&delayed_sentinel),
        parent_pid = shell_single_quote(&parent_pid),
    );
    write_executable_script(&script, &contents);
    HangingBackupFixture {
        script,
        ready,
        parent_pid,
        descendant_pid: Some(descendant_pid),
        delayed_sentinel,
    }
}

#[cfg(unix)]
pub(super) async fn wait_for_nonempty_file(path: &std::path::Path, label: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = std::fs::read_to_string(path)
                && !contents.trim().is_empty()
            {
                return contents;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {label}: {}", path.display()))
}

#[cfg(unix)]
pub(super) fn fixture_pid(path: &std::path::Path, label: &str) -> i32 {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {label} {}: {error}", path.display()))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("invalid {label} in {}: {error}", path.display()))
}

#[cfg(unix)]
pub(super) fn pid_exists(pid: i32) -> bool {
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(unix)]
pub(super) fn kill_fixture_pid(pid: i32) {
    if pid_exists(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(unix)]
pub(super) async fn wait_for_pid_exit(pid: i32, label: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while pid_exists(pid) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{label} PID {pid} remained alive"));
}

#[cfg(unix)]
pub(super) async fn assert_path_remains_absent(path: &std::path::Path) {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_200);
    while tokio::time::Instant::now() < deadline {
        assert!(
            !path.exists(),
            "delayed survival sentinel appeared: {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[cfg(unix)]
pub(super) async fn assert_pre_op_backup_timeout_terminates_fixture(with_descendant: bool) {
    let temp = tempfile::tempdir().expect("hanging backup fixture directory should create");
    let fixture = write_hanging_backup_fixture(temp.path(), with_descendant);
    let script = fixture.script.clone();
    let mut backup_task = tokio::spawn(async move {
        crate::adapters::rekordbox::backup::execute_script_with_timeout_for_test(
            &script,
            Duration::from_millis(250),
        )
        .await
    });

    wait_for_nonempty_file(&fixture.ready, "backup ready marker").await;
    let parent_pid = fixture_pid(&fixture.parent_pid, "parent PID");
    let descendant_pid = fixture
        .descendant_pid
        .as_deref()
        .map(|path| fixture_pid(path, "descendant PID"));

    let result = match tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await {
        Ok(joined) => joined.expect("backup timeout task should join"),
        Err(_) => {
            if let Some(pid) = descendant_pid {
                kill_fixture_pid(pid);
            }
            kill_fixture_pid(parent_pid);
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await;
            panic!("pre-operation backup ignored its injected 250ms timeout");
        }
    };
    let error = result.expect_err("hanging pre-operation backup should time out");
    assert!(
        error.starts_with("pre-operation backup timed out after"),
        "unexpected timeout error: {error}"
    );

    wait_for_pid_exit(parent_pid, "backup parent").await;
    if let Some(pid) = descendant_pid {
        wait_for_pid_exit(pid, "backup descendant").await;
    }
    assert_path_remains_absent(&fixture.delayed_sentinel).await;
}

#[cfg(unix)]
pub(super) async fn assert_pre_op_backup_rejects_early_exit_descendant(close_output: bool) {
    let temp = tempfile::tempdir().expect("early-exit backup fixture directory should create");
    let fixture = if close_output {
        write_early_exit_backup_fixture_with_closed_output(temp.path())
    } else {
        write_early_exit_backup_fixture(temp.path())
    };
    let script = fixture.script.clone();
    let mut backup_task = tokio::spawn(async move {
        crate::adapters::rekordbox::backup::execute_script_with_timeout_for_test(
            &script,
            Duration::from_millis(250),
        )
        .await
    });

    wait_for_nonempty_file(&fixture.ready, "early-exit backup ready marker").await;
    let parent_pid = fixture_pid(&fixture.parent_pid, "early-exit parent PID");
    let descendant_pid = fixture_pid(
        fixture
            .descendant_pid
            .as_deref()
            .expect("early-exit fixture should record a descendant PID"),
        "early-exit descendant PID",
    );

    let result = match tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await {
        Ok(joined) => joined.expect("early-exit backup task should join"),
        Err(_) => {
            kill_fixture_pid(descendant_pid);
            kill_fixture_pid(parent_pid);
            let _ = tokio::time::timeout(Duration::from_secs(5), &mut backup_task).await;
            panic!("pre-operation backup blocked on an early-exit descendant");
        }
    };
    let error = result.expect_err("background backup work must fail closed");
    assert!(
        error.starts_with("backup script exited while descendant processes were still running"),
        "unexpected early-exit error: {error}"
    );

    wait_for_pid_exit(parent_pid, "early-exit backup parent").await;
    wait_for_pid_exit(descendant_pid, "early-exit backup descendant").await;
    assert_path_remains_absent(&fixture.delayed_sentinel).await;
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_success_path_env_preserves_first_argument_and_parent_env() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let configured_dir = temp.path().join("Configured Library");
    std::fs::create_dir(&configured_dir).expect("configured directory should create");
    let configured = configured_dir.join("master.db");
    std::fs::write(&configured, []).expect("configured master.db should create");
    let canonical = configured
        .canonicalize()
        .expect("configured master.db should canonicalize");

    let parent_value = temp.path().join("parent-process-master.db");
    let _db_env = EnvVarGuard::set("REKORDBOX_DB_PATH", &parent_value);
    let marker = temp.path().join("custom-script-marker.txt");
    let script = temp.path().join("custom backup.sh");
    write_executable_script(
        &script,
        &format!(
            "#!/bin/sh\nprintf '%s\\n%s\\n' \"$1\" \"$REKORDBOX_DB_PATH\" > '{}'\n",
            marker.display()
        ),
    );
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let status = crate::adapters::rekordbox::backup::run_pre_op_backup(&canonical)
        .await
        .expect("custom script zero exit should attest success");
    assert_eq!(
        status,
        crate::adapters::rekordbox::backup::BackupStatus::Success
    );
    let observed = std::fs::read_to_string(&marker).expect("custom script marker should exist");
    let mut lines = observed.lines();
    assert_eq!(lines.next(), Some("--pre-op"));
    assert_eq!(lines.next(), canonical.to_str());
    assert_eq!(lines.next(), None);
    assert_eq!(
        std::env::var_os("REKORDBOX_DB_PATH"),
        Some(parent_value.into())
    );
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_output_is_bounded() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");
    let script = temp.path().join("noisy-failure.sh");
    write_executable_script(
        &script,
        "#!/bin/sh\ni=0\nwhile [ \"$i\" -lt 9000 ]; do printf x >&2; i=$((i + 1)); done\nexit 17\n",
    );
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("nonzero custom script should fail");
    assert!(error.contains("exit status 17"));
    assert!(error.contains("[truncated]"));
    assert!(error.len() < 8_500, "failure output should remain bounded");
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_nonzero_exit_is_reported() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");
    let script = temp.path().join("nonzero.sh");
    write_executable_script(&script, "#!/bin/sh\necho 'nonzero backup' >&2\nexit 19\n");
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("nonzero custom script should fail");
    assert!(error.contains("exit status 19"));
    assert!(error.contains("nonzero backup"));
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_nonzero_diagnostic_precedence_is_stable() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");

    let cases = [
        (
            "stderr-first.sh",
            "#!/bin/sh\nprintf 'stdout detail\\n'\nprintf 'stderr detail\\n' >&2\nexit 7\n",
            "backup failed with exit status 7: stderr detail",
        ),
        (
            "stdout-fallback.sh",
            "#!/bin/sh\nprintf 'stdout detail\\n'\nexit 8\n",
            "backup failed with exit status 8: stdout detail",
        ),
        (
            "no-output.sh",
            "#!/bin/sh\nexit 9\n",
            "backup failed with exit status 9: backup script exited without output",
        ),
    ];

    for (name, contents, expected) in cases {
        let script = temp.path().join(name);
        write_executable_script(&script, contents);
        let script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);
        let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
            .await
            .expect_err("nonzero custom script should fail");
        assert_eq!(error, expected);
        drop(script_env);
    }
}

#[tokio::test]
#[cfg(unix)]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_success_with_stdout_and_stderr_is_stable() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");
    let temp = tempfile::tempdir().expect("temp backup fixture should create");
    let db_path = temp.path().join("master.db");
    std::fs::write(&db_path, []).expect("configured master.db should create");
    let script = temp.path().join("output-success.sh");
    write_executable_script(
        &script,
        "#!/bin/sh\nprintf 'successful stdout\\n'\nprintf 'successful stderr\\n' >&2\nexit 0\n",
    );
    let _script_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &script);

    let status = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect("zero exit with both output streams should succeed");
    assert_eq!(
        status,
        crate::adapters::rekordbox::backup::BackupStatus::Success
    );
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_timeout_reaps_direct_hung_child() {
    assert_pre_op_backup_timeout_terminates_fixture(false).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_timeout_reaps_descendant_holding_output_pipes() {
    assert_pre_op_backup_timeout_terminates_fixture(true).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_early_exit_reaps_descendant_holding_output_pipes() {
    assert_pre_op_backup_rejects_early_exit_descendant(false).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_early_exit_detects_descendant_that_closed_output_pipes() {
    assert_pre_op_backup_rejects_early_exit_descendant(true).await;
}

#[tokio::test]
#[cfg(unix)]
async fn pre_op_backup_cancellation_keeps_supervisor_cleanup_alive() {
    let temp = tempfile::tempdir().expect("cancelled backup fixture directory should create");
    let fixture = write_hanging_backup_fixture(temp.path(), true);
    let script = fixture.script.clone();
    let reader_activity = Arc::new(AtomicUsize::new(0));
    let task_activity = Arc::clone(&reader_activity);
    let caller = tokio::spawn(async move {
        crate::adapters::rekordbox::backup::execute_script_with_timeout_and_activity_for_test(
            &script,
            Duration::from_millis(250),
            task_activity,
        )
        .await
    });

    wait_for_nonempty_file(&fixture.ready, "cancelled backup ready marker").await;
    let parent_pid = fixture_pid(&fixture.parent_pid, "cancelled backup parent PID");
    let descendant_pid = fixture_pid(
        fixture
            .descendant_pid
            .as_deref()
            .expect("cancelled fixture should record a descendant PID"),
        "cancelled backup descendant PID",
    );
    tokio::time::timeout(Duration::from_secs(1), async {
        while reader_activity.load(Ordering::Acquire) != 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled backup output readers should start");

    caller.abort();
    assert!(
        caller
            .await
            .expect_err("cancelled caller task should not complete")
            .is_cancelled()
    );

    let cleanup = tokio::time::timeout(Duration::from_secs(5), async {
        while pid_exists(parent_pid)
            || pid_exists(descendant_pid)
            || reader_activity.load(Ordering::Acquire) != 0
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if let Err(error) = cleanup {
        kill_fixture_pid(descendant_pid);
        kill_fixture_pid(parent_pid);
        panic!("cancelled backup supervisor did not quiesce: {error}");
    }
    assert_path_remains_absent(&fixture.delayed_sentinel).await;
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn pre_op_backup_missing_script_fails_closed() {
    let _env_guard = backup_script_env_lock()
        .lock()
        .expect("backup env mutex should not be poisoned");

    let backup_dir = tempfile::tempdir().expect("temp backup dir should create");
    let missing_script = backup_dir.path().join("missing-backup.sh");
    let db_path = backup_dir.path().join("master.db");
    std::fs::write(&db_path, b"test db").expect("temp master.db should create");
    let _backup_env = EnvVarGuard::set("REKLAWDBOX_BACKUP_SCRIPT", &missing_script);

    let error = crate::adapters::rekordbox::backup::run_pre_op_backup(&db_path)
        .await
        .expect_err("missing custom backup script must fail closed");
    assert!(error.contains(&missing_script.to_string_lossy().to_string()));
    assert!(!error.contains("REKORDBOX_DB_PATH="));
    assert!(!error.contains("environment"));
}
