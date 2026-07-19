//! Test-only ownership for opt-in private Rekordbox and audio fixtures.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
    archive_identity: u64,
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
        extractor(archive, root.path())?;

        let database_path = validate_extracted_database(root.path())?;

        let mut hasher = DefaultHasher::new();
        archive.hash(&mut hasher);
        Ok(Self {
            root,
            database_path,
            archive_identity: hasher.finish(),
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

    pub(crate) fn archive_identity(&self) -> u64 {
        self.archive_identity
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

        let original_hash = hash_file(&original_path)?;
        std::fs::copy(&original_path, &copied_path).map_err(PrivateFixtureError::AudioIo)?;
        let copied_hash = hash_file(&copied_path)?;
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
        Ok(hash_file(&self.original_path)? == self.original_hash)
    }
}

fn hash_file(path: &Path) -> Result<[u8; 32], PrivateFixtureError> {
    let mut file = File::open(path).map_err(PrivateFixtureError::AudioIo)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(PrivateFixtureError::AudioIo)?;
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
    // File-backed capture cannot deadlock on a full pipe and has no reader
    // thread whose lifetime can be extended by an inherited descendant FD.
    let mut stdout_capture = tempfile::tempfile().map_err(TarMemberError::Capture)?;
    let mut stderr_capture = tempfile::tempfile().map_err(TarMemberError::Capture)?;
    let stdout_sink = stdout_capture
        .try_clone()
        .map_err(TarMemberError::Capture)?;
    let stderr_sink = stderr_capture
        .try_clone()
        .map_err(TarMemberError::Capture)?;

    let mut command = Command::new(program);
    command
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .arg(member)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_sink))
        .stderr(Stdio::from(stderr_sink));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    let child = command.spawn().map_err(TarMemberError::Spawn)?;
    let mut child = OwnedTarChild::new(child)?;
    let status = child.wait_until(deadline)?;
    let stdout = read_capture(&mut stdout_capture).map_err(TarMemberError::Capture)?;
    let stderr = read_capture(&mut stderr_capture).map_err(TarMemberError::Capture)?;
    if Instant::now() >= deadline {
        return Err(TarMemberError::TimedOut);
    }
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

struct OwnedTarChild {
    child: Option<Child>,
    #[cfg(unix)]
    process_group: Option<i32>,
}

impl OwnedTarChild {
    fn new(mut child: Child) -> Result<Self, TarMemberError> {
        #[cfg(unix)]
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
            #[cfg(unix)]
            process_group,
        })
    }

    #[cfg(unix)]
    fn wait_until(&mut self, deadline: Instant) -> Result<ExitStatus, TarMemberError> {
        loop {
            match self.leader_exit_observed() {
                Ok(true) => return self.finish_observed(),
                Ok(false) => {}
                Err(error) => {
                    let _ = self.terminate_and_reap();
                    return Err(TarMemberError::Wait(error));
                }
            }
            if Instant::now() >= deadline {
                return match self.terminate_and_reap() {
                    Ok(_) => Err(TarMemberError::TimedOut),
                    Err(error) => Err(TarMemberError::Cleanup(error)),
                };
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(not(unix))]
    fn wait_until(&mut self, deadline: Instant) -> Result<ExitStatus, TarMemberError> {
        loop {
            let child = self.child.as_mut().expect("owned tar child should exist");
            if let Some(status) = child.try_wait().map_err(TarMemberError::Wait)? {
                self.child = None;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return match self.terminate_and_reap() {
                    Ok(_) => Err(TarMemberError::TimedOut),
                    Err(error) => Err(TarMemberError::Cleanup(error)),
                };
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(unix)]
    fn leader_exit_observed(&self) -> io::Result<bool> {
        let child = self.child.as_ref().expect("owned tar child should exist");
        leader_exit_observed_without_reaping(
            i32::try_from(child.id())
                .map_err(|error| io::Error::other(format!("tar PID conversion failed: {error}")))?,
        )
    }

    #[cfg(unix)]
    fn finish_observed(&mut self) -> Result<ExitStatus, TarMemberError> {
        let group_result = self.terminate_group(true);
        let status = self.reap().map_err(TarMemberError::Wait)?;
        group_result.map_err(TarMemberError::Cleanup)?;
        Ok(status)
    }

    fn terminate_and_reap(&mut self) -> io::Result<ExitStatus> {
        #[cfg(unix)]
        let terminate_result = self.terminate_group(false);
        #[cfg(not(unix))]
        let terminate_result = self
            .child
            .as_mut()
            .expect("owned tar child should exist")
            .kill();
        if terminate_result.is_err()
            && let Some(child) = self.child.as_mut()
        {
            let _ = child.kill();
        }
        let status = self.reap()?;
        terminate_result.map(|()| status)
    }

    #[cfg(unix)]
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

impl Drop for OwnedTarChild {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.terminate_and_reap();
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

fn reports_missing_member(stdout: &[u8], stderr: &[u8]) -> bool {
    let stdout = String::from_utf8_lossy(stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    stdout.contains("not found in archive")
        || stderr.contains("not found in archive")
        || stdout.contains("not found")
        || stderr.contains("not found")
}

fn read_capture(capture: &mut File) -> io::Result<Vec<u8>> {
    capture.seek(SeekFrom::Start(0))?;
    let mut captured = Vec::new();
    capture
        .take(DIAGNOSTIC_LIMIT as u64)
        .read_to_end(&mut captured)?;
    Ok(captured)
}

enum TarMemberError {
    Spawn(io::Error),
    Capture(io::Error),
    Wait(io::Error),
    Cleanup(io::Error),
    TimedOut,
    Absent(String),
    Exit {
        status: ExitStatus,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    },
}

impl TarMemberError {
    fn into_fixture_error(self, archive: &Path) -> PrivateFixtureError {
        let diagnostic = match self {
            Self::Spawn(error) => format!("could not start tar: {error}"),
            Self::Capture(error) => format!("could not capture tar output: {error}"),
            Self::Wait(error) => format!("could not observe tar: {error}"),
            Self::Cleanup(error) => format!("could not clean up tar: {error}"),
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

    #[test]
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
while [ "$i" -lt 6000 ]; do
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
