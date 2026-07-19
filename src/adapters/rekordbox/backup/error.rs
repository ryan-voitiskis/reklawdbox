use std::fmt;
use std::time::Duration;

#[derive(Debug)]
pub(super) struct BackupError {
    pub(super) kind: BackupErrorKind,
    cleanup: CleanupReport,
}

impl BackupError {
    pub(super) fn new(kind: BackupErrorKind) -> Self {
        Self {
            kind,
            cleanup: CleanupReport::default(),
        }
    }

    pub(super) fn with_cleanup(mut self, cleanup: CleanupReport) -> Self {
        self.cleanup.extend(cleanup);
        self
    }
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.kind)?;
        if !self.cleanup.is_empty() {
            write!(formatter, "; cleanup: {}", self.cleanup)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) enum BackupErrorKind {
    ScriptPreparation(String),
    Launch(String),
    OutputCaptureSetup(&'static str),
    OutputCaptureRead { label: &'static str, source: String },
    OutputCaptureJoin { label: &'static str, source: String },
    DeadlineExceeded(Duration),
    ProcessGroup(String),
    DescendantProcessDetected,
    Wait(String),
    NonZeroExit { status: String, details: String },
    SupervisorTask(String),
}

impl fmt::Display for BackupErrorKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScriptPreparation(message) | Self::ProcessGroup(message) => {
                formatter.write_str(message)
            }
            Self::Launch(source) => write!(formatter, "backup launch failed: {source}"),
            Self::OutputCaptureSetup(label) => {
                write!(formatter, "backup {label} capture was unavailable")
            }
            Self::OutputCaptureRead { label, source } => {
                write!(formatter, "backup {label} capture failed: {source}")
            }
            Self::OutputCaptureJoin { label, source } => {
                write!(formatter, "backup {label} capture task failed: {source}")
            }
            Self::DeadlineExceeded(timeout) => write!(
                formatter,
                "pre-operation backup timed out after {}",
                duration_label(*timeout)
            ),
            Self::DescendantProcessDetected => formatter
                .write_str("backup script exited while descendant processes were still running"),
            Self::Wait(source) => write!(formatter, "backup wait failed: {source}"),
            Self::NonZeroExit { status, details } => {
                write!(
                    formatter,
                    "backup failed with exit status {status}: {details}"
                )
            }
            Self::SupervisorTask(source) => {
                write!(formatter, "backup supervisor task failed: {source}")
            }
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct CleanupReport {
    entries: Vec<CleanupEntry>,
}

impl CleanupReport {
    pub(super) fn push(&mut self, entry: CleanupEntry) {
        self.entries.push(entry);
    }

    pub(super) fn extend(&mut self, other: Self) {
        self.entries.extend(other.entries);
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &CleanupEntry> {
        self.entries.iter()
    }
}

impl fmt::Display for CleanupReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, entry) in self.entries.iter().enumerate() {
            if index > 0 {
                formatter.write_str("; ")?;
            }
            write!(formatter, "{entry}")?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(super) enum CleanupEntry {
    ProcessGroup(String),
    DirectChildTermination(String),
    DirectChildReap(String),
    DirectChildTimeout(Duration),
    CapturedOutput { label: &'static str, output: String },
    OutputError(String),
    OutputTaskTimeout(Duration),
    OutputTasksStillActive,
}

impl fmt::Display for CleanupEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProcessGroup(message) | Self::OutputError(message) => {
                formatter.write_str(message)
            }
            Self::DirectChildTermination(source) => {
                write!(formatter, "direct-child termination failed: {source}")
            }
            Self::DirectChildReap(source) => {
                write!(formatter, "direct-child reap failed: {source}")
            }
            Self::DirectChildTimeout(timeout) => write!(
                formatter,
                "direct child did not exit within {} after termination",
                duration_label(*timeout)
            ),
            Self::CapturedOutput { label, output } => {
                write!(formatter, "backup {label}: {output}")
            }
            Self::OutputTaskTimeout(timeout) => write!(
                formatter,
                "backup output capture tasks did not stop within {} after cancellation",
                duration_label(*timeout)
            ),
            Self::OutputTasksStillActive => {
                formatter.write_str("backup output capture tasks remained active after cleanup")
            }
        }
    }
}

pub(super) fn duration_label(duration: Duration) -> String {
    if duration.subsec_nanos() == 0 {
        format!("{}s", duration.as_secs())
    } else {
        format!("{}ms", duration.as_millis())
    }
}
