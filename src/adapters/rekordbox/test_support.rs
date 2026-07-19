//! Test-only ownership for opt-in private Rekordbox and audio fixtures.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::thread;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tempfile::{Builder, TempDir};

const EXTRACTION_TIMEOUT: Duration = Duration::from_secs(10);
const DIAGNOSTIC_LIMIT: usize = 4 * 1024;
const ARCHIVE_MEMBERS: [&str; 3] = ["master.db", "master.db-wal", "master.db-shm"];

#[derive(Debug, thiserror::Error)]
pub(crate) enum PrivateFixtureError {
    #[error("REKORDBOX_TEST_BACKUP is not configured")]
    Unconfigured,
    #[error("could not create a private Rekordbox fixture root: {0}")]
    TempRoot(#[source] io::Error),
    #[error("private Rekordbox fixture extraction failed: {diagnostic}")]
    Extraction { diagnostic: String },
    #[error("private Rekordbox fixture archive could not be read: {0}")]
    ArchiveIo(#[source] io::Error),
    #[error("private Rekordbox fixture archive changed during extraction")]
    ArchiveChanged,
    #[error("private Rekordbox fixture archive did not contain master.db")]
    MissingDatabase,
    #[error("private Rekordbox fixture database files must be owned regular files")]
    UnsafeDatabaseFile,
    #[error("private Rekordbox fixture path is not valid UTF-8")]
    InvalidDatabasePath,
    #[error(
        "private Rekordbox fixture could not open through the read-only SQLCipher adapter: {0}"
    )]
    Open(#[source] rusqlite::Error),
    #[error("private Rekordbox fixture has no accessible audio track")]
    NoAccessibleAudio,
    #[error("private audio fixture I/O failed: {0}")]
    AudioIo(#[source] io::Error),
}

/// Owns one unique extraction root for an explicitly configured private backup.
pub(crate) struct PrivateRekordboxFixture {
    root: TempDir,
    database_path: PathBuf,
    archive_path: PathBuf,
    archive_content_identity: [u8; 32],
    drop_log: Option<Arc<Mutex<Vec<&'static str>>>>,
}

impl PrivateRekordboxFixture {
    pub(crate) fn from_env() -> Result<Self, PrivateFixtureError> {
        let archive = std::env::var_os("REKORDBOX_TEST_BACKUP")
            .map(PathBuf::from)
            .ok_or(PrivateFixtureError::Unconfigured)?;
        Self::from_archive_with(&archive, extract_archive)
    }

    pub(crate) fn from_archive_with(
        archive: &Path,
        extractor: impl FnOnce(&Path, &Path) -> Result<(), PrivateFixtureError>,
    ) -> Result<Self, PrivateFixtureError> {
        let root = Builder::new()
            .prefix("reklawdbox-private-fixture-")
            .tempdir()
            .map_err(PrivateFixtureError::TempRoot)?;
        let archive_content_identity =
            hash_file(archive).map_err(PrivateFixtureError::ArchiveIo)?;
        let extraction = extractor(archive, root.path());
        let content_after = hash_file(archive).map_err(PrivateFixtureError::ArchiveIo)?;
        if content_after != archive_content_identity {
            return Err(PrivateFixtureError::ArchiveChanged);
        }
        extraction?;

        let database_path = validate_extracted_database(root.path())?;

        Ok(Self {
            root,
            database_path,
            archive_path: archive.to_path_buf(),
            archive_content_identity,
            drop_log: None,
        })
    }

    pub(crate) fn open(&self) -> Result<Connection, PrivateFixtureError> {
        let path = self
            .database_path
            .to_str()
            .ok_or(PrivateFixtureError::InvalidDatabasePath)?;
        super::open(path).map_err(PrivateFixtureError::Open)
    }

    pub(crate) fn root(&self) -> &Path {
        self.root.path()
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub(crate) fn source_archive_is_unchanged(&self) -> Result<bool, PrivateFixtureError> {
        Ok(
            hash_file(&self.archive_path).map_err(PrivateFixtureError::ArchiveIo)?
                == self.archive_content_identity,
        )
    }

    pub(crate) fn with_drop_log(mut self, drop_log: Arc<Mutex<Vec<&'static str>>>) -> Self {
        self.drop_log = Some(drop_log);
        self
    }

    pub(crate) fn copy_accessible_audio(
        &self,
        destination_root: &Path,
    ) -> Result<PrivateAudioCopy, PrivateFixtureError> {
        let conn = self.open()?;
        let entries = super::all_track_paths(&conn, None).map_err(PrivateFixtureError::Open)?;
        drop(conn);

        let original_path = entries
            .into_iter()
            .find_map(|entry| resolve_accessible_path(&entry.path))
            .ok_or(PrivateFixtureError::NoAccessibleAudio)?;
        let extension = original_path.extension().and_then(|value| value.to_str());
        let copied_path = match extension {
            Some(extension) if !extension.is_empty() => {
                destination_root.join(format!("private-audio-copy.{extension}"))
            }
            _ => destination_root.join("private-audio-copy"),
        };

        let original_hash = hash_file(&original_path).map_err(PrivateFixtureError::AudioIo)?;
        std::fs::copy(&original_path, &copied_path).map_err(PrivateFixtureError::AudioIo)?;
        let copied_hash = hash_file(&copied_path).map_err(PrivateFixtureError::AudioIo)?;
        Ok(PrivateAudioCopy {
            original_path,
            copied_path,
            original_hash,
            copied_hash,
        })
    }
}

impl Drop for PrivateRekordboxFixture {
    fn drop(&mut self) {
        if let Some(drop_log) = &self.drop_log
            && let Ok(mut drop_log) = drop_log.lock()
        {
            drop_log.push("fixture");
        }
    }
}

fn validate_extracted_database(root: &Path) -> Result<PathBuf, PrivateFixtureError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    let database_path = root.join(ARCHIVE_MEMBERS[0]);
    validate_extracted_file(&canonical_root, &database_path, true)?;
    for member in &ARCHIVE_MEMBERS[1..] {
        validate_extracted_file(&canonical_root, &root.join(member), false)?;
    }
    Ok(database_path)
}

fn validate_extracted_file(
    canonical_root: &Path,
    path: &Path,
    required: bool,
) -> Result<(), PrivateFixtureError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !required => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(PrivateFixtureError::MissingDatabase);
        }
        Err(_) => return Err(PrivateFixtureError::UnsafeDatabaseFile),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(PrivateFixtureError::UnsafeDatabaseFile);
    }
    let canonical_path = path
        .canonicalize()
        .map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    if !canonical_path.starts_with(canonical_root) {
        return Err(PrivateFixtureError::UnsafeDatabaseFile);
    }
    validate_exclusive_file_ownership(canonical_root, path, &metadata)?;
    Ok(())
}

#[cfg(unix)]
fn validate_exclusive_file_ownership(
    _canonical_root: &Path,
    _path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<(), PrivateFixtureError> {
    use std::os::unix::fs::MetadataExt as _;

    if metadata.nlink() != 1 {
        return Err(PrivateFixtureError::UnsafeDatabaseFile);
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_exclusive_file_ownership(
    canonical_root: &Path,
    path: &Path,
    _metadata: &std::fs::Metadata,
) -> Result<(), PrivateFixtureError> {
    // Link-count inspection is not portable. Replacing the extracted path
    // with a private-root copy gives the fixture an exclusively owned file
    // without changing an externally linked source inode.
    let owned = Builder::new()
        .prefix("reklawdbox-owned-database-")
        .tempfile_in(canonical_root)
        .map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    std::fs::copy(path, owned.path()).map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    owned
        .persist(path)
        .map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    let metadata =
        std::fs::symlink_metadata(path).map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    let canonical_path = path
        .canonicalize()
        .map_err(|_| PrivateFixtureError::UnsafeDatabaseFile)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || !canonical_path.starts_with(canonical_root)
    {
        return Err(PrivateFixtureError::UnsafeDatabaseFile);
    }
    Ok(())
}

pub(crate) struct PrivateAudioCopy {
    pub(crate) original_path: PathBuf,
    pub(crate) copied_path: PathBuf,
    pub(crate) original_hash: [u8; 32],
    pub(crate) copied_hash: [u8; 32],
}

impl PrivateAudioCopy {
    pub(crate) fn original_is_unchanged(&self) -> Result<bool, PrivateFixtureError> {
        Ok(
            hash_file(&self.original_path).map_err(PrivateFixtureError::AudioIo)?
                == self.original_hash,
        )
    }
}

fn hash_file(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

fn resolve_accessible_path(raw_path: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(raw_path);
    if direct.is_file() {
        return Some(direct);
    }
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .ok()?;
    let decoded = PathBuf::from(decoded.as_ref());
    decoded.is_file().then_some(decoded)
}

fn extract_archive(archive: &Path, destination: &Path) -> Result<(), PrivateFixtureError> {
    extract_archive_with_program(archive, destination, Path::new("tar"), EXTRACTION_TIMEOUT)
}

#[cfg(unix)]
fn extract_archive_with_program(
    archive: &Path,
    destination: &Path,
    program: &Path,
    timeout: Duration,
) -> Result<(), PrivateFixtureError> {
    let deadline = Instant::now() + timeout;
    if let Err(error) = run_tar_member(program, archive, destination, ARCHIVE_MEMBERS[0], deadline)
    {
        return Err(error.into_fixture_error(archive));
    }
    for member in &ARCHIVE_MEMBERS[1..] {
        match run_tar_member(program, archive, destination, member, deadline) {
            Ok(()) | Err(TarMemberError::Absent(_)) => {}
            Err(error) => return Err(error.into_fixture_error(archive)),
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn extract_archive_with_program(
    archive: &Path,
    _destination: &Path,
    _program: &Path,
    _timeout: Duration,
) -> Result<(), PrivateFixtureError> {
    Err(PrivateFixtureError::Extraction {
        diagnostic: sanitize_diagnostic(
            "safe bounded tar extraction is unsupported on this platform",
            archive,
        ),
    })
}

#[cfg(unix)]
fn run_tar_member(
    program: &Path,
    archive: &Path,
    destination: &Path,
    member: &str,
    deadline: Instant,
) -> Result<(), TarMemberError> {
    if Instant::now() >= deadline {
        return Err(TarMemberError::TimedOut);
    }

    let mut command = Command::new(program);
    command
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .arg(member)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
    let child = command.spawn().map_err(TarMemberError::Spawn)?;
    let mut child = OwnedTarChild::new(child)?;
    let mut capture = TarOutputCapture::take_from(&mut child)?;
    let status = child.wait_until(deadline, &mut capture)?;
    let (stdout, stderr) = capture.into_output();
    if status.success() {
        Ok(())
    } else if member != ARCHIVE_MEMBERS[0] && reports_missing_member(&stdout, &stderr) {
        Err(TarMemberError::Absent(member.to_string()))
    } else {
        Err(TarMemberError::Exit {
            status,
            stdout,
            stderr,
        })
    }
}

#[cfg(unix)]
struct OwnedTarChild {
    child: Option<Child>,
    process_group: Option<i32>,
}

#[cfg(unix)]
impl OwnedTarChild {
    fn new(mut child: Child) -> Result<Self, TarMemberError> {
        let process_group = match i32::try_from(child.id()) {
            Ok(process_group) => Some(process_group),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TarMemberError::Wait(io::Error::other(format!(
                    "tar PID conversion failed: {error}"
                ))));
            }
        };
        Ok(Self {
            child: Some(child),
            process_group,
        })
    }

    fn wait_until(
        &mut self,
        deadline: Instant,
        capture: &mut TarOutputCapture,
    ) -> Result<ExitStatus, TarMemberError> {
        loop {
            if let Err(error) = capture.drain_available() {
                return Err(self.cleanup_for(error));
            }
            match self.leader_exit_observed() {
                Ok(true) => {
                    let status = self.finish_observed()?;
                    capture.drain_until_closed(deadline)?;
                    return Ok(status);
                }
                Ok(false) => {}
                Err(error) => {
                    return Err(self.cleanup_for(TarMemberError::Wait(error)));
                }
            }
            if Instant::now() >= deadline {
                return match self.terminate_and_reap() {
                    Ok(_) => Err(TarMemberError::TimedOut),
                    Err(error) => Err(TarMemberError::Cleanup(error)),
                };
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn cleanup_for(&mut self, error: TarMemberError) -> TarMemberError {
        match self.terminate_and_reap() {
            Ok(_) => error,
            Err(cleanup) => TarMemberError::Cleanup(cleanup),
        }
    }

    fn leader_exit_observed(&self) -> io::Result<bool> {
        let child = self.child.as_ref().expect("owned tar child should exist");
        leader_exit_observed_without_reaping(
            i32::try_from(child.id())
                .map_err(|error| io::Error::other(format!("tar PID conversion failed: {error}")))?,
        )
    }

    fn finish_observed(&mut self) -> Result<ExitStatus, TarMemberError> {
        let group_result = self.terminate_group(true);
        let status = self.reap().map_err(TarMemberError::Wait)?;
        group_result.map_err(TarMemberError::Cleanup)?;
        Ok(status)
    }

    fn terminate_and_reap(&mut self) -> io::Result<ExitStatus> {
        let terminate_result = self.terminate_group(false);
        if terminate_result.is_err()
            && let Some(child) = self.child.as_mut()
        {
            let _ = child.kill();
        }
        let status = self.reap()?;
        terminate_result.map(|()| status)
    }

    fn terminate_group(&mut self, leader_exit_observed: bool) -> io::Result<()> {
        let Some(process_group) = self.process_group.take() else {
            return Ok(());
        };
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result == 0 {
            Ok(())
        } else {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH)
                || (leader_exit_observed && error.raw_os_error() == Some(libc::EPERM))
            {
                // Darwin can report EPERM for a group containing only the
                // unreaped zombie leader. A live same-UID descendant makes
                // the group signal succeed; the synthetic descendant test
                // exercises that distinction before the leader is reaped.
                Ok(())
            } else {
                Err(error)
            }
        }
    }

    fn reap(&mut self) -> io::Result<ExitStatus> {
        let status = self
            .child
            .as_mut()
            .expect("owned tar child should exist")
            .wait()?;
        self.child = None;
        Ok(status)
    }
}

#[cfg(unix)]
impl Drop for OwnedTarChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.terminate_and_reap();
        }
    }
}

#[cfg(unix)]
struct TarOutputCapture {
    stdout: Option<std::process::ChildStdout>,
    stderr: Option<std::process::ChildStderr>,
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
}

#[cfg(unix)]
impl TarOutputCapture {
    fn take_from(child: &mut OwnedTarChild) -> Result<Self, TarMemberError> {
        let child = child.child.as_mut().expect("owned tar child should exist");
        let stdout = child.stdout.take().ok_or_else(|| {
            TarMemberError::Capture(io::Error::other("tar stdout capture was unavailable"))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            TarMemberError::Capture(io::Error::other("tar stderr capture was unavailable"))
        })?;
        set_nonblocking(&stdout).map_err(TarMemberError::Capture)?;
        set_nonblocking(&stderr).map_err(TarMemberError::Capture)?;
        Ok(Self {
            stdout: Some(stdout),
            stderr: Some(stderr),
            stdout_bytes: Vec::new(),
            stderr_bytes: Vec::new(),
        })
    }

    fn drain_available(&mut self) -> Result<(), TarMemberError> {
        drain_stream(&mut self.stdout, &mut self.stdout_bytes)?;
        drain_stream(&mut self.stderr, &mut self.stderr_bytes)?;
        Ok(())
    }

    fn drain_until_closed(&mut self, deadline: Instant) -> Result<(), TarMemberError> {
        while self.stdout.is_some() || self.stderr.is_some() {
            self.drain_available()?;
            if self.stdout.is_none() && self.stderr.is_none() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(TarMemberError::TimedOut);
            }
            thread::sleep(Duration::from_millis(2));
        }
        Ok(())
    }

    fn into_output(self) -> (Vec<u8>, Vec<u8>) {
        (self.stdout_bytes, self.stderr_bytes)
    }
}

#[cfg(unix)]
fn set_nonblocking(stream: &impl std::os::fd::AsRawFd) -> io::Result<()> {
    let descriptor = stream.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn drain_stream<R: Read>(
    stream: &mut Option<R>,
    captured: &mut Vec<u8>,
) -> Result<(), TarMemberError> {
    let Some(reader) = stream.as_mut() else {
        return Ok(());
    };
    let mut buffer = [0_u8; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                *stream = None;
                return Ok(());
            }
            Ok(read) => {
                let remaining = DIAGNOSTIC_LIMIT.saturating_sub(captured.len());
                captured.extend_from_slice(&buffer[..read.min(remaining)]);
                if read > remaining {
                    return Err(TarMemberError::OutputLimit);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(TarMemberError::Capture(error)),
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "android"))]
fn leader_exit_observed_without_reaping(leader_pid: i32) -> io::Result<bool> {
    let leader_id = libc::id_t::try_from(leader_pid)
        .map_err(|error| io::Error::other(format!("tar PID conversion failed: {error}")))?;
    loop {
        let mut info = std::mem::MaybeUninit::<libc::siginfo_t>::zeroed();
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                leader_id,
                info.as_mut_ptr(),
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            let info = unsafe { info.assume_init() };
            let observed_pid = unsafe { info.si_pid() };
            if observed_pid == 0 {
                return Ok(false);
            }
            if observed_pid != leader_pid {
                return Err(io::Error::other(
                    "tar exit observation returned another PID",
                ));
            }
            return Ok(true);
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "linux", target_os = "android"))
))]
fn leader_exit_observed_without_reaping(_leader_pid: i32) -> io::Result<bool> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!(
            "safe pre-reap tar observation is unsupported on {}",
            std::env::consts::OS
        ),
    ))
}

#[cfg(unix)]
fn reports_missing_member(stdout: &[u8], stderr: &[u8]) -> bool {
    let stdout = String::from_utf8_lossy(stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    stdout.contains("not found in archive")
        || stderr.contains("not found in archive")
        || stdout.contains("not found")
        || stderr.contains("not found")
}

#[cfg(unix)]
enum TarMemberError {
    Spawn(io::Error),
    Capture(io::Error),
    Wait(io::Error),
    Cleanup(io::Error),
    OutputLimit,
    TimedOut,
    Absent(String),
    Exit {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

#[cfg(unix)]
impl TarMemberError {
    fn into_fixture_error(self, archive: &Path) -> PrivateFixtureError {
        let diagnostic = match self {
            Self::Spawn(error) => format!("could not start tar: {error}"),
            Self::Capture(error) => format!("could not capture tar output: {error}"),
            Self::Wait(error) => format!("could not observe tar: {error}"),
            Self::Cleanup(error) => format!("could not clean up tar: {error}"),
            Self::OutputLimit => {
                format!("tar output exceeded the {DIAGNOSTIC_LIMIT}-byte per-stream capture limit")
            }
            Self::TimedOut => "tar timed out".to_string(),
            Self::Absent(member) => format!("archive member {member} was unavailable"),
            Self::Exit {
                status,
                stdout,
                stderr,
            } => {
                let output = if stderr.is_empty() { stdout } else { stderr };
                let output = String::from_utf8_lossy(&output);
                format!("tar exited with {status}: {output}")
            }
        };
        PrivateFixtureError::Extraction {
            diagnostic: sanitize_diagnostic(&diagnostic, archive),
        }
    }
}

fn sanitize_diagnostic(diagnostic: &str, archive: &Path) -> String {
    let mut sanitized = diagnostic.to_string();
    if let Some(archive) = archive.to_str().filter(|value| !value.is_empty()) {
        sanitized = sanitized.replace(archive, "<configured archive>");
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        sanitized = sanitized.replace(&home, "<home>");
    }
    sanitized.chars().take(DIAGNOSTIC_LIMIT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn create_archive(source: &Path, archive: &Path, members: &[&str]) {
        let status = Command::new("tar")
            .arg("-czf")
            .arg(archive)
            .arg("-C")
            .arg(source)
            .args(members)
            .status()
            .expect("synthetic tar creation should start");
        assert!(status.success(), "synthetic tar creation should succeed");
    }

    #[cfg(unix)]
    fn write_executable_script(root: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;

        let script = root.join(name);
        std::fs::write(&script, format!("#!/bin/sh\n{body}\n"))
            .expect("synthetic tar script should write");
        let mut permissions = std::fs::metadata(&script)
            .expect("synthetic tar script metadata should exist")
            .permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions)
            .expect("synthetic tar script should become executable");
        script
    }

    #[cfg(unix)]
    fn wait_for_condition(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[cfg(unix)]
    fn process_or_group_is_absent(id: i32) -> bool {
        let result = unsafe { libc::kill(id, 0) };
        result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_extracts_exact_database_and_optional_sidecars() {
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("master.db"), b"database").unwrap();
        std::fs::write(source.path().join("master.db-wal"), b"wal").unwrap();
        std::fs::write(source.path().join("master.db-shm"), b"shm").unwrap();
        let archive = source.path().join("all-members.tar.gz");
        create_archive(source.path(), &archive, &ARCHIVE_MEMBERS);

        let destination = tempfile::tempdir().unwrap();
        extract_archive(&archive, destination.path()).unwrap();
        assert_eq!(
            std::fs::read(destination.path().join("master.db")).unwrap(),
            b"database"
        );
        assert_eq!(
            std::fs::read(destination.path().join("master.db-wal")).unwrap(),
            b"wal"
        );
        assert_eq!(
            std::fs::read(destination.path().join("master.db-shm")).unwrap(),
            b"shm"
        );

        let required_only_archive = source.path().join("required-only.tar.gz");
        create_archive(source.path(), &required_only_archive, &[ARCHIVE_MEMBERS[0]]);
        let required_only_destination = tempfile::tempdir().unwrap();
        extract_archive(&required_only_archive, required_only_destination.path()).unwrap();
        assert!(required_only_destination.path().join("master.db").is_file());
        assert!(
            !required_only_destination
                .path()
                .join("master.db-wal")
                .exists()
        );
        assert!(
            !required_only_destination
                .path()
                .join("master.db-shm")
                .exists()
        );
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_requires_the_exact_master_database_member() {
        let source = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join("nested")).unwrap();
        std::fs::write(source.path().join("nested/master.db"), b"not exact").unwrap();
        let archive = source.path().join("nested-only.tar.gz");
        create_archive(source.path(), &archive, &["nested/master.db"]);
        let destination = tempfile::tempdir().unwrap();

        let error = extract_archive(&archive, destination.path())
            .expect_err("a nested database member must not satisfy exact extraction");
        assert!(matches!(error, PrivateFixtureError::Extraction { .. }));
        assert!(!destination.path().join("master.db").exists());
        assert!(!destination.path().join("nested").exists());
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_bounds_and_sanitizes_failure_diagnostics() {
        let root = tempfile::tempdir().unwrap();
        let script = write_executable_script(
            root.path(),
            "diagnostic-tar",
            r#"
printf '%s\n%s\n' "$2" "$HOME" >&2
i=0
while [ "$i" -lt 1000 ]; do
  printf 'x' >&2
  i=$((i + 1))
done
exit 7
"#,
        );
        let archive = root.path().join("configured-private-name.tar.gz");
        std::fs::write(&archive, b"synthetic").unwrap();
        let destination = tempfile::tempdir().unwrap();

        let error = extract_archive_with_program(
            &archive,
            destination.path(),
            &script,
            Duration::from_secs(2),
        )
        .expect_err("synthetic diagnostic tar should fail");
        let PrivateFixtureError::Extraction { diagnostic } = error else {
            panic!("diagnostic failure should retain the extraction category");
        };
        assert!(diagnostic.chars().count() <= DIAGNOSTIC_LIMIT);
        assert!(diagnostic.contains("<configured archive>"));
        assert!(!diagnostic.contains(archive.to_string_lossy().as_ref()));
        if let Ok(home) = std::env::var("HOME")
            && !home.is_empty()
        {
            assert!(diagnostic.contains("<home>"));
            assert!(!diagnostic.contains(&home));
        }
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_owned_tar_child_drop_terminates_live_group_and_reaps_leader() {
        let root = tempfile::tempdir().unwrap();
        let script = write_executable_script(
            root.path(),
            "drop-tar",
            r#"
(sleep 0.25; : > "$1") &
printf '%s' "$!" > "$2"
: > "$3"
sleep 60
"#,
        );
        let leaked_marker = root.path().join("drop-descendant-leak");
        let descendant_pid_file = root.path().join("drop-descendant-pid");
        let ready = root.path().join("drop-ready");
        let mut command = Command::new(&script);
        command
            .arg(&leaked_marker)
            .arg(&descendant_pid_file)
            .arg(&ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
        let child = command.spawn().unwrap();
        let leader_pid = i32::try_from(child.id()).unwrap();
        let owner = OwnedTarChild::new(child)
            .unwrap_or_else(|_| panic!("synthetic tar child ownership should initialize"));

        assert!(wait_for_condition(Duration::from_secs(1), || {
            ready.is_file() && descendant_pid_file.is_file()
        }));
        let descendant_pid: i32 = std::fs::read_to_string(&descendant_pid_file)
            .unwrap()
            .parse()
            .unwrap();
        assert!(!process_or_group_is_absent(leader_pid));
        assert!(!process_or_group_is_absent(descendant_pid));

        let started = Instant::now();
        drop(owner);
        assert!(started.elapsed() < Duration::from_secs(1));

        let mut wait_status = 0;
        assert_eq!(
            unsafe { libc::waitpid(leader_pid, &mut wait_status, libc::WNOHANG) },
            -1,
            "OwnedTarChild::drop must reap its leader"
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        assert!(wait_for_condition(Duration::from_secs(1), || {
            process_or_group_is_absent(-leader_pid) && process_or_group_is_absent(descendant_pid)
        }));
        thread::sleep(Duration::from_millis(300));
        assert!(!leaked_marker.exists());
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_output_cap_cleans_descendants_under_shared_deadline() {
        let root = tempfile::tempdir().unwrap();
        let script = write_executable_script(
            root.path(),
            "chatty-tar",
            r#"
(sleep 0.25; : > "$2") &
printf '%s' "$!" > "$3"
: > "$4"
if [ "$1" = stdout ]; then
  dd if=/dev/zero bs=8192 count=1 2>/dev/null
else
  dd if=/dev/zero bs=8192 count=1 1>&2 2>/dev/null
fi
sleep 60
"#,
        );

        for stream in ["stdout", "stderr"] {
            let leaked_marker = root.path().join(format!("{stream}-descendant-leak"));
            let descendant_pid_file = root.path().join(format!("{stream}-descendant-pid"));
            let ready = root.path().join(format!("{stream}-ready"));
            let mut command = Command::new(&script);
            command
                .arg(stream)
                .arg(&leaked_marker)
                .arg(&descendant_pid_file)
                .arg(&ready)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            use std::os::unix::process::CommandExt as _;
            command.process_group(0);
            let child = command.spawn().unwrap();
            let leader_pid = i32::try_from(child.id()).unwrap();
            let mut owner = OwnedTarChild::new(child)
                .unwrap_or_else(|_| panic!("synthetic tar child ownership should initialize"));
            let mut capture = TarOutputCapture::take_from(&mut owner)
                .unwrap_or_else(|_| panic!("synthetic tar capture should initialize"));
            let deadline = Instant::now() + Duration::from_secs(1);
            let error = owner
                .wait_until(deadline, &mut capture)
                .expect_err("over-cap tar output must fail closed");

            assert!(matches!(error, TarMemberError::OutputLimit));
            assert!(capture.stdout_bytes.len() <= DIAGNOSTIC_LIMIT);
            assert!(capture.stderr_bytes.len() <= DIAGNOSTIC_LIMIT);
            assert!(Instant::now() < deadline);
            assert!(owner.child.is_none(), "over-cap leader must be reaped");
            let descendant_pid: i32 = std::fs::read_to_string(&descendant_pid_file)
                .unwrap()
                .parse()
                .unwrap();
            assert!(wait_for_condition(Duration::from_secs(1), || {
                process_or_group_is_absent(-leader_pid)
                    && process_or_group_is_absent(descendant_pid)
            }));
            thread::sleep(Duration::from_millis(300));
            assert!(!leaked_marker.exists());
        }
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_timeout_terminates_leader_and_descendants() {
        let root = tempfile::tempdir().unwrap();
        let script = write_executable_script(
            root.path(),
            "timeout-tar",
            r#"
(sleep 0.25; : > "$2-descendant-leak") &
sleep 60
"#,
        );
        let archive = root.path().join("timeout-archive");
        std::fs::write(&archive, b"synthetic").unwrap();
        let leaked_marker = PathBuf::from(format!("{}-descendant-leak", archive.display()));
        let destination = tempfile::tempdir().unwrap();
        let started = Instant::now();

        let error = extract_archive_with_program(
            &archive,
            destination.path(),
            &script,
            Duration::from_millis(60),
        )
        .expect_err("synthetic tar should time out");
        assert!(matches!(error, PrivateFixtureError::Extraction { .. }));
        assert!(started.elapsed() < Duration::from_secs(1));
        thread::sleep(Duration::from_millis(350));
        assert!(
            !leaked_marker.exists(),
            "timed-out tar descendants must not survive cleanup"
        );
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_tar_normal_exit_cleans_descendant_output_holders() {
        let root = tempfile::tempdir().unwrap();
        let script = write_executable_script(
            root.path(),
            "descendant-tar",
            r#"
printf 'fixture' > "$4/$5"
(sleep 0.25; : > "$2-descendant-leak") &
exit 0
"#,
        );
        let archive = root.path().join("normal-archive");
        std::fs::write(&archive, b"synthetic").unwrap();
        let leaked_marker = PathBuf::from(format!("{}-descendant-leak", archive.display()));
        let destination = tempfile::tempdir().unwrap();
        let started = Instant::now();

        extract_archive_with_program(
            &archive,
            destination.path(),
            &script,
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(350));
        assert!(
            !leaked_marker.exists(),
            "normally exited tar descendants must not retain output resources"
        );
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_fixture_rejects_out_of_root_database_symlink() {
        use std::os::unix::fs::symlink;

        let external = tempfile::NamedTempFile::new().unwrap();
        let archive = tempfile::NamedTempFile::new().unwrap();
        let result =
            PrivateRekordboxFixture::from_archive_with(archive.path(), |_archive, destination| {
                symlink(external.path(), destination.join("master.db")).unwrap();
                Ok(())
            });

        assert!(matches!(
            result,
            Err(PrivateFixtureError::UnsafeDatabaseFile)
        ));
        assert!(external.path().is_file());
    }

    #[test]
    #[cfg(unix)]
    fn rekordbox_connection_fixture_rejects_external_hard_linked_database_files() {
        for linked_member in ARCHIVE_MEMBERS {
            let archive = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(archive.path(), b"synthetic archive").unwrap();
            let external = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(external.path(), b"external database content").unwrap();
            let result = PrivateRekordboxFixture::from_archive_with(
                archive.path(),
                |_archive, destination| {
                    if linked_member != ARCHIVE_MEMBERS[0] {
                        std::fs::write(destination.join(ARCHIVE_MEMBERS[0]), b"database").unwrap();
                    }
                    std::fs::hard_link(external.path(), destination.join(linked_member)).unwrap();
                    Ok(())
                },
            );

            assert!(matches!(
                result,
                Err(PrivateFixtureError::UnsafeDatabaseFile)
            ));
            assert!(external.path().is_file());
        }
    }

    #[test]
    fn rekordbox_connection_fixture_verifies_source_archive_content_identity() {
        let archive = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(archive.path(), b"stable synthetic archive").unwrap();
        let fixture =
            PrivateRekordboxFixture::from_archive_with(archive.path(), |archive, destination| {
                std::fs::copy(archive, destination.join(ARCHIVE_MEMBERS[0]))
                    .map(|_| ())
                    .map_err(PrivateFixtureError::ArchiveIo)
            })
            .unwrap();
        assert!(fixture.source_archive_is_unchanged().unwrap());

        std::fs::write(archive.path(), b"changed after fixture creation").unwrap();
        assert!(!fixture.source_archive_is_unchanged().unwrap());

        let changing_archive = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(changing_archive.path(), b"before extraction").unwrap();
        let result = PrivateRekordboxFixture::from_archive_with(
            changing_archive.path(),
            |archive, destination| {
                std::fs::copy(archive, destination.join(ARCHIVE_MEMBERS[0])).unwrap();
                std::fs::write(archive, b"changed during extraction").unwrap();
                Ok(())
            },
        );
        assert!(matches!(result, Err(PrivateFixtureError::ArchiveChanged)));
    }

    #[test]
    fn rekordbox_connection_fixture_rejects_non_regular_database() {
        let archive = tempfile::NamedTempFile::new().unwrap();
        let result =
            PrivateRekordboxFixture::from_archive_with(archive.path(), |_archive, destination| {
                std::fs::create_dir(destination.join("master.db")).unwrap();
                Ok(())
            });

        assert!(matches!(
            result,
            Err(PrivateFixtureError::UnsafeDatabaseFile)
        ));
    }
}
