use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{
    CommandRequest, CommandRunner, OutputStream, ProcessErrorKind, ReaderFault,
    SystemCommandRunner, TestHooks,
};

const COMMAND_BOUND: Duration = Duration::from_secs(10);
const CLEANUP_BOUND: Duration = Duration::from_secs(2);
const FIXTURE_READY_BOUND: Duration = Duration::from_secs(5);
const FIXTURE_FAILSAFE_SECS: u64 = 5;

#[derive(Clone)]
struct RunnerProbe {
    runner: Arc<SystemCommandRunner>,
    active_readers: Arc<AtomicUsize>,
    spawned_pids: Arc<Mutex<Vec<u32>>>,
    fault_release: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl RunnerProbe {
    fn new(fault: ReaderFault) -> Self {
        Self::with_options(fault, None, false, None)
    }

    fn with_inspection_fault(fault: ReaderFault, inject_inspection_failure: bool) -> Self {
        Self::with_options(fault, None, inject_inspection_failure, None)
    }

    fn with_spawn_failure(stream: OutputStream) -> Self {
        Self::with_options(ReaderFault::None, Some(stream), false, None)
    }

    fn gated(fault: ReaderFault) -> Self {
        Self::with_options(
            fault,
            None,
            false,
            Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
        )
    }

    fn with_options(
        fault: ReaderFault,
        spawn_failure: Option<OutputStream>,
        inject_inspection_failure: bool,
        fault_release: Option<Arc<std::sync::atomic::AtomicBool>>,
    ) -> Self {
        let active_readers = Arc::new(AtomicUsize::new(0));
        let spawned_pids = Arc::new(Mutex::new(Vec::new()));
        let runner = Arc::new(SystemCommandRunner::with_hooks(TestHooks {
            fault,
            spawn_failure,
            inject_inspection_failure,
            fault_release: fault_release.clone(),
            active_readers: Arc::clone(&active_readers),
            spawned_pids: Arc::clone(&spawned_pids),
        }));
        Self {
            runner,
            active_readers,
            spawned_pids,
            fault_release,
        }
    }

    fn run(
        &self,
        program: &Path,
        args: &[String],
        timeout: Duration,
    ) -> Result<super::CommandResult, super::ProcessError> {
        self.runner.run(CommandRequest {
            program: &program.to_string_lossy(),
            args,
            timeout,
        })
    }

    fn spawned(&self) -> Vec<u32> {
        self.spawned_pids.lock().unwrap().clone()
    }

    fn assert_no_readers(&self) {
        assert_eq!(
            self.active_readers.load(Ordering::SeqCst),
            0,
            "all output reader threads must be joined"
        );
    }

    fn release_fault(&self) {
        self.fault_release
            .as_ref()
            .expect("probe should have a fault gate")
            .store(true, Ordering::SeqCst);
    }
}

fn executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn read_pid(path: &Path) -> i32 {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("PID file {} should be readable: {error}", path.display()))
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("PID file {} should be numeric: {error}", path.display()))
}

fn pid_exists(pid: i32) -> bool {
    // SAFETY: signal 0 performs existence/permission probing only.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn wait_for_pid_exit(pid: i32) {
    let deadline = Instant::now() + CLEANUP_BOUND;
    while pid_exists(pid) {
        assert!(
            Instant::now() < deadline,
            "process {pid} remained after the cleanup deadline"
        );
        std::thread::yield_now();
    }
}

fn wait_for_file(path: &Path) {
    let deadline = Instant::now() + FIXTURE_READY_BOUND;
    while !path.is_file() {
        assert!(
            Instant::now() < deadline,
            "fixture file {} was not created before the deadline",
            path.display()
        );
        std::thread::yield_now();
    }
}

fn wait_for_reader_count(probe: &RunnerProbe, expected: usize) {
    let deadline = Instant::now() + CLEANUP_BOUND;
    while probe.active_readers.load(Ordering::SeqCst) != expected {
        assert!(
            Instant::now() < deadline,
            "reader count did not reach {expected} before the deadline"
        );
        std::thread::yield_now();
    }
}

#[test]
fn essentia_environment_process_success_captures_stdout_and_stderr() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "success",
        "printf 'stdout-value'; printf 'stderr-value' >&2",
    );
    let probe = RunnerProbe::new(ReaderFault::None);

    let result = probe.run(&script, &[], COMMAND_BOUND).unwrap();

    assert!(result.success);
    assert_eq!(result.stdout, b"stdout-value");
    assert_eq!(result.stderr, b"stderr-value");
    assert_eq!(probe.spawned().len(), 1);
    wait_for_pid_exit(probe.spawned()[0] as i32);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_nonzero_preserves_both_streams() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "nonzero",
        "printf 'stdout-value'; printf 'stderr-value' >&2; exit 7",
    );
    let probe = RunnerProbe::new(ReaderFault::None);

    let result = probe.run(&script, &[], COMMAND_BOUND).unwrap();

    assert!(!result.success);
    assert_eq!(result.stdout, b"stdout-value");
    assert_eq!(result.stderr, b"stderr-value");
    wait_for_pid_exit(probe.spawned()[0] as i32);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_direct_child_timeout_cleans_pid_and_readers() {
    let root = tempfile::tempdir().unwrap();
    let pid_file = root.path().join("leader.pid");
    let script = executable_script(
        root.path(),
        "direct-timeout",
        &format!("echo $$ > \"$1\"; exec sleep {FIXTURE_FAILSAFE_SECS}",),
    );
    let probe = RunnerProbe::new(ReaderFault::None);
    let started = Instant::now();
    let runner_probe = probe.clone();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let runner = std::thread::spawn(move || {
        let result = runner_probe.run(
            &script,
            &[pid_file.to_string_lossy().into_owned()],
            Duration::from_secs(2),
        );
        result_tx.send(result).unwrap();
    });

    wait_for_reader_count(&probe, 2);
    let error = result_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("timed command should complete before the outer bound")
        .unwrap_err();
    runner.join().unwrap();

    assert!(matches!(error.kind, ProcessErrorKind::Timeout));
    assert!(started.elapsed() < Duration::from_secs(4));
    wait_for_pid_exit(probe.spawned()[0] as i32);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_descendant_timeout_cleans_group_and_readers() {
    let root = tempfile::tempdir().unwrap();
    let leader_pid = root.path().join("leader.pid");
    let descendant_pid = root.path().join("descendant.pid");
    let ready = root.path().join("descendant.ready");
    let script = executable_script(
        root.path(),
        "descendant-timeout",
        &format!(
            "echo $$ > \"$1\"\nsh -c 'echo $$ > \"$1\"; : > \"$2\"; exec sleep {FIXTURE_FAILSAFE_SECS}' fixture \"$2\" \"$3\" &\nwhile [ ! -f \"$3\" ]; do :; done\nwait"
        ),
    );
    let probe = RunnerProbe::new(ReaderFault::None);

    let error = probe
        .run(
            &script,
            &[
                leader_pid.to_string_lossy().into_owned(),
                descendant_pid.to_string_lossy().into_owned(),
                ready.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(2),
        )
        .unwrap_err();

    assert!(matches!(error.kind, ProcessErrorKind::Timeout));
    wait_for_file(&ready);
    wait_for_pid_exit(read_pid(&leader_pid));
    wait_for_pid_exit(read_pid(&descendant_pid));
    probe.assert_no_readers();
}

fn run_early_exit_fixture(root: &Path, close_pipes: bool) {
    let leader_pid = root.join("leader.pid");
    let descendant_pid = root.join("descendant.pid");
    let ready = root.join("descendant.ready");
    let redirection = if close_pipes {
        "exec >/dev/null 2>&1; "
    } else {
        ""
    };
    let script = executable_script(
        root,
        "early-exit",
        &format!(
            "echo $$ > \"$1\"\nsh -c '{redirection}echo $$ > \"$1\"; : > \"$2\"; exec sleep {FIXTURE_FAILSAFE_SECS}' fixture \"$2\" \"$3\" &\nwhile [ ! -f \"$3\" ]; do :; done\nexit 0"
        ),
    );
    let probe = RunnerProbe::new(ReaderFault::None);
    let started = Instant::now();

    let error = probe
        .run(
            &script,
            &[
                leader_pid.to_string_lossy().into_owned(),
                descendant_pid.to_string_lossy().into_owned(),
                ready.to_string_lossy().into_owned(),
            ],
            COMMAND_BOUND,
        )
        .unwrap_err();

    assert!(matches!(error.kind, ProcessErrorKind::SurvivingDescendants));
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "runner must inspect and terminate descendants instead of waiting for their pipe fail-safe"
    );
    wait_for_file(&ready);
    wait_for_pid_exit(read_pid(&leader_pid));
    wait_for_pid_exit(read_pid(&descendant_pid));
    probe.assert_no_readers();
}

fn run_under_watchdog(test_name: &str, scenario: &str) {
    const WATCHDOG_ENV: &str = "REKLAWDBOX_ESSENTIA_PROCESS_WATCHDOG";
    const WATCHDOG_ROOT_ENV: &str = "REKLAWDBOX_ESSENTIA_PROCESS_WATCHDOG_ROOT";
    if std::env::var(WATCHDOG_ENV).as_deref() == Ok(scenario) {
        let root = PathBuf::from(
            std::env::var_os(WATCHDOG_ROOT_ENV).expect("watchdog root must be provided"),
        );
        run_early_exit_fixture(&root, scenario == "closed-pipes");
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test_name, "--nocapture"])
        .env(WATCHDOG_ENV, scenario)
        .env(WATCHDOG_ROOT_ENV, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait().unwrap() {
            Some(status) => break Some(status),
            None if Instant::now() < deadline => std::thread::yield_now(),
            None => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    // The outer watchdog owns the fixture root, so even a killed helper leaves
    // observable identities until both nested fixture processes have exited.
    // It never signals a recorded numeric PGID: the script's five-second
    // fail-safe is shorter than the eight-second watchdog, after which these
    // bounded checks prove that no fixture survived.
    for pid_file in [
        root.path().join("leader.pid"),
        root.path().join("descendant.pid"),
    ] {
        if pid_file.is_file() {
            wait_for_pid_exit(read_pid(&pid_file));
        }
    }
    match status {
        Some(status) => assert!(status.success(), "watchdog helper failed with {status}"),
        None => panic!("watchdog terminated hung helper for {scenario}"),
    }
}

#[test]
fn essentia_environment_process_leader_early_exit_with_open_pipes_is_bounded() {
    let test_name = format!(
        "{}::essentia_environment_process_leader_early_exit_with_open_pipes_is_bounded",
        module_path!().trim_start_matches("reklawdbox::")
    );
    run_under_watchdog(&test_name, "open-pipes");
}

#[test]
fn essentia_environment_process_leader_early_exit_with_closed_pipes_is_bounded() {
    let test_name = format!(
        "{}::essentia_environment_process_leader_early_exit_with_closed_pipes_is_bounded",
        module_path!().trim_start_matches("reklawdbox::")
    );
    run_under_watchdog(&test_name, "closed-pipes");
}

#[test]
fn essentia_environment_process_missing_captures_clean_child_without_readers() {
    for stream in [OutputStream::Stdout, OutputStream::Stderr] {
        let root = tempfile::tempdir().unwrap();
        let script = executable_script(
            root.path(),
            stream.name(),
            &format!("exec sleep {FIXTURE_FAILSAFE_SECS}"),
        );
        let probe = RunnerProbe::new(ReaderFault::Missing(stream));

        let error = probe.run(&script, &[], COMMAND_BOUND).unwrap_err();

        assert!(matches!(
            error.kind,
            ProcessErrorKind::MissingCapture(observed) if observed == stream
        ));
        for pid in probe.spawned() {
            wait_for_pid_exit(pid as i32);
        }
        probe.assert_no_readers();
    }
}

#[test]
fn essentia_environment_process_second_reader_spawn_failure_joins_first_reader() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "second-reader-start",
        &format!("exec sleep {FIXTURE_FAILSAFE_SECS}"),
    );
    let probe = RunnerProbe::with_spawn_failure(OutputStream::Stderr);
    let started = Instant::now();

    let error = probe.run(&script, &[], COMMAND_BOUND).unwrap_err();

    match error.kind {
        ProcessErrorKind::ReaderStart { stream, source } => {
            assert_eq!(stream, OutputStream::Stderr);
            assert!(source.to_string().contains("scripted stderr"));
        }
        other => panic!("unexpected reader start failure: {other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(4));
    wait_for_pid_exit(probe.spawned()[0] as i32);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_reader_failures_are_typed_and_joined() {
    for (fault, expected_stream, expects_panic) in [
        (
            ReaderFault::ReadError(OutputStream::Stdout),
            OutputStream::Stdout,
            false,
        ),
        (
            ReaderFault::Panic(OutputStream::Stderr),
            OutputStream::Stderr,
            true,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let script = executable_script(root.path(), expected_stream.name(), "exit 0");
        let probe = RunnerProbe::new(fault);

        let error = probe.run(&script, &[], COMMAND_BOUND).unwrap_err();

        match error.kind {
            ProcessErrorKind::ReaderRead { stream, source } => {
                assert!(!expects_panic);
                assert_eq!(stream, expected_stream);
                assert!(source.to_string().contains("scripted"));
            }
            ProcessErrorKind::ReaderPanicked(stream) => {
                assert!(expects_panic);
                assert_eq!(stream, expected_stream);
            }
            other => panic!("unexpected reader failure: {other:?}"),
        }
        wait_for_pid_exit(probe.spawned()[0] as i32);
        probe.assert_no_readers();
    }
}

#[test]
fn essentia_environment_process_live_reader_failure_terminates_before_timeout() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "live-reader-failure",
        &format!("exec sleep {FIXTURE_FAILSAFE_SECS}"),
    );
    let probe = RunnerProbe::gated(ReaderFault::ReadError(OutputStream::Stdout));
    let runner_probe = probe.clone();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let runner = std::thread::spawn(move || {
        result_tx
            .send(runner_probe.run(&script, &[], COMMAND_BOUND))
            .unwrap();
    });

    wait_for_reader_count(&probe, 2);
    let child_pid = probe.spawned()[0] as i32;
    assert!(
        pid_exists(child_pid),
        "fixture child must be live before fault"
    );
    probe.release_fault();
    let error = result_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("live reader failure should finish before outer bound")
        .unwrap_err();
    runner.join().unwrap();

    assert!(matches!(
        error.kind,
        ProcessErrorKind::ReaderRead {
            stream: OutputStream::Stdout,
            ..
        }
    ));
    assert!(started.elapsed() < Duration::from_secs(4));
    wait_for_pid_exit(child_pid);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_live_reader_panic_terminates_before_timeout() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "live-reader-panic",
        &format!("exec sleep {FIXTURE_FAILSAFE_SECS}"),
    );
    let probe = RunnerProbe::gated(ReaderFault::Panic(OutputStream::Stderr));
    let runner_probe = probe.clone();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let runner = std::thread::spawn(move || {
        result_tx
            .send(runner_probe.run(&script, &[], COMMAND_BOUND))
            .unwrap();
    });

    wait_for_reader_count(&probe, 2);
    let child_pid = probe.spawned()[0] as i32;
    assert!(
        pid_exists(child_pid),
        "fixture child must be live before reader panic"
    );
    probe.release_fault();
    let error = result_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("live reader panic should finish before outer bound")
        .unwrap_err();
    runner.join().unwrap();

    assert!(matches!(
        error.kind,
        ProcessErrorKind::ReaderPanicked(OutputStream::Stderr)
    ));
    assert!(started.elapsed() < Duration::from_secs(4));
    wait_for_pid_exit(child_pid);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_stalled_and_missing_completion_are_bounded() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "stalled-reader",
        &format!("exec sleep {FIXTURE_FAILSAFE_SECS}"),
    );
    let stalled = RunnerProbe::new(ReaderFault::StallUntilCancelled(OutputStream::Stdout));
    let error = stalled
        .run(&script, &[], Duration::from_millis(200))
        .unwrap_err();
    assert!(matches!(error.kind, ProcessErrorKind::Timeout));
    wait_for_pid_exit(stalled.spawned()[0] as i32);
    stalled.assert_no_readers();

    let missing_script = executable_script(root.path(), "missing-completion", "exit 0");
    let missing = RunnerProbe::new(ReaderFault::SuppressCompletion(OutputStream::Stderr));
    let error = missing
        .run(&missing_script, &[], COMMAND_BOUND)
        .unwrap_err();
    assert!(matches!(
        error.kind,
        ProcessErrorKind::ReaderShutdownTimeout
    ));
    wait_for_pid_exit(missing.spawned()[0] as i32);
    missing.assert_no_readers();
}

#[test]
fn essentia_environment_process_inspection_error_releases_reaps_and_joins() {
    let root = tempfile::tempdir().unwrap();
    let leader_pid = root.path().join("inspect-leader.pid");
    let descendant_pid = root.path().join("inspect-descendant.pid");
    let ready = root.path().join("inspect-descendant.ready");
    let script = executable_script(
        root.path(),
        "inspection-error",
        &format!(
            "echo $$ > \"$1\"\nsh -c 'echo $$ > \"$1\"; : > \"$2\"; exec sleep {FIXTURE_FAILSAFE_SECS}' fixture \"$2\" \"$3\" &\nwhile [ ! -f \"$3\" ]; do :; done\nexit 0"
        ),
    );
    let probe = RunnerProbe::with_inspection_fault(ReaderFault::None, true);

    let error = probe
        .run(
            &script,
            &[
                leader_pid.to_string_lossy().into_owned(),
                descendant_pid.to_string_lossy().into_owned(),
                ready.to_string_lossy().into_owned(),
            ],
            COMMAND_BOUND,
        )
        .unwrap_err();

    assert!(matches!(error.kind, ProcessErrorKind::ProcessGroup(_)));
    wait_for_file(&ready);
    wait_for_pid_exit(read_pid(&leader_pid));
    wait_for_pid_exit(read_pid(&descendant_pid));
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_stdout_reader_error_has_stable_precedence() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(root.path(), "reader-order", "exit 0");
    let probe = RunnerProbe::new(ReaderFault::ReadBoth);

    let error = probe.run(&script, &[], COMMAND_BOUND).unwrap_err();

    assert!(matches!(
        error.kind,
        ProcessErrorKind::ReaderRead {
            stream: OutputStream::Stdout,
            ..
        }
    ));
    wait_for_pid_exit(probe.spawned()[0] as i32);
    probe.assert_no_readers();
}

#[test]
fn essentia_environment_process_concurrent_commands_use_independent_groups() {
    let root = tempfile::tempdir().unwrap();
    let script = executable_script(
        root.path(),
        "concurrent",
        "echo $$ > \"$1\"; : > \"$2\"; while [ ! -f \"$3\" ]; do :; done; exit 0",
    );
    let first_pid = root.path().join("first.pid");
    let first_ready = root.path().join("first.ready");
    let first_release = root.path().join("first.release");
    let second_pid = root.path().join("second.pid");
    let second_ready = root.path().join("second.ready");
    let second_release = root.path().join("second.release");
    let probe = RunnerProbe::new(ReaderFault::None);
    let (result_tx, result_rx) = std::sync::mpsc::channel();

    let first_probe = probe.clone();
    let first_script = script.clone();
    let first_tx = result_tx.clone();
    let first_handle = std::thread::spawn(move || {
        let result = first_probe.run(
            &first_script,
            &[
                first_pid.to_string_lossy().into_owned(),
                first_ready.to_string_lossy().into_owned(),
                first_release.to_string_lossy().into_owned(),
            ],
            Duration::from_secs(2),
        );
        first_tx.send(("timed", result)).unwrap();
    });
    let second_probe = probe.clone();
    let second_script = script;
    let second_handle = std::thread::spawn(move || {
        let result = second_probe.run(
            &second_script,
            &[
                second_pid.to_string_lossy().into_owned(),
                second_ready.to_string_lossy().into_owned(),
                second_release.to_string_lossy().into_owned(),
            ],
            COMMAND_BOUND,
        );
        result_tx.send(("peer", result)).unwrap();
    });

    wait_for_file(&root.path().join("first.ready"));
    wait_for_file(&root.path().join("second.ready"));
    let first_pid = read_pid(&root.path().join("first.pid"));
    let second_pid = read_pid(&root.path().join("second.pid"));
    assert_ne!(first_pid, second_pid);
    // SAFETY: getpgid only observes the two live fixture PIDs.
    assert_eq!(unsafe { libc::getpgid(first_pid) }, first_pid);
    // SAFETY: getpgid only observes the two live fixture PIDs.
    assert_eq!(unsafe { libc::getpgid(second_pid) }, second_pid);
    let (label, result) = result_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("timed runner should complete before the peer is released");
    assert_eq!(label, "timed");
    let error = result.unwrap_err();
    assert!(matches!(error.kind, ProcessErrorKind::Timeout));
    wait_for_pid_exit(first_pid);

    assert!(
        pid_exists(second_pid),
        "peer must remain live after timeout cleanup"
    );
    // SAFETY: getpgid only observes the still-live peer fixture PID.
    assert_eq!(unsafe { libc::getpgid(second_pid) }, second_pid);
    wait_for_reader_count(&probe, 2);
    fs::write(root.path().join("second.release"), b"release").unwrap();

    let (label, result) = result_rx
        .recv_timeout(Duration::from_secs(4))
        .expect("released peer should complete before the outer bound");
    assert_eq!(label, "peer");
    assert!(result.unwrap().success);
    first_handle.join().unwrap();
    second_handle.join().unwrap();
    wait_for_pid_exit(first_pid);
    wait_for_pid_exit(second_pid);
    probe.assert_no_readers();
}
