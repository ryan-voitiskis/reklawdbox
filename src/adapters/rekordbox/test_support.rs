//! Test-only ownership for opt-in private Rekordbox and audio fixtures.

use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
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

        let database_path = root.path().join("master.db");
        if !database_path.is_file() {
            return Err(PrivateFixtureError::MissingDatabase);
        }

        let mut hasher = DefaultHasher::new();
        archive.hash(&mut hasher);
        Ok(Self {
            root,
            database_path,
            archive_identity: hasher.finish(),
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
    let deadline = Instant::now() + EXTRACTION_TIMEOUT;
    if let Err(error) = run_tar_member(archive, destination, ARCHIVE_MEMBERS[0], deadline) {
        return Err(error.into_fixture_error(archive));
    }
    for member in &ARCHIVE_MEMBERS[1..] {
        match run_tar_member(archive, destination, member, deadline) {
            Ok(()) | Err(TarMemberError::Absent(_)) => {}
            Err(error) => return Err(error.into_fixture_error(archive)),
        }
    }
    Ok(())
}

fn run_tar_member(
    archive: &Path,
    destination: &Path,
    member: &str,
    deadline: Instant,
) -> Result<(), TarMemberError> {
    let mut child = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(destination)
        .arg(member)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(TarMemberError::Spawn)?;

    let stdout = child.stdout.take().expect("piped tar stdout should exist");
    let stderr = child.stderr.take().expect("piped tar stderr should exist");
    let stdout_reader = thread::spawn(move || read_bounded(stdout));
    let stderr_reader = thread::spawn(move || read_bounded(stderr));

    let status = loop {
        if let Some(status) = child.try_wait().map_err(TarMemberError::Wait)? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(TarMemberError::TimedOut);
        }
        thread::sleep(Duration::from_millis(10));
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
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

fn reports_missing_member(stdout: &[u8], stderr: &[u8]) -> bool {
    let stdout = String::from_utf8_lossy(stdout).to_ascii_lowercase();
    let stderr = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    stdout.contains("not found in archive")
        || stderr.contains("not found in archive")
        || stdout.contains("not found")
        || stderr.contains("not found")
}

fn read_bounded(mut reader: impl Read) -> Vec<u8> {
    let mut captured = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let Ok(read) = reader.read(&mut buffer) else {
            break;
        };
        if read == 0 {
            break;
        }
        let remaining = DIAGNOSTIC_LIMIT.saturating_sub(captured.len());
        captured.extend_from_slice(&buffer[..read.min(remaining)]);
    }
    captured
}

enum TarMemberError {
    Spawn(io::Error),
    Wait(io::Error),
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
            Self::Wait(error) => format!("could not observe tar: {error}"),
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
