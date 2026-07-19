use super::*;

pub(super) fn backup_script_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

pub(super) struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    pub(super) fn set(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[cfg(unix)]
pub(super) fn write_executable_script(path: &std::path::Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, contents).expect("test script should be written");
    let mut permissions = std::fs::metadata(path)
        .expect("test script metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("test script should be executable");
}

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

#[cfg(unix)]
pub(super) fn run_embedded_backup_script(
    args: &[&str],
    home: &std::path::Path,
    db_path: Option<&std::path::Path>,
    stdin: Option<&str>,
) -> std::process::Output {
    run_embedded_backup_script_with_temp_dir(args, home, db_path, stdin, &home.join("tmp"))
}

#[cfg(unix)]
pub(super) fn run_embedded_backup_script_with_temp_dir(
    args: &[&str],
    home: &std::path::Path,
    db_path: Option<&std::path::Path>,
    stdin: Option<&str>,
    temp_dir: &std::path::Path,
) -> std::process::Output {
    use std::io::Write as _;
    use std::os::unix::process::CommandExt as _;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let (script, _script_dir) =
        crate::adapters::rekordbox::backup::write_embedded_script_for_test()
            .expect("embedded backup script should be materialized");
    let fake_bin = home.join("test-bin");
    std::fs::create_dir_all(&fake_bin).expect("fake binary directory should create");
    write_executable_script(&fake_bin.join("pgrep"), "#!/bin/sh\nexit 1\n");
    std::fs::create_dir_all(temp_dir).expect("child temp directory should create");

    let mut command = Command::new("/bin/bash");
    command
        .arg(&script)
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("TMPDIR", temp_dir)
        .env("PATH", format!("{}:/usr/bin:/bin", fake_bin.display()))
        .env("LANG", "C")
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = db_path {
        command.env("REKORDBOX_DB_PATH", path);
    }

    let mut child = command
        .spawn()
        .expect("embedded backup child should launch");
    if let Some(input) = stdin {
        child
            .stdin
            .take()
            .expect("backup child stdin should be piped")
            .write_all(input.as_bytes())
            .expect("backup child input should be written");
    } else {
        drop(child.stdin.take());
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child
            .try_wait()
            .expect("backup child status should be readable")
        {
            Some(_) => break,
            None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            None => {
                let process_group = -(child.id() as i32);
                unsafe {
                    libc::kill(process_group, libc::SIGKILL);
                }
                let _ = child.wait();
                panic!("embedded backup child exceeded the 10-second test timeout");
            }
        }
    }

    child
        .wait_with_output()
        .expect("backup child output should be collected")
}

#[cfg(unix)]
pub(super) fn child_output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(unix)]
pub(super) fn backup_archives(home: &std::path::Path, prefix: &str) -> Vec<std::path::PathBuf> {
    let backup_dir = home.join("Music/rekordbox-backups");
    let mut archives = if backup_dir.is_dir() {
        std::fs::read_dir(&backup_dir)
            .expect("backup directory should be readable")
            .map(|entry| {
                entry
                    .expect("backup directory entry should be readable")
                    .path()
            })
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(prefix) && name.ends_with(".tar.gz"))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    archives.sort();
    archives
}

#[cfg(unix)]
pub(super) fn tar_members(archive: &std::path::Path) -> Vec<String> {
    let output = std::process::Command::new("tar")
        .args(["-tzf"])
        .arg(archive)
        .output()
        .expect("tar member listing should launch");
    assert!(
        output.status.success(),
        "tar member listing should succeed: {}",
        child_output_text(&output)
    );
    String::from_utf8(output.stdout)
        .expect("tar member names should be UTF-8 test paths")
        .lines()
        .map(str::to_owned)
        .collect()
}

#[cfg(unix)]
pub(super) fn create_backup_archive_fixture(
    archive: &std::path::Path,
    source: &std::path::Path,
    members: &[&str],
) {
    let output = std::process::Command::new("tar")
        .args(["-czf"])
        .arg(archive)
        .arg("-C")
        .arg(source)
        .arg("--")
        .args(members)
        .output()
        .expect("DB backup fixture creation should launch");
    assert!(
        output.status.success(),
        "DB backup fixture creation should succeed: {}",
        child_output_text(&output)
    );
}

pub(super) const WRITE_XML_TASK_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) type WriteXmlTaskOutput = Result<CallToolResult, McpError>;

pub(super) struct WriteXmlTaskCleanup {
    handles: Vec<Option<tokio::task::JoinHandle<WriteXmlTaskOutput>>>,
}

impl WriteXmlTaskCleanup {
    pub(super) fn new() -> Self {
        Self {
            handles: Vec::new(),
        }
    }

    pub(super) fn push(&mut self, handle: tokio::task::JoinHandle<WriteXmlTaskOutput>) {
        self.handles.push(Some(handle));
    }

    pub(super) fn all_pending(&self) -> bool {
        self.handles
            .iter()
            .flatten()
            .all(|handle| !handle.is_finished())
    }

    pub(super) async fn join(
        &mut self,
        index: usize,
        phase: &str,
    ) -> Result<WriteXmlTaskOutput, String> {
        let mut handle = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| format!("{phase}: task handle is missing"))?;

        match tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(err)) => Err(format!("{phase}: task join failed: {err}")),
            Err(_) => {
                handle.abort();
                let cleanup = tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await;
                if cleanup.is_err() {
                    return Err(format!(
                        "{phase}: task timed out and abort cleanup did not finish within five seconds"
                    ));
                }
                Err(format!("{phase}: task did not finish within five seconds"))
            }
        }
    }

    pub(super) async fn abort(&mut self, index: usize, phase: &str) -> Result<(), String> {
        let mut handle = self
            .handles
            .get_mut(index)
            .and_then(Option::take)
            .ok_or_else(|| format!("{phase}: task handle is missing"))?;
        handle.abort();
        match tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle).await {
            Ok(Err(err)) if err.is_cancelled() => Ok(()),
            Ok(Err(err)) => Err(format!("{phase}: aborted task join failed: {err}")),
            Ok(Ok(_)) => Err(format!("{phase}: task completed before cancellation")),
            Err(_) => Err(format!(
                "{phase}: aborted task did not join within five seconds"
            )),
        }
    }

    pub(super) async fn abort_all(&mut self) -> Result<(), String> {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }

        for (index, slot) in self.handles.iter_mut().enumerate() {
            let Some(mut handle) = slot.take() else {
                continue;
            };
            if tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, &mut handle)
                .await
                .is_err()
            {
                return Err(format!(
                    "task {index} did not join during cleanup within five seconds"
                ));
            }
        }
        Ok(())
    }
}

impl Drop for WriteXmlTaskCleanup {
    fn drop(&mut self) {
        for handle in self.handles.iter().flatten() {
            handle.abort();
        }
    }
}

pub(super) fn spawn_queued_write_xml(
    server: ReklawdboxServer,
    params: WriteXmlParams,
    queued: Arc<tokio::sync::Notify>,
) -> tokio::task::JoinHandle<WriteXmlTaskOutput> {
    tokio::spawn(async move {
        let mut request = Box::pin(server.write_xml(Parameters(params)));
        std::future::poll_fn(|cx| match request.as_mut().poll(cx) {
            std::task::Poll::Pending => std::task::Poll::Ready(()),
            std::task::Poll::Ready(_) => {
                panic!("write_xml completed instead of waiting for the held export lock")
            }
        })
        .await;
        queued.notify_one();
        request.await
    })
}

pub(super) async fn wait_for_queued_write_xml(
    queued: &tokio::sync::Notify,
    phase: &str,
) -> Result<(), String> {
    tokio::time::timeout(WRITE_XML_TASK_TIMEOUT, queued.notified())
        .await
        .map_err(|_| format!("{phase}: write_xml did not queue within five seconds"))
}
