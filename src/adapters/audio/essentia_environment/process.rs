//! Synchronous, bounded ownership of Essentia subprocesses.
//!
//! This is intentionally independent from the Tokio backup supervisor. The
//! only shared mechanism is the synchronous process-group identity primitive.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::adapters::platform::process_group::{ProcessGroupError, ProcessGroupOwnership};

use super::platform;

const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(25);
const READER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const READER_SHUTDOWN_BOUND: Duration = Duration::from_secs(1);

#[cfg(unix)]
trait CapturedRead: Read + Send {
    fn raw_fd(&self) -> std::os::fd::RawFd;
}
#[cfg(unix)]
impl<T: Read + std::os::fd::AsRawFd + Send> CapturedRead for T {
    fn raw_fd(&self) -> std::os::fd::RawFd {
        std::os::fd::AsRawFd::as_raw_fd(self)
    }
}

#[cfg(not(unix))]
trait CapturedRead: Read + Send {}
#[cfg(not(unix))]
impl<T: Read + Send> CapturedRead for T {}

type CapturedOutput = Box<dyn CapturedRead>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommandResult {
    pub(super) success: bool,
    pub(super) stdout: String,
    pub(super) stderr: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CommandRequest<'a> {
    pub(super) program: &'a str,
    pub(super) args: &'a [String],
    pub(super) timeout: Duration,
}

pub(super) trait CommandRunner {
    fn run(&self, request: CommandRequest<'_>) -> Result<CommandResult, ProcessError>;
}

#[derive(Debug)]
pub(super) struct ProcessError {
    pub(super) kind: ProcessErrorKind,
}

impl ProcessError {
    fn new(kind: ProcessErrorKind) -> Self {
        Self { kind }
    }
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ProcessErrorKind::Start(error) => write!(formatter, "process start failed: {error}"),
            ProcessErrorKind::ProcessGroup(error) => {
                write!(formatter, "process-group lifecycle failed: {error:?}")
            }
            ProcessErrorKind::MissingCapture(stream) => {
                write!(formatter, "missing {} capture", stream.name())
            }
            ProcessErrorKind::Wait(error) => write!(formatter, "process wait failed: {error}"),
            ProcessErrorKind::Timeout => formatter.write_str("process timed out"),
            ProcessErrorKind::SurvivingDescendants => {
                formatter.write_str("process left surviving descendants")
            }
            ProcessErrorKind::ReaderRead { stream, source } => {
                write!(formatter, "{} reader failed: {source}", stream.name())
            }
            ProcessErrorKind::ReaderPanicked(stream) => {
                write!(formatter, "{} reader panicked", stream.name())
            }
            ProcessErrorKind::ReaderShutdownTimeout => {
                formatter.write_str("output readers missed the bounded shutdown handshake")
            }
        }
    }
}

impl std::error::Error for ProcessError {}

#[derive(Debug)]
pub(super) enum ProcessErrorKind {
    Start(std::io::Error),
    ProcessGroup(ProcessGroupError),
    MissingCapture(OutputStream),
    Wait(std::io::Error),
    Timeout,
    SurvivingDescendants,
    ReaderRead {
        stream: OutputStream,
        source: std::io::Error,
    },
    ReaderPanicked(OutputStream),
    ReaderShutdownTimeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Default)]
pub(super) struct SystemCommandRunner {
    #[cfg(test)]
    hooks: Option<TestHooks>,
}

impl CommandRunner for SystemCommandRunner {
    fn run(&self, request: CommandRequest<'_>) -> Result<CommandResult, ProcessError> {
        let mut command = isolated_command(request.program);
        let mut child = command
            .args(request.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| ProcessError::new(ProcessErrorKind::Start(error)))?;
        self.record_spawn(child.id());

        let mut ownership = match ProcessGroupOwnership::new(Some(child.id())) {
            Ok(ownership) => ownership,
            Err(error) => {
                terminate_unowned_child(&mut child);
                return Err(process_group_error(error));
            }
        };

        let stdout = self.take_capture(&mut child, OutputStream::Stdout);
        let stderr = self.take_capture(&mut child, OutputStream::Stderr);
        let (stdout, stderr) = match (stdout, stderr) {
            (Some(stdout), Some(stderr)) => (stdout, stderr),
            (stdout, stderr) => {
                let missing = if stdout.is_none() {
                    OutputStream::Stdout
                } else {
                    OutputStream::Stderr
                };
                drop(stdout);
                drop(stderr);
                terminate_owned_child(&mut child, &mut ownership);
                return Err(ProcessError::new(ProcessErrorKind::MissingCapture(missing)));
            }
        };

        let deadline = Deadline::new(request.timeout);
        let (reader_tx, reader_rx) = mpsc::channel();
        let reader_control = Arc::new(ReaderControl::new(deadline.end));
        let readers = vec![
            self.spawn_reader(
                OutputStream::Stdout,
                stdout,
                reader_tx.clone(),
                Arc::clone(&reader_control),
            ),
            self.spawn_reader(
                OutputStream::Stderr,
                stderr,
                reader_tx,
                Arc::clone(&reader_control),
            ),
        ];
        let process_outcome = wait_for_process(&mut child, &mut ownership, &deadline);
        if process_outcome.is_err() {
            reader_control.cancel();
        }
        let reader_outcome = collect_readers(readers, reader_rx, &deadline, &reader_control);

        let process_outcome = process_outcome?;
        let streams = reader_outcome?;
        if process_outcome.had_surviving_descendants {
            return Err(ProcessError::new(ProcessErrorKind::SurvivingDescendants));
        }
        if deadline.expired() {
            return Err(ProcessError::new(ProcessErrorKind::Timeout));
        }
        Ok(CommandResult {
            success: process_outcome.status.success(),
            stdout: String::from_utf8_lossy(&streams.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&streams.stderr).into_owned(),
        })
    }
}

fn isolated_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    command
}

#[derive(Debug)]
struct ProcessOutcome {
    status: ExitStatus,
    had_surviving_descendants: bool,
}

#[cfg(unix)]
fn wait_for_process(
    child: &mut Child,
    ownership: &mut ProcessGroupOwnership,
    deadline: &Deadline,
) -> Result<ProcessOutcome, ProcessError> {
    loop {
        match ownership.leader_exit_observed_without_reaping() {
            Ok(true) => break,
            Ok(false) if !deadline.expired() => deadline.sleep_poll(),
            Ok(false) => {
                terminate_owned_child(child, ownership);
                return Err(ProcessError::new(ProcessErrorKind::Timeout));
            }
            Err(error) => {
                terminate_owned_child(child, ownership);
                return Err(process_group_error(error));
            }
        }
    }

    let had_surviving_descendants = match ownership.inspect_and_release_before_reap() {
        Ok(had_surviving_descendants) => had_surviving_descendants,
        Err(error) => {
            // Inspection errors still leave this function owning the child.
            // Release/terminate the group first, then reap the unreaped leader.
            terminate_owned_child(child, ownership);
            return Err(process_group_error(error));
        }
    };
    debug_assert!(ownership.is_released());
    let status = child
        .wait()
        .map_err(|error| ProcessError::new(ProcessErrorKind::Wait(error)))?;
    Ok(ProcessOutcome {
        status,
        had_surviving_descendants,
    })
}

#[cfg(not(unix))]
fn wait_for_process(
    child: &mut Child,
    ownership: &mut ProcessGroupOwnership,
    deadline: &Deadline,
) -> Result<ProcessOutcome, ProcessError> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                debug_assert!(ownership.is_released());
                return Ok(ProcessOutcome {
                    status,
                    had_surviving_descendants: false,
                });
            }
            Ok(None) if !deadline.expired() => deadline.sleep_poll(),
            Ok(None) => {
                terminate_owned_child(child, ownership);
                return Err(ProcessError::new(ProcessErrorKind::Timeout));
            }
            Err(error) => {
                terminate_owned_child(child, ownership);
                return Err(ProcessError::new(ProcessErrorKind::Wait(error)));
            }
        }
    }
}

fn terminate_owned_child(child: &mut Child, ownership: &mut ProcessGroupOwnership) {
    for error in ownership.terminate_owned() {
        tracing::warn!("Essentia process-group cleanup failed: {error:?}");
    }
    debug_assert!(ownership.is_released());
    let _ = child.kill();
    let _ = child.wait();
}

fn terminate_unowned_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn process_group_error(error: ProcessGroupError) -> ProcessError {
    ProcessError::new(ProcessErrorKind::ProcessGroup(error))
}

struct Deadline {
    end: Instant,
}

impl Deadline {
    fn new(timeout: Duration) -> Self {
        Self {
            end: Instant::now()
                .checked_add(timeout)
                .unwrap_or_else(Instant::now),
        }
    }

    fn remaining(&self) -> Option<Duration> {
        self.end.checked_duration_since(Instant::now())
    }

    fn expired(&self) -> bool {
        self.remaining().is_none()
    }

    fn sleep_poll(&self) {
        if let Some(remaining) = self.remaining() {
            std::thread::sleep(PROCESS_POLL_INTERVAL.min(remaining));
        }
    }
}

struct ReaderHandle {
    stream: OutputStream,
    handle: JoinHandle<()>,
}

struct ReaderMessage {
    stream: OutputStream,
    outcome: ReaderOutcome,
}

enum ReaderOutcome {
    Bytes(Vec<u8>),
    ReadError(std::io::Error),
    Panicked,
    TimedOut,
}

struct CapturedStreams {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn collect_readers(
    readers: Vec<ReaderHandle>,
    receiver: Receiver<ReaderMessage>,
    deadline: &Deadline,
    control: &ReaderControl,
) -> Result<CapturedStreams, ProcessError> {
    let mut stdout = None;
    let mut stderr = None;
    let mut received = 0;
    let mut shutdown_timed_out = false;
    while received < readers.len() {
        let Some(remaining) = deadline.remaining() else {
            control.cancel();
            break;
        };
        match receiver.recv_timeout(remaining) {
            Ok(message) => {
                store_reader_message(message, &mut stdout, &mut stderr);
                received += 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                control.cancel();
                break;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // Once the command deadline expires or process cleanup requests
    // cancellation, the readers poll that flag at a bounded interval. This
    // shutdown window owns their completion handshake; joins occur only after
    // the corresponding thread has sent or disconnected.
    if received < readers.len() {
        control.cancel();
        let shutdown_deadline = Instant::now() + READER_SHUTDOWN_BOUND;
        while received < readers.len() {
            let Some(remaining) = shutdown_deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            match receiver.recv_timeout(remaining) {
                Ok(message) => {
                    store_reader_message(message, &mut stdout, &mut stderr);
                    received += 1;
                }
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    shutdown_timed_out = received < readers.len();
                    break;
                }
            }
        }
    }

    // `read_stream` has no unbounded operation: after cancellation its only
    // possible blocking call is the platform poll capped at
    // READER_POLL_INTERVAL. These joins are therefore the terminal ownership
    // path even if the scheduling-oriented completion handshake misses its
    // generous shutdown window.
    for reader in readers {
        if reader.handle.join().is_err() {
            let outcome = match reader.stream {
                OutputStream::Stdout => &mut stdout,
                OutputStream::Stderr => &mut stderr,
            };
            *outcome = Some(ReaderOutcome::Panicked);
        }
    }

    if shutdown_timed_out {
        return Err(ProcessError::new(ProcessErrorKind::ReaderShutdownTimeout));
    }

    // Match the planning-base join order: stdout failures take precedence over
    // stderr failures regardless of which reader reported first.
    let stdout = resolve_reader_outcome(OutputStream::Stdout, stdout)?;
    let stderr = resolve_reader_outcome(OutputStream::Stderr, stderr)?;
    Ok(CapturedStreams { stdout, stderr })
}

fn store_reader_message(
    message: ReaderMessage,
    stdout: &mut Option<ReaderOutcome>,
    stderr: &mut Option<ReaderOutcome>,
) {
    match message.stream {
        OutputStream::Stdout => *stdout = Some(message.outcome),
        OutputStream::Stderr => *stderr = Some(message.outcome),
    }
}

fn resolve_reader_outcome(
    stream: OutputStream,
    outcome: Option<ReaderOutcome>,
) -> Result<Vec<u8>, ProcessError> {
    match outcome {
        Some(ReaderOutcome::Bytes(bytes)) => Ok(bytes),
        Some(ReaderOutcome::ReadError(source)) => {
            Err(ProcessError::new(ProcessErrorKind::ReaderRead {
                stream,
                source,
            }))
        }
        Some(ReaderOutcome::Panicked) | None => {
            Err(ProcessError::new(ProcessErrorKind::ReaderPanicked(stream)))
        }
        Some(ReaderOutcome::TimedOut) => Err(ProcessError::new(ProcessErrorKind::Timeout)),
    }
}

struct ReaderControl {
    deadline: Instant,
    cancelled: AtomicBool,
}

impl ReaderControl {
    fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            cancelled: AtomicBool::new(false),
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(unix)]
fn read_stream(reader: &mut dyn CapturedRead, control: &ReaderControl) -> ReaderOutcome {
    let deadline = control.deadline;
    let mut bytes = Vec::new();
    loop {
        if control.is_cancelled() {
            return ReaderOutcome::Bytes(bytes);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return ReaderOutcome::TimedOut;
        };
        let poll_for = remaining.min(READER_POLL_INTERVAL);
        match platform::output_ready(reader.raw_fd(), poll_for) {
            Ok(false) => continue,
            Ok(true) => {}
            Err(error) => {
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return ReaderOutcome::ReadError(error);
            }
        }
        let mut buffer = [0_u8; 8 * 1024];
        match reader.read(&mut buffer) {
            Ok(0) => return ReaderOutcome::Bytes(bytes),
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return ReaderOutcome::ReadError(error),
        }
    }
}

#[cfg(not(unix))]
fn read_stream(reader: &mut dyn CapturedRead, _control: &ReaderControl) -> ReaderOutcome {
    let mut bytes = Vec::new();
    match reader.read_to_end(&mut bytes) {
        Ok(_) => ReaderOutcome::Bytes(bytes),
        Err(error) => ReaderOutcome::ReadError(error),
    }
}

#[cfg(test)]
#[derive(Clone)]
struct TestHooks {
    fault: ReaderFault,
    active_readers: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    spawned_pids: std::sync::Arc<std::sync::Mutex<Vec<u32>>>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReaderFault {
    None,
    Missing(OutputStream),
    ReadError(OutputStream),
    ReadBoth,
    Panic(OutputStream),
}

#[cfg(test)]
impl SystemCommandRunner {
    fn with_hooks(hooks: TestHooks) -> Self {
        Self { hooks: Some(hooks) }
    }

    fn record_spawn(&self, pid: u32) {
        if let Some(hooks) = &self.hooks {
            hooks.spawned_pids.lock().unwrap().push(pid);
        }
    }

    fn take_capture(&self, child: &mut Child, stream: OutputStream) -> Option<CapturedOutput> {
        let capture: Option<CapturedOutput> = match stream {
            OutputStream::Stdout => child
                .stdout
                .take()
                .map(|capture| Box::new(capture) as CapturedOutput),
            OutputStream::Stderr => child
                .stderr
                .take()
                .map(|capture| Box::new(capture) as CapturedOutput),
        };
        if self
            .hooks
            .as_ref()
            .is_some_and(|hooks| hooks.fault == ReaderFault::Missing(stream))
        {
            None
        } else {
            capture
        }
    }

    fn spawn_reader(
        &self,
        stream: OutputStream,
        mut reader: CapturedOutput,
        sender: mpsc::Sender<ReaderMessage>,
        control: Arc<ReaderControl>,
    ) -> ReaderHandle {
        let hooks = self.hooks.clone();
        let handle = std::thread::spawn(move || {
            let _activity = ReaderActivity::new(
                hooks
                    .as_ref()
                    .map(|hooks| std::sync::Arc::clone(&hooks.active_readers)),
            );
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let fault = hooks
                    .as_ref()
                    .map_or(ReaderFault::None, |hooks| hooks.fault);
                if fault == ReaderFault::Panic(stream) {
                    panic!("scripted {stream:?} reader panic");
                }
                if fault == ReaderFault::ReadError(stream) || fault == ReaderFault::ReadBoth {
                    return ReaderOutcome::ReadError(std::io::Error::other(format!(
                        "scripted {} reader failure",
                        stream.name()
                    )));
                }
                read_stream(reader.as_mut(), &control)
            }))
            .unwrap_or(ReaderOutcome::Panicked);
            let _ = sender.send(ReaderMessage { stream, outcome });
        });
        ReaderHandle { stream, handle }
    }
}

#[cfg(not(test))]
impl SystemCommandRunner {
    fn record_spawn(&self, _pid: u32) {}

    fn take_capture(&self, child: &mut Child, stream: OutputStream) -> Option<CapturedOutput> {
        let capture: Option<CapturedOutput> = match stream {
            OutputStream::Stdout => child
                .stdout
                .take()
                .map(|capture| Box::new(capture) as CapturedOutput),
            OutputStream::Stderr => child
                .stderr
                .take()
                .map(|capture| Box::new(capture) as CapturedOutput),
        };
        capture
    }

    fn spawn_reader(
        &self,
        stream: OutputStream,
        mut reader: CapturedOutput,
        sender: mpsc::Sender<ReaderMessage>,
        control: Arc<ReaderControl>,
    ) -> ReaderHandle {
        let handle = std::thread::spawn(move || {
            let outcome = read_stream(reader.as_mut(), &control);
            let _ = sender.send(ReaderMessage { stream, outcome });
        });
        ReaderHandle { stream, handle }
    }
}

#[cfg(test)]
struct ReaderActivity {
    active: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
}

#[cfg(test)]
impl ReaderActivity {
    fn new(active: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>) -> Self {
        if let Some(active) = &active {
            active.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Self { active }
    }
}

#[cfg(test)]
impl Drop for ReaderActivity {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
#[path = "tests/process.rs"]
mod tests;
