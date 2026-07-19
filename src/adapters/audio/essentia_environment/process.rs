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

#[cfg(unix)]
pub(super) fn output_ready(
    descriptor: std::os::fd::RawFd,
    timeout: Duration,
) -> std::io::Result<bool> {
    let timeout_ms = i32::try_from(timeout.as_millis().max(1)).unwrap_or(i32::MAX);
    let mut descriptor = libc::pollfd {
        fd: descriptor,
        events: libc::POLLIN | libc::POLLHUP,
        revents: 0,
    };
    // SAFETY: `descriptor` references one valid pollfd for the duration of the
    // call. Its owning reader retains the underlying descriptor until return.
    let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if ready >= 0 {
        Ok(ready > 0)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
pub(super) fn output_ready(_descriptor: (), _timeout: Duration) -> std::io::Result<bool> {
    Ok(true)
}

type CapturedOutput = Box<dyn CapturedRead>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CommandResult {
    pub(super) success: bool,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
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
    pub(super) fn new(kind: ProcessErrorKind) -> Self {
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
            ProcessErrorKind::ReaderStart { stream, source } => {
                write!(
                    formatter,
                    "{} reader failed to start: {source}",
                    stream.name()
                )
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
    ReaderStart {
        stream: OutputStream,
        source: std::io::Error,
    },
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
        let (completion_tx, completion_rx) = mpsc::channel();
        let reader_control = Arc::new(ReaderControl::new(deadline.end));
        let mut reader_owner =
            ReaderOwner::new(reader_rx, completion_rx, Arc::clone(&reader_control));
        for (stream, capture, sender, completion_sender) in [
            (
                OutputStream::Stdout,
                stdout,
                reader_tx.clone(),
                completion_tx.clone(),
            ),
            (OutputStream::Stderr, stderr, reader_tx, completion_tx),
        ] {
            match self.spawn_reader(
                stream,
                capture,
                sender,
                completion_sender,
                Arc::clone(&reader_control),
            ) {
                Ok(reader) => reader_owner.push(reader),
                Err(source) => {
                    reader_control.cancel();
                    terminate_owned_child(&mut child, &mut ownership);
                    reader_owner.shutdown(&deadline);
                    return Err(ProcessError::new(ProcessErrorKind::ReaderStart {
                        stream,
                        source,
                    }));
                }
            }
        }
        let process_outcome = wait_for_process(
            &mut child,
            &mut ownership,
            &deadline,
            &mut reader_owner,
            self.inject_inspection_failure(),
        );
        if !matches!(process_outcome, ProcessWait::Exited(_)) {
            reader_control.cancel();
        }
        let reader_outcome = reader_owner.finish(&deadline);

        match process_outcome {
            ProcessWait::ReaderFailed => Err(reader_outcome
                .err()
                .unwrap_or_else(|| ProcessError::new(ProcessErrorKind::ReaderShutdownTimeout))),
            ProcessWait::Failed(error) => Err(error),
            ProcessWait::Exited(process_outcome) => {
                let streams = reader_outcome?;
                if process_outcome.had_surviving_descendants {
                    return Err(ProcessError::new(ProcessErrorKind::SurvivingDescendants));
                }
                if deadline.expired() {
                    return Err(ProcessError::new(ProcessErrorKind::Timeout));
                }
                Ok(CommandResult {
                    success: process_outcome.status.success(),
                    stdout: streams.stdout,
                    stderr: streams.stderr,
                })
            }
        }
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

enum ProcessWait {
    Exited(ProcessOutcome),
    Failed(ProcessError),
    ReaderFailed,
}

#[cfg(unix)]
fn wait_for_process(
    child: &mut Child,
    ownership: &mut ProcessGroupOwnership,
    deadline: &Deadline,
    readers: &mut ReaderOwner,
    inject_inspection_failure: bool,
) -> ProcessWait {
    loop {
        readers.drain_outcomes();
        if readers.has_failure() {
            terminate_owned_child(child, ownership);
            return ProcessWait::ReaderFailed;
        }
        match ownership.leader_exit_observed_without_reaping() {
            Ok(true) => break,
            Ok(false) if !deadline.expired() => deadline.sleep_poll(),
            Ok(false) => {
                terminate_owned_child(child, ownership);
                return ProcessWait::Failed(ProcessError::new(ProcessErrorKind::Timeout));
            }
            Err(error) => {
                terminate_owned_child(child, ownership);
                return ProcessWait::Failed(process_group_error(error));
            }
        }
    }

    readers.drain_outcomes();
    if readers.has_failure() {
        terminate_owned_child(child, ownership);
        return ProcessWait::ReaderFailed;
    }
    let inspection = inspect_and_release_before_reap(ownership, inject_inspection_failure);
    let had_surviving_descendants = match inspection {
        Ok(had_surviving_descendants) => had_surviving_descendants,
        Err(error) => {
            // Inspection errors still leave this function owning the child.
            // Release/terminate the group first, then reap the unreaped leader.
            terminate_owned_child(child, ownership);
            return ProcessWait::Failed(process_group_error(error));
        }
    };
    debug_assert!(ownership.is_released());
    match child.wait() {
        Ok(status) => ProcessWait::Exited(ProcessOutcome {
            status,
            had_surviving_descendants,
        }),
        Err(error) => ProcessWait::Failed(ProcessError::new(ProcessErrorKind::Wait(error))),
    }
}

#[cfg(unix)]
fn inspect_and_release_before_reap(
    ownership: &mut ProcessGroupOwnership,
    inject_failure: bool,
) -> Result<bool, ProcessGroupError> {
    #[cfg(test)]
    if inject_failure {
        return Err(ProcessGroupError::injected_inspection_failure());
    }
    let _ = inject_failure;
    ownership.inspect_and_release_before_reap()
}

#[cfg(not(unix))]
fn wait_for_process(
    child: &mut Child,
    ownership: &mut ProcessGroupOwnership,
    deadline: &Deadline,
    readers: &mut ReaderOwner,
    _inject_inspection_failure: bool,
) -> ProcessWait {
    loop {
        readers.drain_outcomes();
        if readers.has_failure() {
            terminate_owned_child(child, ownership);
            return ProcessWait::ReaderFailed;
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                debug_assert!(ownership.is_released());
                return ProcessWait::Exited(ProcessOutcome {
                    status,
                    had_surviving_descendants: false,
                });
            }
            Ok(None) if !deadline.expired() => deadline.sleep_poll(),
            Ok(None) => {
                terminate_owned_child(child, ownership);
                return ProcessWait::Failed(ProcessError::new(ProcessErrorKind::Timeout));
            }
            Err(error) => {
                terminate_owned_child(child, ownership);
                return ProcessWait::Failed(ProcessError::new(ProcessErrorKind::Wait(error)));
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

struct ReaderCompletion {
    stream: OutputStream,
}

struct ReaderCompletionGuard {
    stream: OutputStream,
    sender: Option<mpsc::Sender<ReaderCompletion>>,
}

impl Drop for ReaderCompletionGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let _ = sender.send(ReaderCompletion {
                stream: self.stream,
            });
        }
    }
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

struct ReaderOwner {
    readers: Vec<ReaderHandle>,
    receiver: Receiver<ReaderMessage>,
    completion_receiver: Receiver<ReaderCompletion>,
    control: Arc<ReaderControl>,
    stdout: Option<ReaderOutcome>,
    stderr: Option<ReaderOutcome>,
}

impl ReaderOwner {
    fn new(
        receiver: Receiver<ReaderMessage>,
        completion_receiver: Receiver<ReaderCompletion>,
        control: Arc<ReaderControl>,
    ) -> Self {
        Self {
            readers: Vec::new(),
            receiver,
            completion_receiver,
            control,
            stdout: None,
            stderr: None,
        }
    }

    fn push(&mut self, reader: ReaderHandle) {
        self.readers.push(reader);
    }

    fn drain_outcomes(&mut self) {
        while let Ok(message) = self.receiver.try_recv() {
            self.store(message);
        }
    }

    fn has_failure(&self) -> bool {
        [&self.stdout, &self.stderr].into_iter().any(|outcome| {
            matches!(
                outcome,
                Some(
                    ReaderOutcome::ReadError(_) | ReaderOutcome::Panicked | ReaderOutcome::TimedOut
                )
            )
        })
    }

    fn finish(mut self, deadline: &Deadline) -> Result<CapturedStreams, ProcessError> {
        let joined = self.join_readers(deadline);
        let stdout = resolve_reader_outcome(OutputStream::Stdout, self.stdout.take())?;
        let stderr = resolve_reader_outcome(OutputStream::Stderr, self.stderr.take())?;
        if joined.exceeded_total_deadline || joined.missing_completion_ack {
            return Err(ProcessError::new(ProcessErrorKind::ReaderShutdownTimeout));
        }
        Ok(CapturedStreams { stdout, stderr })
    }

    fn shutdown(mut self, deadline: &Deadline) {
        self.control.cancel();
        let _ = self.join_readers(deadline);
    }

    fn join_readers(&mut self, deadline: &Deadline) -> ReaderJoinState {
        self.drain_outcomes();
        let expects_stdout = self.has_reader(OutputStream::Stdout);
        let expects_stderr = self.has_reader(OutputStream::Stderr);
        let mut stdout_complete = false;
        let mut stderr_complete = false;
        let mut exceeded_total_deadline = false;
        let mut cleanup_deadline = self
            .control
            .is_cancelled()
            .then(|| cleanup_deadline_from(Instant::now()));
        loop {
            self.drain_outcomes();
            while let Ok(completion) = self.completion_receiver.try_recv() {
                match completion.stream {
                    OutputStream::Stdout => stdout_complete = true,
                    OutputStream::Stderr => stderr_complete = true,
                }
            }

            if self
                .readers
                .iter()
                .all(|reader| reader.handle.is_finished())
            {
                break;
            }

            let now = Instant::now();
            if cleanup_deadline.is_none() && now >= deadline.end {
                exceeded_total_deadline = true;
                self.control.cancel();
                cleanup_deadline = Some(cleanup_deadline_from(now));
            }
            if cleanup_deadline.is_some_and(|cleanup_deadline| now >= cleanup_deadline) {
                panic!("Essentia output reader violated its bounded cancellation invariant");
            }

            let next_deadline = cleanup_deadline.unwrap_or(deadline.end);
            let wait = next_deadline
                .checked_duration_since(now)
                .unwrap_or_default()
                .min(READER_POLL_INTERVAL);
            match self.completion_receiver.recv_timeout(wait) {
                Ok(completion) => match completion.stream {
                    OutputStream::Stdout => stdout_complete = true,
                    OutputStream::Stderr => stderr_complete = true,
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Sender disconnection proves both reader closures have
                    // returned; `is_finished` is still checked before join.
                }
            }
        }
        // Each handle exclusively owns one pipe. On Unix `read_stream` first
        // polls that sole descriptor for at most 10 ms, so its subsequent read
        // cannot lose readiness to a competing consumer. The panic guard still
        // sends an outcome, and cancellation closes every scripted stall. The
        // completion acknowledgement is only a wake-up: `is_finished` above is
        // the actual nonblocking-join precondition.
        for reader in std::mem::take(&mut self.readers) {
            assert!(reader.handle.is_finished());
            if reader.handle.join().is_err() {
                let outcome = match reader.stream {
                    OutputStream::Stdout => &mut self.stdout,
                    OutputStream::Stderr => &mut self.stderr,
                };
                *outcome = Some(ReaderOutcome::Panicked);
            }
        }

        // A guard acknowledgement may race just behind `is_finished`; joining
        // establishes completion, so judge the handshake only after this final
        // drain. Outcomes use the same ordering for deterministic diagnostics.
        self.drain_outcomes();
        while let Ok(completion) = self.completion_receiver.try_recv() {
            match completion.stream {
                OutputStream::Stdout => stdout_complete = true,
                OutputStream::Stderr => stderr_complete = true,
            }
        }
        ReaderJoinState {
            exceeded_total_deadline,
            missing_completion_ack: (expects_stdout && !stdout_complete)
                || (expects_stderr && !stderr_complete),
        }
    }

    fn store(&mut self, message: ReaderMessage) {
        match message.stream {
            OutputStream::Stdout => self.stdout = Some(message.outcome),
            OutputStream::Stderr => self.stderr = Some(message.outcome),
        }
    }

    fn has_reader(&self, stream: OutputStream) -> bool {
        self.readers.iter().any(|reader| reader.stream == stream)
    }
}

struct ReaderJoinState {
    exceeded_total_deadline: bool,
    missing_completion_ack: bool,
}

impl Drop for ReaderOwner {
    fn drop(&mut self) {
        self.control.cancel();
        // Normal paths take only already-finished handles. If the bounded
        // invariant panics, retain ownership and join during unwinding instead
        // of silently detaching an output reader.
        for reader in std::mem::take(&mut self.readers) {
            let _ = reader.handle.join();
        }
    }
}

fn cleanup_deadline_from(started: Instant) -> Instant {
    started
        .checked_add(READER_SHUTDOWN_BOUND)
        .unwrap_or(started)
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
        match output_ready(reader.raw_fd(), poll_for) {
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
    spawn_failure: Option<OutputStream>,
    inject_inspection_failure: bool,
    fault_release: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
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
    StallUntilCancelled(OutputStream),
    SuppressCompletion(OutputStream),
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

    fn inject_inspection_failure(&self) -> bool {
        self.hooks
            .as_ref()
            .is_some_and(|hooks| hooks.inject_inspection_failure)
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
}

#[cfg(not(test))]
impl SystemCommandRunner {
    fn record_spawn(&self, _pid: u32) {}

    fn inject_inspection_failure(&self) -> bool {
        false
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
        capture
    }
}

impl SystemCommandRunner {
    fn spawn_reader(
        &self,
        stream: OutputStream,
        mut reader: CapturedOutput,
        sender: mpsc::Sender<ReaderMessage>,
        completion_sender: mpsc::Sender<ReaderCompletion>,
        control: Arc<ReaderControl>,
    ) -> std::io::Result<ReaderHandle> {
        #[cfg(test)]
        if self
            .hooks
            .as_ref()
            .is_some_and(|hooks| hooks.spawn_failure == Some(stream))
        {
            return Err(std::io::Error::other(format!(
                "scripted {} reader spawn failure",
                stream.name()
            )));
        }

        #[cfg(test)]
        let hooks = self.hooks.clone();
        let handle = std::thread::Builder::new()
            .name(format!("essentia-{}-reader", stream.name()))
            .spawn(move || {
                #[cfg(test)]
                let suppress_completion = hooks
                    .as_ref()
                    .is_some_and(|hooks| hooks.fault == ReaderFault::SuppressCompletion(stream));
                #[cfg(not(test))]
                let suppress_completion = false;
                let _completion = ReaderCompletionGuard {
                    stream,
                    sender: (!suppress_completion).then_some(completion_sender),
                };
                #[cfg(test)]
                let _activity = ReaderActivity::new(
                    hooks
                        .as_ref()
                        .map(|hooks| std::sync::Arc::clone(&hooks.active_readers)),
                );
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    #[cfg(test)]
                    {
                        read_stream_with_test_hooks(
                            reader.as_mut(),
                            &control,
                            stream,
                            hooks.as_ref(),
                        )
                    }
                    #[cfg(not(test))]
                    {
                        read_stream(reader.as_mut(), &control)
                    }
                }))
                .unwrap_or(ReaderOutcome::Panicked);
                let _ = sender.send(ReaderMessage { stream, outcome });
            })?;
        Ok(ReaderHandle { stream, handle })
    }
}

#[cfg(test)]
fn read_stream_with_test_hooks(
    reader: &mut dyn CapturedRead,
    control: &ReaderControl,
    stream: OutputStream,
    hooks: Option<&TestHooks>,
) -> ReaderOutcome {
    let fault = hooks.map_or(ReaderFault::None, |hooks| hooks.fault);
    if let Some(release) = hooks.and_then(|hooks| hooks.fault_release.as_ref()) {
        while !release.load(std::sync::atomic::Ordering::SeqCst) {
            if control.is_cancelled() || Instant::now() >= control.deadline {
                return ReaderOutcome::TimedOut;
            }
            std::thread::yield_now();
        }
    }
    if fault == ReaderFault::Panic(stream) {
        panic!("scripted {stream:?} reader panic");
    }
    if fault == ReaderFault::ReadError(stream) || fault == ReaderFault::ReadBoth {
        return ReaderOutcome::ReadError(std::io::Error::other(format!(
            "scripted {} reader failure",
            stream.name()
        )));
    }
    if fault == ReaderFault::StallUntilCancelled(stream) {
        while !control.is_cancelled() {
            std::thread::sleep(READER_POLL_INTERVAL);
        }
        return ReaderOutcome::Bytes(Vec::new());
    }
    read_stream(reader, control)
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
