use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncRead, AsyncReadExt};

use super::error::{BackupError, BackupErrorKind};

const FAILURE_OUTPUT_LIMIT: usize = 8 * 1024;
type ReaderJoinHandle = tokio::task::JoinHandle<std::io::Result<BoundedOutput>>;

pub(super) struct BoundedOutput {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

impl BoundedOutput {
    pub(super) fn render(self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).trim().to_string();
        if self.truncated {
            text.push_str(" …[truncated]");
        }
        text
    }
}

pub(super) struct OutputReaderTask {
    label: &'static str,
    handle: Option<ReaderJoinHandle>,
}

impl OutputReaderTask {
    pub(super) fn spawn(
        label: &'static str,
        reader: impl AsyncRead + Unpin + Send + 'static,
        activity: Arc<AtomicUsize>,
    ) -> Self {
        let handle = tokio::spawn(async move {
            let _activity = OutputReaderActivityGuard::new(activity);
            read_bounded_output(reader).await
        });
        Self {
            label,
            handle: Some(handle),
        }
    }

    pub(super) async fn finish(&mut self) -> Result<BoundedOutput, BackupError> {
        let result = self
            .handle
            .as_mut()
            .expect("output reader should be awaited once")
            .await;
        self.handle.take();
        map_output_reader_result(self.label, result)
    }

    pub(super) fn abort_if_running(&self) {
        if let Some(handle) = self.handle.as_ref()
            && !handle.is_finished()
        {
            handle.abort();
        }
    }

    pub(super) async fn finish_after_abort(
        &mut self,
    ) -> Option<Result<BoundedOutput, BackupError>> {
        let result = self.handle.as_mut()?.await;
        self.handle.take();
        match result {
            Err(error) if error.is_cancelled() => None,
            result => Some(map_output_reader_result(self.label, result)),
        }
    }

    fn take_handle(&mut self) -> Option<ReaderJoinHandle> {
        self.handle.take()
    }

    #[cfg(test)]
    pub(super) fn owns_handle(&self) -> bool {
        self.handle.is_some()
    }
}

impl Drop for OutputReaderTask {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.as_ref() {
            handle.abort();
        }
    }
}

/// Explicitly owns reader handles that exceeded the bounded cleanup wait.
///
/// Spawning performs the ownership handoff. `detach` only drops the outer task
/// handle; the spawned task continues until both aborted readers are joined.
pub(super) struct DetachedOutputReaderJoinTask {
    handle: tokio::task::JoinHandle<()>,
}

impl DetachedOutputReaderJoinTask {
    pub(super) fn spawn(
        stdout: &mut OutputReaderTask,
        stderr: &mut OutputReaderTask,
    ) -> Option<Self> {
        let stdout = stdout.take_handle();
        let stderr = stderr.take_handle();
        if stdout.is_none() && stderr.is_none() {
            return None;
        }
        let handle = tokio::spawn(async move {
            tokio::join!(join_reader(stdout), join_reader(stderr));
        });
        Some(Self { handle })
    }

    pub(super) fn detach(self) {
        drop(self.handle);
    }

    #[cfg(test)]
    pub(super) async fn join(self) {
        self.handle
            .await
            .expect("detached output-reader join task should complete");
    }
}

async fn join_reader(handle: Option<ReaderJoinHandle>) {
    if let Some(handle) = handle {
        let _ = handle.await;
    }
}

struct OutputReaderActivityGuard {
    activity: Arc<AtomicUsize>,
}

impl OutputReaderActivityGuard {
    fn new(activity: Arc<AtomicUsize>) -> Self {
        activity.fetch_add(1, Ordering::AcqRel);
        Self { activity }
    }
}

impl Drop for OutputReaderActivityGuard {
    fn drop(&mut self) {
        self.activity.fetch_sub(1, Ordering::AcqRel);
    }
}

fn map_output_reader_result(
    label: &'static str,
    result: Result<std::io::Result<BoundedOutput>, tokio::task::JoinError>,
) -> Result<BoundedOutput, BackupError> {
    result
        .map_err(|error| {
            BackupError::new(BackupErrorKind::OutputCaptureJoin {
                label,
                source: error.to_string(),
            })
        })?
        .map_err(|error| {
            BackupError::new(BackupErrorKind::OutputCaptureRead {
                label,
                source: error.to_string(),
            })
        })
}

pub(super) async fn read_bounded_output(
    mut reader: impl AsyncRead + Unpin,
) -> std::io::Result<BoundedOutput> {
    let mut bytes = Vec::with_capacity(FAILURE_OUTPUT_LIMIT);
    let mut truncated = false;
    let mut chunk = [0_u8; 4 * 1024];
    loop {
        let count = reader.read(&mut chunk).await?;
        if count == 0 {
            break;
        }
        let remaining = FAILURE_OUTPUT_LIMIT.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..count.min(remaining)]);
        truncated |= count > remaining;
        // Keep capture cancellation responsive even when a noisy descendant
        // keeps the pipe continuously readable.
        tokio::task::yield_now().await;
    }
    Ok(BoundedOutput { bytes, truncated })
}
